#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR=target/hyprswell
export CARGO_HOME="$CARGO_TARGET_DIR/cargo"
trap 'echo "Gate B maths FAIL" >&2' ERR
cargo test -j 8 --test motion
echo 'Gate B maths PASS; visual acceptance is separate'
