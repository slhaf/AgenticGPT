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

The complete local verification sequence mirrors the current CI policy:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build -p agentic-gpt -p agentic-gpt-hub
python3 -m venv target/contract-venv
target/contract-venv/bin/python -m pip install "PyYAML" "jsonschema[format]>=4.25,<5"
target/contract-venv/bin/python scripts/check_contract_parity.py
```

The parity gate uses the built `target/debug/agentic-gpt` and
`target/debug/agentic-gpt-hub` by default, or explicit `--agent-bin PATH` and
`--hub-bin PATH` values. It runs isolated loopback/private-home processes;
schema validation and live behavior are separate checks. The contract
environment is intentionally under ignored `target/contract-venv`.

Strict Clippy (`-D warnings`) remains a CI policy and may report existing
findings. The sequence above is an explicit verification command, not a claim
that the current repository or every external release check is green.

## Build and release

### Local release preflight (does not publish)

Run the same non-publishing gate used before tag packaging:

```bash
bash scripts/release-preflight.sh
```

The script runs formatting, workspace check, workspace tests, Agent and Hub
builds, and the OpenAPI schema/live contract parity gate. It creates or reuses
`target/contract-venv` for the parity dependencies. To exercise tag/version
pairing locally from a commit checked out at a tag, use:

```bash
RELEASE_TAG="$(git describe --tags --exact-match)" bash scripts/release-preflight.sh
```

The preflight deliberately does not turn the repository's known strict Clippy
debt into a permanent publication blocker; strict Clippy remains enforced by
CI. Neither command publishes an artifact or requires release credentials.

### Version authorities and artifact pairing

Rust, the OpenAPI contract, and Console are independent release surfaces:

- The Rust release identity is the shared version of the `agentic-gpt` and
  `agentic-gpt-hub` packages in `crates/*/Cargo.toml`. Their CLI `--version`
  output comes from the corresponding Cargo package version. A Rust release
tag is `v<that version>`; the tag preflight rejects a mismatch. The bundled
  `agentic-browser-host` binary remains part of each archive but is not a
  second Rust release-tag authority.
- The HTTP contract authority is `openapi/hub.yaml`, including its
  `info.version`, paths, schemas, and responses. It is not inferred from the
  Rust tag. `scripts/check_contract_parity.py` validates that contract and the
  live Agent/Hub behavior from the same commit.
- Console package versions are owned by the Console Gradle targets, not this
  Rust workflow: Android uses `console/androidApp/build.gradle.kts`
  (`versionCode`/`versionName`) and desktop distributions use
  `console/desktopApp/build.gradle.kts` (`packageVersion`). Console versions
  may advance independently; Console is not built or published by the Rust
  release workflow. The version catalog in `console/gradle/libs.versions.toml`
  is dependency/plugin authority, not a Rust release number.

Local multi-target Linux distribution still uses `cross`:

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

For a release, create and push `v<agentic-gpt/agentic-gpt-hub Cargo version>`.
Do not copy a numeric tag from an old release note; the preflight checks the
actual tag against the checked-out Rust package versions:

```bash
rust_version="$(cargo metadata --no-deps --format-version=1 | python3 -c 'import json,sys; packages={p["name"]:p["version"] for p in json.load(sys.stdin)["packages"]}; print(packages["agentic-gpt"])')"
git tag "v${rust_version}"
git push origin "v${rust_version}"
```

Each target archive keeps the existing three-binary contents and names:

- `agentic-gpt-x86_64-unknown-linux-gnu.tar.gz`
- `agentic-gpt-aarch64-unknown-linux-gnu.tar.gz`
- `SHA256SUMS`

## CI

GitHub Actions runs CI on pushes and pull requests to `main`:

- `cargo fmt --all -- --check`
- `cargo check --workspace`
- strict `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo build -p agentic-gpt -p agentic-gpt-hub`
- Create ignored `target/contract-venv` and install `PyYAML` plus `jsonschema[format]>=4.25,<5`.
- Run `python3 scripts/check_contract_parity.py` against the built binaries and isolated loopback/private-home processes.

The release workflow is separate from branch/PR CI. Every `v*` tag starts a
`preflight` job on the tagged commit; the Linux packaging and publication job
requires that job with `needs: preflight`. Therefore a tag-only push cannot
bypass the release preflight, and any failed preflight blocks packaging and
publication. The tag workflow covers the listed fmt/check/test/build,
version-pairing, and schema/live parity checks. It does not claim strict
Clippy is green, ARM runtime behavior, an external Actions importer, Console
build behavior, or a production deployment; those remain separate boundaries.

GitHub Actions, cross compilation, external importers, and publication are not
run by this local documentation update. Existing versioned release notes remain
historical records and are intentionally preserved.
