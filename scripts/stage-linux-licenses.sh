#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${1:?usage: stage-linux-licenses.sh <target-dir>}"

mkdir -p "$TARGET_DIR/licenses"
cp "$ROOT_DIR/LICENSE" "$TARGET_DIR/LICENSE"
cp "$ROOT_DIR/licenses/THIRD_PARTY_NOTICES.md" "$TARGET_DIR/THIRD_PARTY_NOTICES.md"
cp "$ROOT_DIR"/licenses/third-party/*.txt "$TARGET_DIR/licenses/"
