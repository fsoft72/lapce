#!/usr/bin/env bash
# Build the debug binary and copy it to ./bin/goldend-ide-debug
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="$ROOT_DIR/bin"

cd "$ROOT_DIR"
cargo build --bin goldend-ide

mkdir -p "$BIN_DIR"
cp "$ROOT_DIR/target/debug/goldend-ide" "$BIN_DIR/goldend-ide-debug"
echo "Debug build ready: $BIN_DIR/goldend-ide-debug"
