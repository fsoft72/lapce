#!/usr/bin/env bash
# Build the debug binary and copy it to ./bin/lapce-debug
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="$ROOT_DIR/bin"

cd "$ROOT_DIR"
cargo build --bin lapce

mkdir -p "$BIN_DIR"
cp "$ROOT_DIR/target/debug/lapce" "$BIN_DIR/lapce-debug"
echo "Debug build ready: $BIN_DIR/lapce-debug"
