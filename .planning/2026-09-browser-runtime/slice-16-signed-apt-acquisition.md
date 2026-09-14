# Slice 16 — Signed APT Acquisition and Managed Runtime Source

Slice 15 established an offline, content-addressed Browser runtime materializer from an already-verified official Linux ChatGPT `.deb`. Slice 16 closes the portable Linux distribution chain: verify OpenAI's signed APT metadata with a pinned repository key, select/download the matching package for the current architecture, materialize it through Slice 15, and make the resulting managed cache a production Browser runtime source.

This remains a post-V1 distribution extension. The frozen six Browser tools and manager/kernel semantics do not change.

## Proven upstream contract

The current official Linux repository is:

```text
https://persistent.oaistatic.com/codex-app-prod/linux/deb
```

The signed metadata entry point is:

```text
dists/stable/InRelease
```

Current `InRelease` is an RFC OpenPGP Cleartext Signature Framework document. Its signed body contains `SHA256:` entries for:

```text
main/binary-amd64/Packages
main/binary-arm64/Packages
```

The architecture-specific `Packages` file contains one current `chatgpt` stanza with `Version`, `Architecture`, `Filename`, `Size`, and `SHA256` fields. Current versioned `.deb` artifacts are below `pool/main/c/chatgpt/`.

The OpenAI repository public key is distributed inside the official ChatGPT package's `postinst` and identifies itself as `Codex Linux Repository`. Its pinned fingerprint is:

```text
3BFA0E4AE8B8CC16A2D9BA684A3B4A566C4660E4
```

Orchestrator live evidence on 2026-09-14 proved `pgp 0.20.0` can parse that exact public key and verify the current official `InRelease` via `CleartextSignedMessage::from_string(...).verify(...)`; the verified body contains the expected architecture `Packages` entries. This is the implementation basis, not a speculative library choice.

## Trust root

- Commit one small ASCII-armored copy of the official repository public key as a Browser distribution trust asset. Public key material is a trust anchor, not a secret.
- Runtime parsing must independently compute the parsed key's full fingerprint and require exact equality with the pinned fingerprint above before using it.
- Do not download repository verification keys at runtime. A key served by the same origin it authenticates is not an independent trust root.
- Use `pgp = 0.20.0` with default features disabled. Do not shell out to `gpg`, `gpgv`, `apt`, `dpkg`, or other system package-manager tools.
- Accept only SHA-2 cleartext signatures (`SHA256`, `SHA384`, or `SHA512`) from the pinned key. Reject SHA-1/MD5/unknown signature hashes even if the library could cryptographically verify them.
- Key rotation is intentionally fail-closed: a future OpenAI repository key requires an Agentic release that adds/replaces a reviewed pinned trust root. Do not silently fetch or trust a new key.

## Browser managed configuration

Extend the optional top-level `browser` section with a sparse managed-runtime policy:

```json
{
  "browser": {
    "managed": {
      "enabled": true,
      "autoProvision": false
    }
  }
}
```

- `managed.enabled` defaults to `true`.
- `managed.autoProvision` defaults to `false`.
- Sparse/default serialization should omit a default managed section.
- No repository URL, key, package URL, cache path, arbitrary headers, proxy setting, version string, or raw environment map is configurable in this slice.
- Runtime config remains restart-required; do not add Browser hot swap.

The default paths are internal Agentic policy, derived from `agentic_home()`:

```text
managed cache: ~/.agentic_gpt/cache/browser-runtime
managed CODEX_HOME: ~/.agentic_gpt/browser-runtime/codex-home
```

Create the managed CODEX_HOME as an Agentic-owned private directory before constructing a managed descriptor. It is persistent runtime state, not a Codex installation and not a Codex CLI path.

## Production source selection

Runtime source selection becomes asynchronous because provisioning may perform bounded network I/O. Resolve the Browser runtime before constructing the final `AppState`, while keeping the resulting `BrowserRuntimeContext` source-agnostic.

Selection order is frozen:

1. **Explicit override** (`browser.runtime`) if configured. It remains authoritative. Invalid explicit config disables Browser for that process; never fall through to another source.
2. **Existing managed cache** when `browser.managed.enabled=true`. A valid active managed artifact is preferred over Desktop discovery.
3. **Managed auto-provision** when enabled and `autoProvision=true`, if no valid managed cache was selected. Attempt one signed APT acquisition/materialization; if it succeeds, select the resulting managed descriptor.
4. **Desktop registry** if managed cache/provision did not yield a runtime. This preserves zero-download behavior on ordinary Desktop hosts when `autoProvision=false` and gives an opt-in managed path when requested.
5. Otherwise Browser is unavailable, while Agentic startup itself remains fail-open.

Managed cache corruption/missing active metadata is never treated as a valid runtime. With `autoProvision=true`, acquisition may repair/provision it; otherwise source selection may continue to the trusted Desktop fallback with a bounded diagnostic. Do not choose an arbitrary older artifact by directory scan.

`autoProvision=false` must perform no Browser distribution network requests.

## Platform mapping

Only Linux targets supported by Slice 15 are provisioned:

```text
x86_64  -> target linux-x64   -> APT architecture amd64
aarch64 -> target linux-arm64 -> APT architecture arm64
```

Other OS/architecture combinations skip managed auto-provision with a stable unsupported error/diagnostic and may still use an explicit source if one was provided. Do not invent macOS/Windows distribution in this slice.

## HTTP boundary

Use Agentic's Rust `reqwest`/rustls stack. The upstream base URL and host are constants.

- HTTPS only.
- Host must be exactly `persistent.oaistatic.com`.
- Disable automatic redirects for these distribution requests; any redirect is rejected rather than following to an unreviewed host.
- Require HTTP 200 for metadata and package responses.
- Honor the process/network environment according to existing reqwest behavior; do not add Browser-specific proxy credentials or bypass policy.
- Use explicit connect/request/overall provisioning bounds. No network await is unbounded.
- Response diagnostics are bounded and do not include arbitrary response bodies.

Maximum accepted sizes:

```text
InRelease: 256 KiB
Packages:    8 MiB
.deb:        1 GiB
```

The `.deb` package's signed `Size` field must also fit within that cap.

## InRelease verification

Fetch `dists/stable/InRelease` into a bounded buffer.

1. Parse the pinned ASCII-armored public key with rPGP and verify its exact fingerprint.
2. Parse the response with `CleartextSignedMessage`.
3. Require at least one signature and verify against the pinned key.
4. Require the verified matching signature's hash algorithm to be SHA-2 as defined above.
5. Parse **only the authenticated cleartext returned by the verified cleartext message**. Never parse an unsigned parallel `Release` body.
6. Require `Suite: stable` and/or `Codename: stable` to be internally consistent with the frozen repository endpoint; reject contradictory suite/codename metadata.
7. Require exactly one SHA256 entry for the target architecture's uncompressed `Packages` path. Validate the digest as lowercase 64-hex and size as bounded decimal.

Do not use MD5/SHA1 entries for trust decisions. SHA512 entries may be present but SHA256 is the frozen downstream metadata digest for this slice.

The repository currently publishes a signed `Date` but no `Valid-Until`. Parse/validate the date format for diagnostics, but do not invent an arbitrary expiry interval. HTTPS + pinned signed metadata authenticates the first acquisition; automatic update/freshness/rollback policy is a future updater concern because this slice provisions only when no valid managed runtime is already selected.

## Packages verification and parsing

Fetch exactly the authenticated architecture path from the fixed repository base.

- Bound bytes by both the signed InRelease size and the global Packages cap.
- Require the received byte count to equal the signed size.
- Stream/compute SHA-256 and require exact equality with the signed InRelease digest before parsing any package record as trusted input.
- Parse Debian control paragraphs without executing or invoking apt/dpkg.
- Continuation fields may be ignored for fields not used by this selector, but malformed field syntax, duplicate trust-relevant fields, invalid UTF-8, or oversized field/paragraph counts must fail closed.
- Select a record only when `Package: chatgpt` and `Architecture` exactly matches `amd64`/`arm64` for the host.
- For this first provisioning slice, require **exactly one** matching `chatgpt` record. If the repository later exposes multiple versions simultaneously, fail closed with a stable ambiguity error rather than inventing an incomplete Debian version-order implementation.

Required selected fields:

```text
Package
Version
Architecture
Filename
Size
SHA256
```

- `Version` must be non-empty, bounded, and safe as the Slice 15 artifact version path component. Do not sanitize/rename it silently.
- `Filename` must be a relative forward-slash path with only normal components, no scheme/query/fragment/backslash/traversal, must remain under the fixed repository base, and must be under `pool/main/c/chatgpt/` with a `.deb` suffix.
- `Size` must be decimal, non-zero, and at most 1 GiB.
- `SHA256` must be lowercase 64-hex.

The `channel` supplied to `VerifiedBrowserPackage` is frozen to `prod`. Do **not** use APT suite `stable` as `BROWSER_USE_CODEX_APP_BUILD_FLAVOR`; current official Desktop runtime evidence uses `prod`.

## Package download

Download the selected versioned `.deb` to an acquisition-owned unique temporary file beneath the Browser managed cache boundary; Slice 15 must still receive a normal path and independently rehash it.

- Before network acquisition under `autoProvision=true`, serialize same-target provisioning with a dedicated provisioning lock. A second process waits/rechecks the active managed cache after acquiring the lock so concurrent startup does not duplicate a ~380 MiB download.
- Provision-lock stale-owner recovery follows the accepted Slice 15 Linux PID + bounded-age pattern; do not nest the exact Slice 15 materializer lock in a way that self-deadlocks.
- Stream the body to disk; never buffer the `.deb` in memory.
- Enforce signed expected size during streaming and reject overshoot immediately.
- Compute SHA-256 while streaming and require the exact signed Packages digest before invoking Slice 15.
- Sync the completed temporary file before materialization.
- Call the accepted Slice 15 materializer with `target`, signed `Version`, `channel="prod"`, exact package SHA256, and Agentic-owned managed CODEX_HOME.
- The acquisition layer owns the downloaded temporary `.deb` and removes it best-effort after materialization success or failure. Slice 15 continues to never delete its caller's input.
- If provisioning fails, do not publish/activate any new managed runtime. Continue source selection to Desktop fallback where allowed by the source-order rules above.

## Startup integration

Refactor startup narrowly so the async runtime source is resolved before final `AppState` construction.

- `run_hub`, `run_stdio_worker`, and `run_local` must all use the same production resolver.
- Standalone supervised worker reaches the same resolver through its existing stdio-worker process; do not duplicate provisioning in the supervisor and worker.
- `build_app_state` should receive or otherwise consume the already-resolved optional Browser context rather than performing hidden network I/O in a synchronous constructor.
- Ordinary unit-test AppState builders remain deterministic and do not hit the host network/cache.
- Existing explicit-source fail-open behavior and source diagnostics remain bounded and truthful.
- Successful source diagnostics may identify only stable labels (`explicit-config`, `managed-cache`, `managed-provision`, `desktop-registry`) and bounded runtime version/target metadata; do not print signed metadata bodies or package URLs containing untrusted fields.

## Tests

Network tests use local controllable HTTP fixtures or injected fetch seams. They must never depend on the live OpenAI CDN for ordinary `cargo test`.

Cover at minimum:

1. pinned public-key asset parses and fingerprint equals the frozen OpenAI fingerprint;
2. a generated/checked-in test cleartext signed fixture verifies and tampered body/signature fails;
3. unsupported weak signature hash is rejected by policy even if cryptographically valid;
4. InRelease parser selects only the target uncompressed Packages SHA256+size and rejects duplicate/malformed/contradictory suite entries;
5. Packages bytes must match signed size+SHA256 before package parsing;
6. Packages selector requires exactly one `chatgpt` + matching architecture record and rejects ambiguity, duplicate trust fields, unsafe filename, invalid size/hash/version;
7. HTTP rejects redirects/non-200/oversized metadata and never follows a different host;
8. `.deb` streaming rejects size/hash mismatch/overshoot and never returns unverified bytes to Slice 15;
9. same-target provision attempts serialize and recheck cache; unrelated target lock identity remains separate;
10. acquisition temp package is removed after both materializer success and failure;
11. managed config defaults/sparse round trip: enabled true, autoProvision false;
12. `autoProvision=false` performs zero distribution network calls;
13. source order: explicit > managed cache > opt-in managed provision > Desktop > unavailable;
14. invalid explicit source never falls through;
15. corrupt/missing managed cache can provision when opted in and may fall back to Desktop if provisioning cannot succeed;
16. managed descriptor uses Agentic-owned CODEX_HOME, `codex_cli_path=None`, `channel=prod`, and accepted Slice 15 paths;
17. all three production run modes share the resolver seam without duplicate supervisor download behavior;
18. existing Browser tools/manager/runtime/full package tests stay green.

Include one **opt-in/manual live acquisition smoke** after orchestrator review that verifies the current official InRelease/key/Packages chain and, when explicitly exercised, may download the real current package into a temporary managed cache. Ordinary test suites do not download it.

## Verification

- focused acquisition/trust/parser/source-selection suites;
- `cargo test -p agentic-gpt browser_distribution`;
- `cargo test -p agentic-gpt browser_runtime`;
- `cargo test -p agentic-gpt browser_`;
- relevant config/startup tests;
- `cargo test -p agentic-gpt --no-run` during implementation;
- orchestrator full `cargo test -p agentic-gpt`;
- `cargo fmt --all -- --check`;
- `git diff --check`;
- orchestrator opt-in live signed-metadata smoke before final acceptance.

## Non-goals

- no automatic update check when a valid managed cache is already selected
- no background updater/scheduler or retention/garbage collection
- no arbitrary repository/key/URL override
- no Debian dependency installation and no ChatGPT Desktop installation
- no `apt`, `dpkg`, `gpg`, `gpgv`, shell package-manager invocation, or root requirement
- no Debian multi-version ordering logic; ambiguity fails closed
- no macOS/Windows acquisition
- no Browser semantic/tool/manager changes
- no Neko-specific logic
- no Codex CLI installation or dependency
- no mirror/redistribution of OpenAI proprietary runtime artifacts; packages are fetched directly from the official source onto the user's machine

## Worker rules

- Read `AGENTS.md`, frozen `PLAN.md`, Slice 15 contract, this Slice 16 contract, current `progress.md`/`findings.md`, accepted `browser_distribution.rs`, `browser_runtime.rs`, config Browser structs, current runtime source resolver/AppState construction, and `tunnel_distribution.rs` only as repository pattern evidence before editing.
- The exact official trust evidence above is frozen. Do not replace pinned verification with TLS-only hashes or a downloaded key.
- Keep network trust, package parsing, and startup source selection explicit and testable. Do not bury acquisition inside a generic helper that obscures which signed layer authenticated which digest.
- This is a nontrivial multi-file implementation: retain worker session/state and do not impose a hard wall-clock timeout.
- Update `progress.md` at meaningful checkpoints/completion and `findings.md` only for actual upstream/contract contradictions or new evidence.
- Do not modify the frozen V1 PLAN or six-tool contracts.
- Do not commit/push or destructively clean. Leave `experimental/chrome-control-poc/__pycache__/` untouched.
