#!/usr/bin/env python3
"""Review files one at a time with the Claude CLI, using this repository's
review skill (.claude/skills/review/SKILL.md) and the code-review-graph
package for token-efficient context lookup.

Each file is reviewed in its own ``claude -p`` (print mode) invocation.
Print mode starts a brand-new session every time, so the context is
cleared between files automatically — each review only sees the one file
it was asked about, keeping the review local.

Every session gets a code-review-graph MCP server (tree-sitter knowledge
graph of the repository) so it can look up callers, callees, and tests
through graph queries instead of broad greps. The graph is built or
incrementally updated once at startup with ``code-review-graph build``.

Every session also gets a rust-analyzer MCP server
(github.com/zeenix/rust-analyzer-mcp) so it can resolve definitions,
references, inferred types, and compiler diagnostics from
rust-analyzer's semantic model instead of guessing from text. Both
servers are optional: disable them with --no-graph and
--no-rust-analyzer.

When the Claude CLI reports a usage/session limit, the run aborts with
exit code 3, the aborted file's partial output is removed, and findings
collected so far are still gathered. Rerun the same command once the
limit resets; with --all the run resumes, skipping finished reports.

Output:
    OUT_DIR/files/<path>.md      raw per-file reports
    OUT_DIR/files/<path>.jsonl   full session transcripts (--verbose only)
    OUT_DIR/mcp-config.json      MCP config handed to each session
    OUT_DIR/review.md            all findings gathered into one markdown

Every option can also be set through the environment variable named in
its help text; the command-line flag wins.
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

SKILL_FILE = ".claude/skills/review/SKILL.md"
UNSAFE_SKILL_FILE = ".claude/skills/unsafe-verify/SKILL.md"

# The message the CLI prints when a session/usage limit is exhausted,
# e.g. "You've hit your session limit". One aborted call is enough to
# know every following call would fail the same way.
LIMIT_REGEX = re.compile(
    r"session limit|usage limit|rate limit|limit reached"
    r"|limit will reset|out of extra usage",
    re.IGNORECASE,
)


def parse_args(argv):
    parser = argparse.ArgumentParser(
        prog="scripts/claude-review.py",
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "files",
        nargs="*",
        metavar="file",
        help="review exactly these files; default is the files changed "
        "on this branch relative to the base branch",
    )
    parser.add_argument(
        "--all",
        action="store_true",
        help="review every tracked source file matching --source-glob; "
        "files that already have a report are skipped, so an "
        "interrupted run resumes where it left off (implies --clean)",
    )
    parser.add_argument(
        "--clean",
        action="store_true",
        help="review each file's full current contents from scratch, "
        "instead of reviewing the branch diff",
    )
    parser.add_argument(
        "--read-only",
        action="store_true",
        help="inspection-only review. By default the review may write a "
        "minimal test in the affected crate and run it with cargo "
        "(including the miri/loom/sanitizer invocations from the "
        "check and unsafe-verify skills) to demonstrate a defect; a "
        "demonstrably failing test is left in the working tree as "
        "the regression test for the fix",
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="save each session's full activity as stream-JSON next to "
        "the report AND mirror it (messages, tool calls) to the "
        "terminal while the review runs",
    )
    parser.add_argument(
        "--base-branch",
        default=os.environ.get("BASE_BRANCH", "main"),
        metavar="BRANCH",
        help="diff base, falling back to origin/BRANCH "
        "(env: BASE_BRANCH, default: %(default)s)",
    )
    parser.add_argument(
        "--out-dir",
        default=os.environ.get("OUT_DIR", "review-output"),
        metavar="DIR",
        help="output directory (env: OUT_DIR, default: %(default)s)",
    )
    parser.add_argument(
        "--model",
        default=os.environ.get("MODEL", ""),
        help='model override, e.g. "fable" or "sonnet" (env: MODEL, '
        "default: the CLI's configured model)",
    )
    parser.add_argument(
        "--source-glob",
        default=os.environ.get("SOURCE_GLOB", "*.rs"),
        metavar="GLOB",
        help="pathspec for --all (env: SOURCE_GLOB, default: %(default)s)",
    )
    parser.add_argument(
        "--review-command",
        default=os.environ.get("REVIEW_COMMAND", ""),
        metavar="CMD",
        help="slash command to run instead of the built-in prompt, "
        'invoked per file as "CMD <file>" (env: REVIEW_COMMAND)',
    )
    parser.add_argument(
        "--no-graph",
        action="store_true",
        default=os.environ.get("NO_GRAPH", "") == "1",
        help="skip the code-review-graph build and MCP server "
        "(env: NO_GRAPH=1)",
    )
    parser.add_argument(
        "--no-rust-analyzer",
        action="store_true",
        default=os.environ.get("NO_RUST_ANALYZER", "") == "1",
        help="skip the rust-analyzer MCP server (env: NO_RUST_ANALYZER=1)",
    )
    args = parser.parse_args(argv)
    if args.all and args.files:
        parser.error("--all and explicit files are mutually exclusive")
    # A full-codebase sweep has no meaningful diff to anchor on.
    if args.all:
        args.clean = True
    return args


def run_git(*args):
    return subprocess.run(
        ["git", *args], capture_output=True, text=True, check=True
    ).stdout


def resolve_base(base_branch):
    """A plain branch name like "main" may only exist as a remote-tracking
    ref in a shallow or single-branch clone."""
    for ref in (base_branch, f"origin/{base_branch}"):
        proc = subprocess.run(
            ["git", "rev-parse", "--verify", "--quiet", f"{ref}^{{commit}}"],
            capture_output=True,
        )
        if proc.returncode == 0:
            return ref
    sys.exit(
        f"error: base branch '{base_branch}' not found (set --base-branch)"
    )


def collect_files(args, base):
    if args.all:
        out = run_git("ls-files", "--", args.source_glob)
    elif args.files:
        return list(args.files)
    else:
        out = run_git(
            "diff", "--name-only", "--diff-filter=ACMR",
            f"{base}...HEAD", "--", ".", ":(exclude)*.lock",
        )
    return [line for line in out.splitlines() if line]


def find_code_review_graph():
    """Prefer the venv the script itself runs from, then PATH, then the
    repository's .venv (post-create puts it there)."""
    candidates = [
        Path(sys.executable).parent / "code-review-graph",
        shutil.which("code-review-graph"),
        Path(".venv/bin/code-review-graph"),
    ]
    for cand in candidates:
        if cand and Path(cand).is_file() and os.access(cand, os.X_OK):
            return Path(cand).resolve()
    sys.exit(
        "error: code-review-graph not found\n"
        "install it with: python3 -m venv .venv && "
        ".venv/bin/pip install -r scripts/requirements.txt"
    )


def find_rust_analyzer_mcp():
    """Locate the rust-analyzer MCP server
    (github.com/zeenix/rust-analyzer-mcp, `cargo install rust-analyzer-mcp`),
    which shells out to the `rust-analyzer` binary on PATH. It is
    supplementary context, so a missing binary is a warning that skips the
    server rather than a fatal error."""
    binary = shutil.which("rust-analyzer-mcp")
    if binary is None:
        print(
            "    warning: rust-analyzer-mcp not on PATH; reviewing without "
            "semantic Rust navigation "
            "(install: cargo install rust-analyzer-mcp)"
        )
        return None
    if shutil.which("rust-analyzer") is None:
        print(
            "    warning: rust-analyzer-mcp found but `rust-analyzer` is not "
            "on PATH; skipping the rust-analyzer MCP server"
        )
        return None
    return Path(binary).resolve()


def build_graph_server(repo_root):
    """Build/update the knowledge graph and return its MCP server entry."""
    crg = find_code_review_graph()
    print("==> building code-review-graph knowledge graph")
    build = subprocess.run(
        [str(crg), "build"], capture_output=True, text=True
    )
    tail = (build.stdout + build.stderr).strip().splitlines()
    for line in tail[-3:]:
        print(f"    {line}")
    if build.returncode != 0:
        sys.exit("error: code-review-graph build failed")

    # The server is launched through its interpreter, matching the config
    # `code-review-graph install --platform claude-code` generates, but
    # written to OUT_DIR so the repository's own MCP/settings files are
    # left untouched.
    python = crg.parent / "python3"
    return {
        "command": str(python if python.is_file() else crg),
        "args": (
            ["-m", "code_review_graph", "serve"]
            if python.is_file() else ["serve"]
        ),
        "cwd": str(repo_root),
        "type": "stdio",
    }


def setup_mcp(args, out_dir, repo_root):
    """Assemble every enabled MCP server and write the config that each
    review session is started with, or return None when none are enabled."""
    servers = {}

    if not args.no_graph:
        servers["code-review-graph"] = build_graph_server(repo_root)

    if not args.no_rust_analyzer:
        ra = find_rust_analyzer_mcp()
        if ra is not None:
            # rust-analyzer indexes the workspace rooted at cwd; merc's
            # nested tools/* workspaces are reachable through the server's
            # rust_analyzer_set_workspace tool.
            servers["rust-analyzer"] = {
                "command": str(ra),
                "args": [],
                "cwd": str(repo_root),
                "type": "stdio",
            }

    if not servers:
        return None

    mcp_config = out_dir / "mcp-config.json"
    mcp_config.write_text(
        json.dumps({"mcpServers": servers}, indent=2)
    )
    return mcp_config


def review_prompt(args, file, base):
    """Both variants follow the repository's review skill; they differ
    only in whether the subject is the branch diff or the file's full
    current contents."""
    if args.clean:
        subject = (
            f"Do a clean, from-scratch review of the file '{file}' as it "
            "currently exists — this is not a diff review; judge the "
            "entire file."
        )
    else:
        subject = (
            f"Review the changes made to '{file}' on this branch relative "
            f"to '{base}' (see: git diff {base}...HEAD -- '{file}'). "
            "Read the rest of the file for context."
        )
    if args.no_graph:
        graph = ""
    else:
        graph = (
            "\nA code-review-graph MCP server exposes a knowledge graph "
            "of this repository. Use its tools for context lookup instead "
            "of broad greps: query_graph_tool (patterns callers_of, "
            "callees_of, tests_for, file_summary), get_review_context_tool "
            "and traverse_graph_tool — they return precise, small answers."
            "\n"
        )
    if args.no_rust_analyzer:
        rust_analyzer = ""
    else:
        rust_analyzer = (
            "\nA rust-analyzer MCP server exposes rust-analyzer's semantic "
            "model. Prefer its tools over textual search when resolving "
            "Rust symbols: rust_analyzer_definition and "
            "rust_analyzer_references follow symbols precisely, "
            "rust_analyzer_hover gives inferred types and signatures, and "
            "rust_analyzer_diagnostics reports compiler-level errors and "
            "warnings for a file. Its positions are zero-based line/column "
            "into the current file. Call rust_analyzer_set_workspace first "
            "if the file under review lives in a nested workspace under "
            "tools/.\n"
        )
    if args.read_only:
        verification = (
            "This is a read-only review: you cannot edit files or run "
            "tests or builds, so skip the skill's failing-test step and "
            'report any defect you cannot demonstrate as "plausible" '
            "rather than confirmed."
        )
    else:
        verification = (
            "To demonstrate a defect (step 4 of the skill), write a "
            "minimal test in the affected crate and run it with cargo "
            "(e.g. 'cargo nextest run -p <crate> <test-name>' or 'cargo "
            "test'). For unsafe or concurrent code, also use the "
            f"miri/loom/sanitizer invocations from {UNSAFE_SKILL_FILE} — "
            "those exact command lines are pre-approved; type them "
            "verbatim (same env-variable quoting and order), because "
            "modified variants are denied. Only add or edit tests — "
            "never change non-test code, even to fix a defect you found. "
            "Leave a demonstrably failing test in the working tree as "
            "the regression test for the eventual fix and state its "
            "exact location in the report; delete any candidate test "
            "that passes. Report a defect you could not demonstrate as "
            '"plausible" rather than confirmed.'
        )
    return f"""Follow the review instructions in {SKILL_FILE}.

{subject} Look up callers and definitions elsewhere in the repository as
needed for context, but confine your findings to this file.
{graph}{rust_analyzer}
{verification}

Write the report as GitHub-flavored markdown: a one-line verdict first,
then findings ranked by severity, each with file:line references and a
concrete failure scenario. If you find no defects, say so and list what
you checked. Output only the report itself.
"""


def allowed_tools(args):
    """Tools the review session may use without prompting; anything else
    is auto-denied in print mode, so a run never blocks on a permission
    prompt."""
    tools = [
        "Read", "Grep", "Glob",
        "Bash(git diff:*)", "Bash(git log:*)", "Bash(git show:*)",
    ]
    if not args.no_graph:
        # A bare server name allows every tool that server provides.
        tools.append("mcp__code-review-graph")
    if not args.no_rust_analyzer:
        tools.append("mcp__rust-analyzer")
    if not args.read_only:
        # Plain cargo plus the exact env-prefixed invocations the check
        # and unsafe-verify skills prescribe. The permission matcher does
        # NOT strip env assignments — 'Bash(cargo:*)' alone does not
        # match 'RUSTFLAGS=... cargo ...' — so each form is allowed
        # verbatim, and the prompt tells the session to use them exactly
        # as written.
        tools += [
            "Edit", "Write", "Bash(cd:*)", "Bash(cargo:*)",
            'Bash(MIRIFLAGS="-Zmiri-disable-isolation'
            ' --cfg chacha20_force_soft" cargo +nightly miri:*)',
            'Bash(RUSTFLAGS="--cfg loom" cargo:*)',
            'Bash(RUST_MIN_STACK=104857600 cargo:*)',
            'Bash(RUSTDOCFLAGS="-D warnings" cargo:*)',
            'Bash(RUST_BACKTRACE=full RUST_LOG=debug cargo:*)',
        ]
    return tools


def render_event(event):
    """One terse terminal line per interesting stream-JSON event."""
    if event.get("type") == "assistant":
        lines = []
        for block in event.get("message", {}).get("content", []):
            if block.get("type") == "text":
                text = " ".join(block.get("text", "").split())
                lines.append(f"    [claude] {text[:200]}")
            elif block.get("type") == "tool_use":
                arg = block.get("input", {})
                arg = (
                    arg.get("command")
                    or arg.get("file_path")
                    or arg.get("pattern")
                    or arg.get("target")
                    or ""
                )
                arg = " ".join(str(arg).split())
                lines.append(f"    [{block.get('name')}] {arg[:160]}")
        return lines
    if event.get("type") == "result":
        return [f"    [result] {event.get('subtype', '?')}"]
    return []


def claude_command(args, prompt, mcp_config):
    cmd = ["claude", "-p", prompt, "--allowedTools", *allowed_tools(args)]
    if args.model:
        cmd += ["--model", args.model]
    if mcp_config is not None:
        cmd += ["--mcp-config", str(mcp_config), "--strict-mcp-config"]
    if args.verbose:
        cmd += ["--verbose", "--output-format", "stream-json"]
    return cmd


def run_review(args, cmd, report, err_path, jsonl_path):
    """One fresh `claude -p`: no session is resumed, so nothing from the
    previous file's review leaks into this one. Returns True on success
    (a non-empty report was written)."""
    if not args.verbose:
        with open(report, "w") as out, open(err_path, "w") as err:
            proc = subprocess.run(cmd, stdout=out, stderr=err, text=True)
        return proc.returncode == 0 and report.stat().st_size > 0

    # stream-json records the whole session in the .jsonl while each
    # event is mirrored to the terminal; the final report is extracted
    # from the stream's result event afterwards. Malformed lines are
    # ignored instead of killing the run.
    result_text = None
    with open(jsonl_path, "w") as jsonl, open(err_path, "w") as err:
        proc = subprocess.Popen(
            cmd, stdout=subprocess.PIPE, stderr=err,
            text=True, errors="replace",
        )
        for line in proc.stdout:
            jsonl.write(line)
            jsonl.flush()
            try:
                event = json.loads(line)
            except ValueError:
                continue
            if not isinstance(event, dict):
                continue
            for rendered in render_event(event):
                print(rendered, flush=True)
            if event.get("type") == "result":
                result_text = event.get("result")
        proc.wait()
    if proc.returncode != 0 or not result_text:
        return False
    report.write_text(result_text)
    return True


def first_content_line(report):
    for line in report.read_text().splitlines():
        if line.strip() and line.strip() != "---":
            return line
    return "(empty report)"


def limit_message(*paths):
    for path in paths:
        try:
            text = path.read_text(errors="replace")
        except OSError:
            continue
        for line in text.splitlines():
            if LIMIT_REGEX.search(line):
                return line.strip()
    return None


def write_summary(args, base, out_dir, total, reviewed, failed, aborted):
    """Gather every per-file report into a single markdown document."""
    summary = out_dir / "review.md"
    with open(summary, "w") as out:
        out.write("# Claude review report\n\n")
        if args.clean:
            out.write(
                "- **Mode:** clean review of each file's current contents\n"
            )
        else:
            out.write(f"- **Mode:** review of changes relative to `{base}`\n")
        now = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
        out.write(f"- **Date:** {now}\n")
        out.write(f"- **Files reviewed:** {len(reviewed)} of {total}\n")
        if failed:
            out.write(f"- **Failed:** {len(failed)} (see below)\n")
        if aborted:
            out.write(
                f"- **Aborted:** usage/session limit hit at `{aborted}`; "
                "rerun the same command after the limit resets to resume\n"
            )
        for file in reviewed:
            out.write(f"\n---\n\n## `{file}`\n\n")
            out.write(report_path(out_dir, file).read_text())
        if failed:
            out.write("\n---\n\n## Failed reviews\n\n")
            for file in failed:
                out.write(
                    f"- `{file}` — see `{report_path(out_dir, file)}.err`\n"
                )
    return summary


def report_path(out_dir, file):
    return out_dir / "files" / (file.replace("/", "__") + ".md")


def main(argv=None):
    args = parse_args(argv)

    if shutil.which("claude") is None:
        sys.exit(
            "error: claude CLI not found on PATH\n"
            "install it with: curl -fsSL https://claude.ai/install.sh | bash"
        )

    # Run from the repository root so report paths and pathspecs are stable.
    os.chdir(run_git("rev-parse", "--show-toplevel").strip())

    if not args.review_command and not Path(SKILL_FILE).is_file():
        sys.exit(
            f"error: {SKILL_FILE} not found "
            "(set --review-command to use another command)"
        )

    # The diff base is needed to select files (default mode) and to
    # anchor the prompt (non-clean reviews).
    base = None
    if (not args.all and not args.files) or not args.clean:
        base = resolve_base(args.base_branch)
    files = collect_files(args, base)
    if not files:
        print(
            "nothing to review (no files given and no changes "
            f"vs {args.base_branch})"
        )
        return 0

    out_dir = Path(args.out_dir)
    (out_dir / "files").mkdir(parents=True, exist_ok=True)
    mcp_config = setup_mcp(args, out_dir, Path.cwd())

    reviewed, failed, aborted = [], [], None
    total = len(files)
    for n, file in enumerate(files, 1):
        if not Path(file).is_file():
            print(f"==> [{n}/{total}] skipping {file} (not a regular file)")
            continue

        report = report_path(out_dir, file)
        err_path = Path(f"{report}.err")
        jsonl_path = Path(f"{report}.jsonl")
        if args.all and report.is_file() and report.stat().st_size > 0:
            print(f"==> [{n}/{total}] skipping {file} (already reviewed)")
            reviewed.append(file)
            continue
        print(f"==> [{n}/{total}] reviewing {file} -> {report}", flush=True)

        if args.review_command:
            prompt = f"{args.review_command} {file}"
        else:
            prompt = review_prompt(args, file, base)

        cmd = claude_command(args, prompt, mcp_config)
        ok = run_review(args, cmd, report, err_path, jsonl_path)

        if ok:
            err_path.unlink(missing_ok=True)
            reviewed.append(file)
            print(f"    done: {first_content_line(report)}")
            continue

        # Failed: never keep a partial report — a later --all run must
        # retry this file instead of skipping it.
        report.unlink(missing_ok=True)
        limit = limit_message(err_path, jsonl_path)
        if limit:
            print(f"    ABORTED: {limit}")
            err_path.unlink(missing_ok=True)
            jsonl_path.unlink(missing_ok=True)
            aborted = file
            break
        extra = f" and {jsonl_path}" if args.verbose else ""
        print(f"    FAILED (see {err_path}{extra})")
        failed.append(file)

    summary = write_summary(
        args, base, out_dir, total, reviewed, failed, aborted
    )
    print(
        f"\nfindings gathered in {summary} (per-file reports in "
        f"{out_dir / 'files'}/)"
    )
    if aborted:
        print(f"aborted by usage/session limit at {aborted}", file=sys.stderr)
        print(
            "rerun the same command after the limit resets to resume",
            file=sys.stderr,
        )
        return 3
    if failed:
        for file in failed:
            print(f"failed: {file}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
