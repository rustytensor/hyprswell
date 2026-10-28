#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
export CARGO_TARGET_DIR=target/hyprswell
export CARGO_HOME="$CARGO_TARGET_DIR/cargo"
export CARGO_NET_OFFLINE=true
export PYTHONPYCACHEPREFIX="$CARGO_TARGET_DIR/pycache"
trap 'echo "Reflections FAIL" >&2' ERR
cargo test --manifest-path dock/Cargo.toml --locked -j 8 --test reflections
python3 settings/tests/gate_dock_reflections.py
cargo build --manifest-path dock/Cargo.toml --locked -j 8
python3 dock/gates/reflections_capture.py
echo 'Reflections numeric/capture gate PASS; visual review pending'
