#!/usr/bin/env bash
# Build a release archive with the same layout and name as the GitHub release
# workflow: useless-<target>.tar.gz, with the binary, README.md, and LICENSE at
# the archive root.
#
# Usage: ./scripts/package.sh
#        TARGET=aarch64-unknown-linux-musl ./scripts/package.sh
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${TARGET:-$(rustc -vV | sed -n 's/^host: //p')}"
DIST_DIR="$ROOT_DIR/dist"
NAME="useless-$TARGET"
STAGE_DIR="$DIST_DIR/$NAME"
ARCHIVE_PATH="$DIST_DIR/$NAME.tar.gz"
SUM_PATH="$ARCHIVE_PATH.sha256"

if [ -z "$TARGET" ]; then
  echo "could not determine the target triple; set TARGET explicitly" >&2
  exit 1
fi

cd "$ROOT_DIR"

cargo build --release --locked --target "$TARGET"

rm -rf "$STAGE_DIR" "$ARCHIVE_PATH" "$SUM_PATH"
mkdir -p "$STAGE_DIR"

cp "$ROOT_DIR/target/$TARGET/release/useless" "$STAGE_DIR/useless"
cp "$ROOT_DIR/README.md" "$STAGE_DIR/README.md"
cp "$ROOT_DIR/LICENSE" "$STAGE_DIR/LICENSE"

tar -C "$STAGE_DIR" -czf "$ARCHIVE_PATH" useless README.md LICENSE
rm -rf "$STAGE_DIR"

if command -v sha256sum >/dev/null 2>&1; then
  (cd "$DIST_DIR" && sha256sum "$NAME.tar.gz" > "$NAME.tar.gz.sha256")
elif command -v shasum >/dev/null 2>&1; then
  (cd "$DIST_DIR" && shasum -a 256 "$NAME.tar.gz" > "$NAME.tar.gz.sha256")
else
  echo "no SHA-256 tool found (need sha256sum or shasum)" >&2
  exit 1
fi

printf 'created %s\n' "$ARCHIVE_PATH"
printf 'created %s\n' "$SUM_PATH"
