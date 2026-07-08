#!/usr/bin/env bash
# One-time container setup: everything scripts/claude-review.py and the
# repository skills (check, unsafe-verify) need to run.
#
# Only tools with no devcontainer feature are installed here. Checked
# (July 2026): no feature exists for kani, for a secondary nightly
# toolchain with miri, or for the Claude CLI without pulling in node —
# cargo-nextest and cargo-deny come from features in devcontainer.json.
set -euo pipefail

# Claude CLI (native build; the container image has no node/npm)
curl -fsSL https://claude.ai/install.sh | bash

# Nightly toolchain for the check and unsafe-verify skills:
#   miri      — undefined-behaviour checks on unsafe code
#   rust-src  — required by miri and the sanitizers
#   rustfmt   — rustfmt.toml uses nightly-only options
rustup toolchain install nightly --profile minimal -c miri -c rust-src -c rustfmt

# The rust feature shares CARGO_HOME via the rustlang group, but feature
# installs that run cargo as root (e.g. cargo-nextest) leave files without
# group write, which breaks later `cargo install` as the vscode user.
sudo chmod -R g+w /usr/local/cargo
sudo find /usr/local/cargo -type d -exec chmod g+s {} +

# Kani model checker (proof harnesses in crates/unsafety and crates/number)
cargo install --locked kani-verifier
cargo kani setup

# rust-analyzer MCP server for scripts/claude-review.py: gives review
# sessions rust-analyzer's semantic model (definitions, references, types,
# diagnostics). It shells out to the `rust-analyzer` binary the rust
# devcontainer feature already puts on PATH.
cargo install --locked rust-analyzer-mcp

# Native libraries for the GUI workspace (tools/gui, see the check skill),
# and python3-venv for the review script's virtualenv below
sudo apt-get update -qq
sudo apt-get install -y -qq libfontconfig1-dev libfreetype-dev python3-venv

# Python dependencies for scripts/claude-review.py (the code-review-graph
# MCP server; the base image ships python3 without pip)
python3 -m venv .venv
.venv/bin/pip install --quiet -r scripts/requirements.txt
