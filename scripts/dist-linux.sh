#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

TARGETS=(
  "x86_64-unknown-linux-gnu"
  "aarch64-unknown-linux-gnu"
)
BINS=(
  "agentic-gpt"
  "agentic-gpt-hub"
)

if ! command -v cross >/dev/null 2>&1; then
  echo "error: cross is required for multi-target Linux release builds" >&2
  echo "install it with: cargo install cross --git https://github.com/cross-rs/cross" >&2
  exit 1
fi

for target in "${TARGETS[@]}"; do
  cross build --release --target "$target" --workspace
  mkdir -p "dist/$target"
  for bin in "${BINS[@]}"; do
    cp "target/$target/release/$bin" "dist/$target/$bin"
  done
done

printf 'release artifacts written to:\n'
for target in "${TARGETS[@]}"; do
  for bin in "${BINS[@]}"; do
    printf '  %s\n' "dist/$target/$bin"
  done
done
