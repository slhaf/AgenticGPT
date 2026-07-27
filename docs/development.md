# Development

This document covers source-based development, local verification, CI, and release publishing. For normal installation and usage, start with the main [README](../README.md).

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
- `dist/aarch64-unknown-linux-gnu/agentic-gpt`
- `dist/aarch64-unknown-linux-gnu/agentic-gpt-hub`

Pushing a version tag builds Linux release archives and publishes a GitHub Release:

```bash
git tag v0.9.0
git push origin v0.9.0
```

Release archives contain both binaries for one target:

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

The release workflow is triggered by version tags matching `v*`. It uses `scripts/dist-linux.sh`, packages both binaries per target, writes `SHA256SUMS`, and publishes a GitHub Release.
