# Slice 15 — Managed Browser Runtime Cache Materializer

This slice starts a post-V1 distribution extension. The frozen V1 Browser tool surface and lifecycle remain unchanged. Slice 15 accepts an already-authenticated/verified official Linux ChatGPT `.deb` package and materializes only the official Browser runtime resources into an Agentic-owned versioned cache. It does **not** fetch network metadata or packages; the signed APT acquisition chain is a later slice.

## Goal

Provide one source-agnostic managed-cache layer that can turn a verified official Linux ChatGPT package into the existing `BrowserRuntimeDescriptor` without requiring ChatGPT/Codex Desktop, an installed Codex CLI, `dpkg-deb`, `ar`, or a pre-existing Codex plugin cache.

The current official amd64 and arm64 packages both use the standard Debian `ar` container and a `data.tar.xz` payload. The selected current installed resources contain no symlinks.

## Cache layout

Default root is owned by Agentic under its existing home/cache boundary:

```text
~/.agentic_gpt/cache/browser-runtime/
  artifacts/
    linux-x64/
      <app-version>/
        <deb-sha256>/
          runtime.json
          cua_node/
          chrome/
  active/
    linux-x64.json
  .locks/
  .staging/
```

ARM64 uses `linux-arm64` in the same layout.

- `runtime.json` is Agentic-owned metadata, not an OpenAI file. It records schema version, target, app version, channel, package SHA-256, install time, and the relative resource roots used to derive the descriptor. It never stores machine-external absolute source paths.
- The artifact identity is target + app version + verified `.deb` SHA-256. Same version with a different package digest is a distinct artifact and must never silently overwrite the old one.
- `active/<target>.json` contains only the active artifact identity. Update it atomically after the new artifact is fully validated and activated.
- No symlink is required for activation; descriptor paths resolve from the active metadata to the immutable versioned artifact directory.

## Input contract

Add an internal materialization input equivalent to:

```text
VerifiedBrowserPackage {
  deb_path,
  deb_sha256,
  app_version,
  channel,
  target,
  codex_home,
}
```

- `deb_path` is an already-downloaded regular file.
- `deb_sha256` is a lowercase 64-hex digest supplied by the future verified APT layer. Slice 15 must independently stream-hash the package and reject mismatch before archive extraction.
- `target` is currently only `linux-x64` or `linux-arm64`; host/target mismatch is rejected before installation.
- `codex_home` is a persistent Agentic-owned writable path outside the immutable artifact directory. It is used only for the existing Browser launch contract and trusted-code derivation. Slice 15 does not create or discover a Codex CLI.

## Debian package parser

Implement the small required `ar` reader in Rust; do not shell out to `ar` or `dpkg-deb`.

- Require exact global magic `!<arch>\n`.
- Parse bounded 60-byte classic member headers with decimal sizes and even-byte padding.
- Require one `debian-binary` member whose content is exactly `2.0\n`.
- Require exactly one `data.tar.xz` member for this slice. Other `data.tar.*` compression is rejected with stable `browser_runtime_package_compression_unsupported`; do not guess/decompress unknown formats.
- Duplicate `debian-binary` or data members, malformed sizes/headers, truncated members, or member ranges outside the file are stable package-format errors.
- Do not load the 350–400 MiB data member into memory. Locate it by offset/length and stream it.

## Safe selective extraction

Use Rust archive/decompression libraries, not external `tar`/`xz` commands. Add only the minimal crates necessary for streaming XZ + tar handling.

From `data.tar.xz`, materialize **only** these normalized source prefixes:

```text
usr/lib/chatgpt/resources/cua_node/**
usr/lib/chatgpt/resources/plugins/openai-bundled/plugins/chrome/**
```

Accept an optional leading `./` in tar entry names, then normalize structurally. Do not accept backslash aliases, absolute paths, `.`/`..` traversal components, duplicate selected file paths, or paths that escape the two frozen prefixes.

- Selected entries may be directories or regular files only.
- Reject selected symlinks, hardlinks, devices, FIFOs, sockets, and other special types.
- Preserve ordinary executable/read permission bits needed by `node`, `node_repl`, and package resources, but never preserve setuid/setgid/sticky bits.
- Do not invoke tar's generic `unpack`/`unpack_in` over the package tree; write selected entries explicitly beneath the staging roots.
- Bound selected-entry count to 10,000 and total selected extracted bytes to 768 MiB. Bound individual selected files to 256 MiB. Exceeding a bound is a stable error and leaves no active partial artifact.
- Ignore unrelated package entries without writing them.

The resulting cache roots are renamed to `cua_node/` and `chrome/`; the original `/usr/lib/chatgpt/...` hierarchy is not retained inside the artifact.

## Descriptor derivation and validation

After extraction, validate all required descriptor anchors as regular non-symlink files/directories:

```text
cua_node/bin/node
cua_node/bin/node_repl
cua_node/lib/node_modules/
chrome/scripts/browser-client.mjs
chrome/scripts/browser-service.mjs
chrome/docs/
```

On Unix, `node` and `node_repl` must be executable.

Derive the existing descriptor as:

- `app_version = input.app_version`
- `channel = input.channel`
- `node_repl_path = artifact/cua_node/bin/node_repl`
- `node_path = artifact/cua_node/bin/node`
- `browser_client_path = artifact/chrome/scripts/browser-client.mjs`
- `browser_service_path = artifact/chrome/scripts/browser-service.mjs`
- `codex_home = input.codex_home`
- `codex_cli_path = None`
- `node_module_dirs = [artifact/cua_node/lib/node_modules]`
- `trusted_code_paths` and `docs_root` use the accepted shared descriptor helpers; do not duplicate their semantics.

Validate a descriptor from a completed cached artifact before marking it active. Cache discovery must reject a malformed active manifest, missing artifact, symlinked critical path, wrong target/version/hash identity, or missing required resource without silently selecting another arbitrary artifact.

## Atomicity and concurrency

- Serialize same-target install/activation with an Agentic-owned lock under `.locks/`; a different target may proceed independently.
- Download is out of scope, so the input `.deb` stays outside this cache transaction and must never be deleted by Slice 15.
- Extract into a unique `.staging/` directory under the cache root.
- Write and sync Agentic metadata in staging, validate the complete descriptor there, then atomically rename staging to the content-addressed final artifact directory.
- If a valid identical artifact already exists, reuse it rather than replacing it.
- Corrupt existing final artifacts must not be trusted. Repair through a new staging directory and replace only under the held target lock; never expose a half-written final artifact.
- Update the target's active manifest only after final-artifact validation. Active-manifest write uses temp-file + sync + atomic rename.
- Best-effort cleanup of stale staging is allowed only for installation-owned paths; never use broad `git clean`/home-directory cleanup semantics.

## Source boundary

Slice 15 does **not** change production startup selection yet. Existing source behavior remains:

1. explicit override if configured;
2. Desktop registry discovery otherwise.

The next distribution slice will add verified APT acquisition and managed-cache source integration. Do not make startup download hundreds of MiB in this slice.

## Errors

Use bounded stable error codes with `browser_runtime_cache_*` or `browser_runtime_package_*` prefixes. Never include full external package/cache paths in model-facing error strings. Internal logs may use bounded sensitive diagnostics according to existing Agentic conventions.

## Tests

Use generated tiny `.deb` fixtures; tests must never download the real 380 MiB package.

Cover at minimum:

1. classic `ar` parsing, padding, `debian-binary`, and streamed data-member offset/length;
2. malformed/truncated/duplicate members and unsupported compression;
3. package SHA mismatch is rejected before extraction;
4. selective extraction maps the two prefixes to `cua_node/` and `chrome/` only;
5. traversal/absolute/backslash, duplicate selected files, symlink/hardlink/special types are rejected;
6. entry/individual/total extraction bounds;
7. executable bits for tiny fake `node`/`node_repl` survive while privileged bits do not;
8. descriptor derivation has `codex_cli_path=None`, correct docs/trusted/node-module paths, and no Desktop/Codex path dependency;
9. repeated identical install is idempotent;
10. same-target installers serialize; different targets do not share one global lock;
11. corrupt final artifacts are not returned as valid and can be repaired atomically;
12. active-manifest corruption/missing target fails closed without arbitrary-version fallback;
13. unrelated package entries are never materialized;
14. existing Browser runtime/manager/tool tests remain green.

## Verification

- focused `browser_distribution` tests;
- `cargo test -p agentic-gpt browser_runtime`;
- `cargo test -p agentic-gpt browser_`;
- `cargo test -p agentic-gpt --no-run` during implementation;
- after orchestrator review, full `cargo test -p agentic-gpt`;
- `cargo fmt --all -- --check`;
- `git diff --check`.

## Non-goals

- no network download/fetch or APT index parsing
- no PGP/InRelease verification yet
- no production startup source-order change
- no automatic update scheduler/background updater
- no retention/garbage-collection policy beyond exact staging cleanup
- no macOS/Windows package support
- no committed OpenAI runtime files or test fixtures copied from the real package
- no Neko-specific runtime logic
- no Browser tool/lifecycle/manager semantic changes
- no new model-facing Browser tools

## Worker rules

- Read `AGENTS.md`, frozen `PLAN.md`, this slice, current `progress.md`/`findings.md`, accepted `browser_runtime.rs`, and the existing `tunnel_distribution.rs` only as implementation-pattern evidence before editing.
- Treat this slice as a post-V1 distribution extension; do not rewrite the frozen V1 PLAN to make managed provisioning look retroactively in-scope.
- Architecture is frozen by this contract. Report a genuine contradiction rather than broadening the surface.
- This is a nontrivial multi-file implementation: retain session/state and use no hard wall-clock cutoff.
- Keep the cache module internal. Do not wire production source selection or network fetch.
- Update `progress.md` at meaningful checkpoints/completion; `findings.md` only for new evidence or a contract-vs-package contradiction.
- Do not commit/push, do not destructively clean, and leave `experimental/chrome-control-poc/__pycache__/` untouched.
