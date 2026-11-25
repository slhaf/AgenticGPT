#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

TARGETS=(
  "x86_64-unknown-linux-gnu"
  "aarch64-unknown-linux-gnu"
  "x86_64-unknown-linux-musl"
  "aarch64-unknown-linux-musl"
)
BINS=(
  "agentic-gpt"
  "agentic-gpt-hub"
)
BUILD_DIR="${CARGO_TARGET_DIR:-target}"

if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "aarch64" ]]; then
  echo "error: dist-linux-on-arm.sh requires an aarch64 Linux host" >&2
  exit 1
fi

if ! command -v cargo-zigbuild >/dev/null 2>&1; then
  echo "error: cargo-zigbuild and Zig are required for ARM Linux release builds" >&2
  echo "on Arch Linux, install them with: sudo pacman -S --needed cargo-zigbuild zig" >&2
  exit 1
fi

installed_targets=" $(rustup target list --installed | tr '\n' ' ') "
for target in "${TARGETS[@]}"; do
  if [[ "$installed_targets" != *" $target "* ]]; then
    echo "error: Rust standard library is missing for $target" >&2
    echo "install it with: rustup target add ${TARGETS[*]}" >&2
    exit 1
  fi
done

for target in "${TARGETS[@]}"; do
  if [[ "$target" == "aarch64-unknown-linux-gnu" ]]; then
    cargo build --release --locked --target "$target" --target-dir "$BUILD_DIR" --workspace
  else
    cargo zigbuild --release --locked --target "$target" --target-dir "$BUILD_DIR" --workspace
  fi
  mkdir -p "dist/$target"
  for bin in "${BINS[@]}"; do
    cp "$BUILD_DIR/$target/release/$bin" "dist/$target/$bin"
  done
  scripts/stage-linux-licenses.sh "dist/$target"
done

printf 'release artifacts written to:\n'
for target in "${TARGETS[@]}"; do
  for bin in "${BINS[@]}"; do
    printf '  %s\n' "dist/$target/$bin"
  done
done
