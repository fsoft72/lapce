#!/usr/bin/env bash
# Build the release binary and copy it to ./bin/lapcie
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="$ROOT_DIR/bin"

cd "$ROOT_DIR"
cargo build --release --bin lapce

mkdir -p "$BIN_DIR"
cp "$ROOT_DIR/target/release/lapce" "$BIN_DIR/lapcie"
echo "Release build ready: $BIN_DIR/lapcie"
