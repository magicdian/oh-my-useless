#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${1:-0.1.0}"
TARGET_DIR="$ROOT_DIR/target/release"
STAGE_DIR="$ROOT_DIR/dist/useless-$VERSION"
ARCHIVE_PATH="$ROOT_DIR/dist/useless-$VERSION.tar.gz"

cd "$ROOT_DIR"

cargo build --release

rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR/bin"

cp "$TARGET_DIR/useless" "$STAGE_DIR/bin/useless"
cp "$ROOT_DIR/README.md" "$STAGE_DIR/README.md"

tar -C "$ROOT_DIR/dist" -czf "$ARCHIVE_PATH" "useless-$VERSION"

printf 'created %s\n' "$ARCHIVE_PATH"
