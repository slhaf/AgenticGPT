# Development

This document covers source-based development, local verification, CI, and release publishing. For normal installation and usage, start with the main [README](../README.md).

## Architecture before changes

Start with the [architecture guide](architecture/README.md) (Chinese): current deployment and module boundaries, evidence-backed diagnosis, target architecture draft, engineering rules, and a staged refactoring plan. Agentic is controlled execution infrastructure for upstream agents, not a general Agent Runtime. The guide distinguishes current behavior from proposed rules; no runtime migration is implied by the draft.

Changes to ownership, dependencies, public contracts, permissions, persistence, or deployment must update the relevant architecture document. Historical migration/release notes remain version-specific references, not the current architecture authority.

## Development from source

During development, replace binary commands with Cargo package commands:

```bash
cargo run -p agentic-gpt-hub -- init
cargo run -p agentic-gpt -- config init
cargo run -p agentic-gpt -- run
```

## Verification

```bash
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
cargo build -p agentic-gpt -p agentic-gpt-hub
python3 -m venv target/contract-venv
target/contract-venv/bin/python -m pip install "PyYAML" "jsonschema[format]>=4.25,<5"
target/contract-venv/bin/python scripts/check_contract_parity.py
cargo clippy --workspace --all-targets -- -D warnings
```

The parity gate uses the built `target/debug/agentic-gpt` and
`target/debug/agentic-gpt-hub` by default, or explicit `--agent-bin PATH` and
`--hub-bin PATH` values. It runs isolated loopback/private-home processes;
schema validation and live behavior are separate checks. The contract
environment is intentionally under ignored `target/contract-venv`.
Strict clippy runs after the runtime checks; this sequence makes no claim that
existing clippy findings are resolved or that CI is all green.

## Build and release

Local multi-target Linux release builds use `cross`:

```bash
cargo install cross --git https://github.com/cross-rs/cross
./scripts/dist-linux.sh
```

Artifacts are written to:

- `dist/x86_64-unknown-linux-gnu/agentic-gpt`
- `dist/x86_64-unknown-linux-gnu/agentic-gpt-hub`
- `dist/x86_64-unknown-linux-gnu/agentic-browser-host`
- `dist/aarch64-unknown-linux-gnu/agentic-gpt`
- `dist/aarch64-unknown-linux-gnu/agentic-gpt-hub`
- `dist/aarch64-unknown-linux-gnu/agentic-browser-host`

Pushing a version tag builds Linux release archives and publishes a GitHub Release:

```bash
git tag v0.9.0
git push origin v0.9.0
```

Release archives contain all three binaries for one target:

- `agentic-gpt-x86_64-unknown-linux-gnu.tar.gz`
- `agentic-gpt-aarch64-unknown-linux-gnu.tar.gz`
- `SHA256SUMS`

## CI

GitHub Actions runs CI on pushes and pull requests to `main`:

- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo test --workspace`
- `cargo build -p agentic-gpt -p agentic-gpt-hub`
- Create ignored `target/contract-venv` and install `PyYAML` plus `jsonschema[format]>=4.25,<5`.
- Run `python3 scripts/check_contract_parity.py` against the built binaries and isolated loopback/private-home processes.
- Run strict `cargo clippy --workspace --all-targets -- -D warnings` after the runtime/schema gate; this documentation does not claim that existing findings are resolved or that CI is all green.

## Notes

The release workflow is triggered by version tags matching `v*`. It uses `scripts/dist-linux.sh`, packages all three binaries per target, writes `SHA256SUMS`, and publishes a GitHub Release.
