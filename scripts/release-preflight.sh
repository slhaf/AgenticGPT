#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ -n "${RELEASE_TAG:-}" ]]; then
  expected_version="${RELEASE_TAG#v}"
  if [[ "$RELEASE_TAG" == "$expected_version" ]]; then
    echo "error: RELEASE_TAG must start with v (got $RELEASE_TAG)" >&2
    exit 1
  fi

  metadata="$(cargo metadata --no-deps --format-version=1)"
  runtime_versions="$(
    python3 -c '
import json
import sys

packages = {package["name"]: package["version"] for package in json.load(sys.stdin)["packages"]}
for name in ("agentic-gpt", "agentic-gpt-hub"):
    print("{}\t{}".format(name, packages.get(name, "")))
' <<<"$metadata"
  )"
  while IFS=$'\t' read -r package version; do
    if [[ -z "$version" ]]; then
      echo "error: Rust runtime package $package is missing from cargo metadata" >&2
      exit 1
    fi
    if [[ "$version" != "$expected_version" ]]; then
      echo "error: $RELEASE_TAG does not match $package Cargo version $version" >&2
      exit 1
    fi
  done <<<"$runtime_versions"
  echo "release tag $RELEASE_TAG matches agentic-gpt and agentic-gpt-hub Cargo versions $expected_version"
fi

cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo build -p agentic-gpt -p agentic-gpt-hub

contract_env="${CONTRACT_ENV:-target/contract-venv}"
contract_python="${CONTRACT_PYTHON:-$contract_env/bin/python}"
if [[ ! -x "$contract_python" ]]; then
  python3 -m venv "$contract_env"
  contract_python="$contract_env/bin/python"
fi
"$contract_python" -m pip install "PyYAML" "jsonschema[format]>=4.25,<5"
"$contract_python" scripts/check_contract_parity.py
