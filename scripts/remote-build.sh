#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

SESSION_NAME="${MUTAGEN_SESSION_NAME:-agenticgpt-laptop}"
REMOTE="${REMOTE_BUILD_HOST:-slhaf-laptop}"
REMOTE_DIR="${REMOTE_BUILD_DIR:-~/Projects/RemoteBuild/AgenticGPT}"
LOCAL_DIST_DIR="${LOCAL_DIST_DIR:-dist/remote}"

BINS=(
  "agentic-gpt"
  "agentic-gpt-hub"
)

usage() {
  cat <<USAGE
usage: $0 [check|build|test|release|dist|clean|copy-release]

Environment overrides:
  MUTAGEN_SESSION_NAME  default: agenticgpt-laptop
  REMOTE_BUILD_HOST     default: slhaf-laptop
  REMOTE_BUILD_DIR      default: ~/Projects/RemoteBuild/AgenticGPT
  LOCAL_DIST_DIR        default: dist/remote

Modes:
  check         flush Mutagen, then run cargo check on the remote host
  build         flush Mutagen, then run cargo build on the remote host
  test          flush Mutagen, then run cargo test on the remote host
  release       flush Mutagen, run cargo build --release --workspace, then copy native binaries back
  dist          flush Mutagen, run scripts/dist-linux.sh remotely, then copy remote dist/ back
  clean         run cargo clean on the remote host
  copy-release  only copy native release binaries back from the remote host
USAGE
}

remote_exec() {
  local command="$*"
  ssh "$REMOTE" "bash -lc 'export PATH="\$HOME/.cargo/bin:\$HOME/.local/bin:\$PATH"; cd $REMOTE_DIR && $command'"
}

flush_sync() {
  echo "==> Flushing Mutagen session: $SESSION_NAME"
  mutagen sync flush "$SESSION_NAME"
}

copy_native_release() {
  local dest="$LOCAL_DIST_DIR/native"
  mkdir -p "$dest"

  echo "==> Copying native release binaries to: $dest"
  for bin in "${BINS[@]}"; do
    rsync -az "$REMOTE:$REMOTE_DIR/target/release/$bin" "$dest/$bin"
  done

  echo "==> Copied binaries:"
  for bin in "${BINS[@]}"; do
    printf '  %s\n' "$dest/$bin"
  done
}

copy_remote_dist() {
  mkdir -p "$LOCAL_DIST_DIR"

  echo "==> Copying remote dist/ to: $LOCAL_DIST_DIR/"
  rsync -az --delete "$REMOTE:$REMOTE_DIR/dist/" "$LOCAL_DIST_DIR/"

  echo "==> Remote dist copied to: $LOCAL_DIST_DIR/"
}

mode="${1:-check}"

case "$mode" in
  check)
    flush_sync
    remote_exec "cargo check"
    ;;
  build)
    flush_sync
    remote_exec "cargo build"
    ;;
  test)
    flush_sync
    remote_exec "cargo test"
    ;;
  release)
    flush_sync
    remote_exec "cargo build --release --workspace"
    copy_native_release
    ;;
  dist)
    flush_sync
    remote_exec "bash scripts/dist-linux.sh"
    copy_remote_dist
    ;;
  clean)
    remote_exec "cargo clean"
    ;;
  copy-release)
    copy_native_release
    ;;
  -h|--help|help)
    usage
    ;;
  *)
    usage >&2
    exit 1
    ;;
esac
