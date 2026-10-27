#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
export CARGO_TARGET_DIR=target/hyprswell
export CARGO_HOME="$CARGO_TARGET_DIR/cargo"
export CARGO_NET_OFFLINE=true
export PYTHONPYCACHEPREFIX="$CARGO_TARGET_DIR/pycache"
trap 'echo "Dock screens FAIL" >&2' ERR
cargo test --manifest-path dock/Cargo.toml --locked -j 8 --test parity screens_
python3 settings/tests/gate_dock_screens.py
echo 'Dock screens PASS'
