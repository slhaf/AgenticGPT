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
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 -c "import yaml; yaml.safe_load(open('openapi/hub.yaml')); print('openapi yaml ok')"
```

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
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- OpenAPI YAML parsing for `openapi/hub.yaml`

## Notes

The release workflow is triggered by version tags matching `v*`. It uses `scripts/dist-linux.sh`, packages all three binaries per target, writes `SHA256SUMS`, and publishes a GitHub Release.
