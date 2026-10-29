#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
export CARGO_TARGET_DIR=target/hyprswell
export CARGO_HOME="$CARGO_TARGET_DIR/cargo"
export CARGO_NET_OFFLINE=true
export PYTHONPYCACHEPREFIX="$CARGO_TARGET_DIR/pycache"
trap 'echo "App grid FAIL" >&2' ERR
cargo test --manifest-path dock/Cargo.toml --locked -j 8 --test launcher
python3 settings/tests/gate_dock_launcher.py
cargo build --manifest-path dock/Cargo.toml --locked -j 8
python3 dock/gates/launcher_capture.py
echo 'App grid numeric/capture gate PASS; visual review pending'
