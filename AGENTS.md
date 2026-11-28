# Agent instructions

Instructions for coding agents working in this repository. The human-facing documentation is in
[README.md](README.md) and [CONTRIBUTING.md](CONTRIBUTING.md).

## Workspaces

The repository has three independent cargo workspaces. Run cargo in the workspace that contains
the crate or tool you work on; a `cargo run --bin merc-lps` from the root fails with
"no bin target named `merc-lps`".

| Workspace | Directory | Contents |
|---|---|---|
| root | repository root | all `crates/*`, and the `merc-lts`, `merc-rewrite`, `merc-sym`, `merc-vpg` tools |
| mCRL2 | `tools/mcrl2` | `merc-lps`, `merc-pbes` and their crates; links the mCRL2 C++ toolset, so it needs a C++ compiler. See [tools/mcrl2/AGENTS.md](tools/mcrl2/AGENTS.md) |
| GUI | `tools/gui` | `merc-ltsgraph` (needs `libfontconfig1-dev`, `libfreetype6-dev` on Linux). See [tools/gui/AGENTS.md](tools/gui/AGENTS.md) |

- `tools/mcrl2` and `tools/gui` set `target-dir = "../../target"`, so every binary ends up in the
  root `target/`, for example `target/release/merc-lps`. Never pass `--target-dir` or set
  `CARGO_TARGET_DIR`: that builds every dependency a second time.
- The crates under `crates/` are path dependencies of the other workspaces, so a change there can
  break `tools/mcrl2` or `tools/gui` without breaking the root workspace. Build the affected
  workspaces after such a change.
- Release builds carry line tables (`debug = "line-tables-only"`), so they can be profiled directly.

## Skills and agents

Task-specific instructions live in [`.agents/skills`](.agents/skills). Not every agent discovers
that directory by itself, so read the matching `SKILL.md` before you start such a task.

| Skill | Use for |
|---|---|
| [check](.agents/skills/check/SKILL.md) | build, tests, formatting (nightly rustfmt), clippy and cargo deny, as CI runs them |
| [run-tools](.agents/skills/run-tools/SKILL.md) | building and running the command-line tools on the bundled examples |
| [benchmark](.agents/skills/benchmark/SKILL.md) | the criterion benchmarks and before/after comparisons |
| [unsafe-verify](.agents/skills/unsafe-verify/SKILL.md) | miri, loom, kani and the sanitizers for unsafe or concurrent code |
| [review](.agents/skills/review/SKILL.md) | critical review of a diff, PR, commit or design |
| [architecture](.agents/skills/architecture/SKILL.md) | the aterm term library, visitors and traversal, crate layout and code conventions; read before writing code in `crates/` |
| [tests](.agents/skills/tests/SKILL.md) | writing, moving, reviewing and pruning tests; how to run a test pass |
| [audit-generated-code](.agents/skills/audit-generated-code/SKILL.md) | finding unreviewed or inconsistent code and planning a review pass |
| [documentation](.agents/skills/documentation/SKILL.md) | where documentation for a change belongs |

Subagent definitions are in [`.agents/agents`](.agents/agents):
[implementor](.agents/agents/implementor.md) and [review-adversary](.agents/agents/review-adversary.md).
If your harness cannot register them as subagents, give a general-purpose subagent the file as its
instructions.