#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR=target/hyprswell
export CARGO_HOME="$CARGO_TARGET_DIR/cargo"
trap 'echo "Gate A FAIL" >&2' ERR
cargo test -j 8
cargo build -j 8
python3 gates/surface.py
echo 'Gate A PASS'
