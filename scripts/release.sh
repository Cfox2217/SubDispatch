#!/usr/bin/env sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
TARGET_DIR="$ROOT/dist"
VERSION="$(grep '^version = ' "$ROOT/Cargo.toml" | head -n 1 | sed 's/version = "\(.*\)"/\1/')"
HOST="$(rustc -vV | awk '/host:/ {print $2}')"
NAME="subdispatch-$VERSION-$HOST"

cd "$ROOT"
cargo test
cargo build --release

rm -rf "$TARGET_DIR/$NAME"
mkdir -p "$TARGET_DIR/$NAME"
cp "$ROOT/target/release/subdispatch" "$TARGET_DIR/$NAME/subdispatch"
cp "$ROOT/README.md" "$TARGET_DIR/$NAME/README.md"
cp "$ROOT/README_zh.md" "$TARGET_DIR/$NAME/README_zh.md"
cp "$ROOT/.env.example" "$TARGET_DIR/$NAME/.env.example"

(
  cd "$TARGET_DIR"
  tar -czf "$NAME.tar.gz" "$NAME"
)

printf '%s\n' "$TARGET_DIR/$NAME.tar.gz"
