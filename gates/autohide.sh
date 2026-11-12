#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
export CARGO_TARGET_DIR=target/hyprswell
export CARGO_HOME="$CARGO_TARGET_DIR/cargo"
export CARGO_NET_OFFLINE=true
export CARGO_BUILD_JOBS=8
export PYTHONPYCACHEPREFIX="$CARGO_TARGET_DIR/pycache"
trap 'echo "Auto-hide FAIL" >&2' ERR
# Pure geometry, input, motion, timer, urgency-record and control-socket checks (hand-worked, 1e-6).
cargo test --manifest-path dock/Cargo.toml --locked -j 8 --test autohide -- --test-threads 8
# Temporary-config preservation and the no-restart Settings switch.
python3 settings/tests/gate_dock_autohide.py
cargo build --manifest-path dock/Cargo.toml --locked -j 8
# One owned DP-2 capture session: hidden.png, --show over a private socket, shown.png.
python3 dock/gates/autohide_capture.py
echo 'Auto-hide numeric/capture gate PASS; visual review pending'
