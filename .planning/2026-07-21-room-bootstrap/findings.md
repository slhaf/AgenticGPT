# Findings & Decisions: Room Bootstrap

Plan scope: `.planning/2026-07-21-room-bootstrap`.

## Handoff Status

- **Maturity:** implementation_ready
- **Design role:** complete
- **Implementation authorized:** yes
- **Entry phase:** Phase 3
- **Frozen decisions:** D-01 through D-13
- **Open blocking decisions:** none
- **Product code changed during refinement:** no

## Requirements

- Add a Room Agent-only bootstrap facility exposed through Room-scoped MCP tools.
- Use a stable session entrypoint comparable in purpose to a workspace bootstrap instruction, but intended for repeated session startup rather than one-time onboarding.
- Keep `bootstrap.md` concise and place detailed usage conventions in separate guides.
- Parse YAML frontmatter for the entrypoint and guides.
- Discover guides generically; do not encode Diary or other capability families in implementation logic.
- Allow guides to bind tool names and describe when/how those tools should be used.
- Preserve the distinction between MCP schemas (availability and arguments) and guides (selection, workflow, conventions, safety, and recovery).
- Initial intended guide families include Diary, Notebook, execution/session/tmux, and skills, but V1 runtime must not special-case them.

## Existing implementation

- Room-specific capabilities already travel through `HubCommand` variants and the active Room connection. Notebook, Diary, and Skills handlers are dispatched in `crates/agentic-gpt/src/hub.rs`.
- Hub Room routes are centralized in `crates/agentic-gpt-hub/src/room.rs`; Room notebook and skills calls omit an external `agentId`.
- MCP tool registration, schemas, descriptions, command construction, read-only/destructive annotations, and tool-name regression tests live in `crates/agentic-gpt-hub/src/mcp_server.rs`.
- Protocol request/response types and serde command names live in `crates/agentic-gpt-protocol/src/lib.rs`.
- Local Room Agent source modules currently include `diary.rs`, `notebook.rs`, `skills.rs`, and `skill_installs.rs`; a sibling `bootstrap.rs` would match existing organization, subject to implementation discretion.
- `skills.read` already provides a close precedent for frontmatter parsing, package-relative resource reads, UTF-8/base64 representation, size bounds, SHA-256 metadata, symlink rejection, and workspace-only package roots.
- Room configuration currently has a `RoomConfig` with a nested `RoomSkillsConfig`; whether bootstrap needs its own config block depends on frozen limits and root policy.
- The MCP server currently contains a long static top-level `MCP_INSTRUCTIONS` string describing execution and skills selection. Bootstrap can become the durable home for detailed usage conventions, while the MCP instructions may retain only a concise cue to call `room.bootstrap`.


## Repository evidence: resource and routing conventions

### Phase 3 protocol checkpoint

- `HubCommand` is a serde-tagged enum with explicit public `#[serde(rename = ...)]` command names and camelCase fields; Room commands are grouped beside notebook/diary commands.
- Existing `SkillResource` has a different encoding contract (`utf8 | base64`) and lacks truncation/line fields, so the frozen bootstrap contract requires a dedicated `BootstrapTextResource` and `BootstrapEncoding::Utf8` rather than reusing it directly.
- Protocol tests are inline in `crates/agentic-gpt-protocol/src/lib.rs`; existing tests assert serialized `type`, `requestId`, payload fields, and omission of optional fields. Bootstrap compatibility tests should follow this pattern.
- The protocol crate has only `chrono`, `serde`, and `serde_json` dependencies; the new model should remain dependency-free beyond those existing types.

### Phase 4 loader checkpoint

- The local Room dispatch pattern checks `state.run_mode == RunMode::Room` before invoking Room-only modules and serializes module results into the existing `AgentMessage::Response` path.
- `Config.workspace_root` is already available to Room modules and `serde_yaml`/`sha2` are existing local-agent dependencies, so bootstrap needs no configuration or dependency changes.
- Bootstrap entrypoint metadata errors must be collapsed to the frozen public `bootstrap_invalid` code at the loader boundary; internal validation details remain diagnostics only for optional guide warnings.
- The loader keeps valid guide bytes in a per-call in-memory index so the manifest and ID-based read share the same observed package revision; no workspace state is created or mutated.

### Phase 5 Hub surface checkpoint

- Hub `agents::command_request_id` and `set_command_request_id`, plus `runs::command_type`, are exhaustive matches over `HubCommand`; both must add the two bootstrap variants for replay/idempotency and dispatch compilation.
- Room HTTP APIs use `request_active_room` and a shared `forward_room_command`; adding no-body `room_bootstrap` and JSON-body `room_bootstrap_read` preserves active-Room routing and timeout behavior.
- `room_value_response` currently maps a small set of not-found/conflict codes and defaults other agent errors to 400. Bootstrap must add `bootstrap_not_found`/`guide_not_found` as 404 and `bootstrap_read_failed` as 500.
- MCP tool annotations are centrally derived from tool names. New bootstrap tools are read-only by default if they are absent from mutation, destructive, and open-world match sets; their schemas should contain no `agentId`.
- GPT Actions/OpenAPI is manually maintained and has regression tests that assert Room routes, operation IDs, schemas, and no `agentId`; bootstrap needs explicit paths and component schemas rather than generated output.

### Phase 6 documentation checkpoint

- `docs/interfaces.md` is the repository's canonical public interface map, so the bootstrap authoring model and both public surfaces are documented there; README top-level feature lists only need concise pointers.
- Documentation examples can name Diary, Notebook, execution/session, and skills conventions while keeping the implementation generic because capability names are authored `toolBindings`/body content, not runtime guide branches.

### Phase 7 delivery checkpoint

- The final D-01 through D-13 audit found no contract drift across the dedicated protocol types, local loader, Hub command helpers, Room HTTP routes, MCP tools, OpenAPI schemas, and interface documentation. The public identity remains the active Room Agent; both operations have no `agentId`, are read-only/non-destructive/non-open-world, and Actions operations are non-consequential.
- The loader now avoids accumulating guide bodies during manifest calls. It reads and hashes each candidate for validation/revision, retains no body for `room.bootstrap`, and retains only the requested valid guide for `room.bootstrap.read`; complete-file size, line count, and SHA-256 semantics are unchanged.
- Documentation explicitly lists all implemented stable warning prefixes, including entrypoint/guide truncation and unreadable guide-directory entries. The implementation continues to use fixed `<workspaceRoot>/bootstrap`, direct lowercase `.md` guide discovery, generic typed metadata with raw detail retention, deterministic priority/ID order, and full-file revision membership for valid guides beyond the 64-item manifest.
- Focused protocol, loader, and Hub tests plus formatting and workspace check passed. The full workspace test caveat is environmental/pre-existing and reproducible only in the Diary test at the local pre-05:00 logical-day boundary; bootstrap-specific behavior is unaffected.

- `SkillResource` is a stable protocol object with `path`, `encoding`, `content`, optional `mediaType`, `sizeBytes`, and `sha256`; encoding is `utf8` or `base64`.
- `skills.read` normalizes a package-relative path, rejects empty/absolute/parent/backslash paths, rejects symlinks at every component, requires a regular file, and caps a returned resource at 1 MiB.
- Frontmatter parsing currently normalizes CRLF, recognizes a leading YAML block, converts an object to JSON, and reports malformed/non-object YAML through warnings rather than rejecting the skill package.
- Skill discovery skips hidden directories, validates stable IDs, sorts through a `BTreeMap`, and treats malformed optional metadata as warnings.
- Room HTTP handlers use `request_active_room` with no caller-supplied `agentId`; missing active Room maps to `room_not_active`, inconsistent state to `room_state_conflict`, and timeouts to operation-specific gateway timeout codes.
- MCP annotations are centrally derived. A new bootstrap read tool will be read-only by default as long as it is not added to mutation/open-world/destructive sets.
- `skills.list` and `skills.read` exist in both MCP and `/v1/room/skills/*` HTTP routes, establishing a strong surface-parity precedent for Room package discovery/read operations.
- `RoomConfig` currently holds notebook/timezone/diary settings plus a nested skills block. A fixed `workspace_root/bootstrap` directory would require no new configuration; only configurable limits or root overrides would justify a bootstrap config block.

## Contract implications from repository evidence

- The bootstrap resource response should reuse `SkillResource` directly or introduce a semantically identical generic resource object; duplicating a third read encoding contract would be unnecessary.
- Bootstrap Markdown is instruction text, so V1 can reasonably require UTF-8 even though generic skill resources support base64. Returning base64 for a malformed guide would make it unusable as an instruction document and complicate manifest parsing.
- Generic guide IDs should be validated with the same conservative ASCII identifier grammar used for skill IDs unless the public contract intentionally needs broader names.
- Deterministic manifest order can use explicit priority followed by guide ID; filesystem iteration order must not leak into the API.
- Entrypoint frontmatter cannot follow the lenient skill behavior unchanged if fields such as `id`, `kind`, or schema version are contract-critical. Required bootstrap metadata needs explicit validation after generic YAML parsing.
- Optional malformed guides can be excluded with structured warnings, while a missing or invalid sole entrypoint should fail the bootstrap call because no safe startup instruction remains.


## Repository evidence: public contract and errors

- `openapi/hub.yaml` is a manually maintained strict GPT Actions contract, not generated from protocol Rust types. Room Skills paths, operation IDs, schemas, and `agentId` absence have dedicated regression tests in Hub `main.rs`.
- MCP parameter schemas are separately derived from local `*Args` structs in `mcp_server.rs`; protocol structs are then constructed explicitly. Bootstrap fields must therefore be synchronized across protocol, MCP args, OpenAPI YAML, command routing, and tests.
- Read-only Actions operations use `x-openai-isConsequential: false`; bootstrap get/read should follow this annotation.
- Local Room handlers encode business failures into `{ error: { code, message } }`. `room_value_response` maps known not-found codes to HTTP 404, conflict codes to 409, and other business/validation errors to 400.
- Current generic skills errors collapse some local failures into `validation_error` or operation-level defaults. Bootstrap can define a clearer small taxonomy because missing entrypoint, malformed entrypoint, duplicate guide ID, unknown guide ID, invalid path/symlink, and size limits are observably distinct.

## Recommended V1 contract package for discussion

- Fixed root: `<workspaceRoot>/bootstrap`; fixed entrypoint file `bootstrap.md`; guides discovered recursively or one level under `guides/` only (decision pending).
- `room.bootstrap` takes no payload, returns the UTF-8 entrypoint resource inline plus a deterministic guide manifest, top-level revision, and warnings.
- `room.bootstrap.read` takes stable guide `id`, not a path, and returns the same bounded resource shape used by skills plus parsed guide metadata.
- Entrypoint required metadata: `id`, `kind: entrypoint`, `name`, `description`, `schemaVersion`.
- Guide required metadata: `id`, `kind: guide`, `title`, `summary`; optional `loadPolicy`, `priority`, `loadWhen`, `toolBindings`, and `tags` with safe defaults.
- Unknown frontmatter fields are ignored for typed behavior but may be retained in a raw `frontmatter` object for forward compatibility and diagnostics.
- Missing/invalid entrypoint fails the whole `room.bootstrap` request. Malformed optional guides are excluded with warnings. Duplicate IDs should exclude every colliding guide rather than silently choosing by filesystem order.
- Deterministic order: descending priority, then ascending guide ID.
- Suggested fixed V1 limits: 64 guides, 64 KiB entrypoint, 256 KiB per guide, 4 MiB total scanned Markdown. These are deliberately smaller than install-package limits because bootstrap text is intended for model context.
- Suggested revision: SHA-256 over ordered relative path and raw bytes for `bootstrap.md` plus every discovered guide candidate, including malformed candidates; each accepted resource also exposes its own SHA-256. This lets clients detect any package edit while still identifying unchanged individual resources.

## Contract gaps

- Workspace root: fixed `workspace/bootstrap`, configurable path, or a nested Room configuration value.
- Entrypoint frontmatter: exact required fields and whether `schemaVersion` belongs in frontmatter, response, or both.
- Guide identity and discovery: filename versus frontmatter `id`, duplicate IDs, nested directories, hidden files, and ordering.
- Manifest behavior: whether entrypoint content is inlined, whether guide summaries/load policies/tool bindings are always returned, and how revision/digest is calculated.
- Read behavior: select by guide ID versus path, entrypoint readability, response reuse of the skill resource model, and unknown ID errors.
- Partial failure: invalid guide excluded with warnings versus failing the whole bootstrap call; missing/invalid entrypoint likely needs fail-closed behavior.
- Compatibility: behavior when no bootstrap package exists on upgraded deployments.
- Surface parity: existing skills work synchronized MCP and GPT Actions/OpenAPI; bootstrap likely should follow that convention but repository evidence must confirm whether all Room read APIs do.
- Operational/security: file count, per-file/entrypoint/package limits, UTF-8-only versus base64 support, symlink policy, path containment, and digest stability.

## Options and tradeoffs

- A monolithic `bootstrap.md` simplifies retrieval but grows startup context and weakens discoverability; rejected by confirmed D-02.
- Generic frontmatter-driven guides allow capability additions without code changes; accepted by D-03/D-04.
- Tool bindings may be treated as descriptive strings without validating against currently exposed tools. Validation would create coupling to platform/tool discovery and could incorrectly reject cross-platform guides; descriptive semantics are confirmed by D-05.
- Reading guides by stable ID decouples callers from filesystem layout; path-based reads expose implementation detail but mirror `skills.read`. This remains to be frozen after examining protocol reuse costs.

## Decision rationale

### D-01 — Room-only ownership

The existing architecture already centralizes Room notebook, diary, and skills commands through the active Room connection. A bootstrap API without `agentId` preserves the same identity and avoids exposing workspace location or generic execution as the read mechanism.

### D-02 — Entrypoint plus guides

Every session needs a small deterministic orientation, but detailed Diary, Notebook, execution, tmux, and skills conventions need not occupy startup context. A manifest lets the entrypoint route the agent to relevant guides.

### D-03/D-04 — Frontmatter-driven generic guides

Skills demonstrate that Markdown frontmatter can carry stable metadata while the body remains human- and agent-readable. Generic parsing prevents code branches for each capability family and makes guide addition a workspace content operation.

### D-05 — Descriptive tool bindings

Bindings identify which guide governs a tool or workflow. They must not become authorization, execution capability, or an assertion that a particular MCP client exposed the named tool.

## Risks and unknowns

- A large manifest can itself become startup overhead if `loadWhen` arrays or tool bindings are unbounded.
- Duplicate IDs or malformed frontmatter can make discovery ambiguous; deterministic resolution and warnings are required.
- If entrypoint content is inlined on every call, its size needs a tighter limit than arbitrary guide resources.
- Copying complete MCP schemas into guides would drift; guides should contain selection/workflow rules and only minimal examples.
- A revision based on all file contents is accurate but changes when any guide changes; separate entrypoint and guide digests may be more cache-friendly.

## Issues Encountered

| Issue | Resolution |
|---|---|
| Local path policy rejected direct invocation of the external planning script | Used `bash` as the approved project-root process and passed the `~/.codex` script path as its argument |

## Resources

- `crates/agentic-gpt-protocol/src/lib.rs`
- `crates/agentic-gpt-hub/src/mcp_server.rs`
- `crates/agentic-gpt-hub/src/room.rs`
- `crates/agentic-gpt/src/hub.rs`
- `crates/agentic-gpt/src/skills.rs`
- `crates/agentic-gpt/src/config.rs`

## Decision rationale: refinement round 1

### D-08 — Typed frontmatter with raw retention

The typed subset gives agents a reliable manifest contract and gives the implementation deterministic validation/defaulting. Retaining the raw frontmatter object avoids making every future metadata addition a breaking protocol change. Unknown fields do not affect V1 ordering, loading, or bindings.

### D-09 — Inline entrypoint and ID-based guide reads

The entrypoint is always startup-relevant and therefore belongs inline in `room.bootstrap`. Guide bodies remain on demand. Stable ID selection means directory restructuring cannot break callers, which also makes nested filesystem organization safe to support independently.

### D-10 — Fail-closed entrypoint, partial optional guides

A malformed sole entrypoint leaves no trustworthy session bootstrap and must fail. Optional guides are independent enhancements; excluding only the bad guide avoids turning a typo in one capability convention into total bootstrap outage. Duplicate IDs cannot be resolved safely by incidental traversal order, so every colliding candidate is excluded.

### D-11 — MCP and Actions parity

The repository treats Room package discovery/read as a public contract on both MCP and HTTP/OpenAPI surfaces. Keeping bootstrap parallel avoids platform-dependent capability behavior and preserves the no-`agentId` Room abstraction.

## Guide organization analysis

The confirmed ID-based read contract already separates public identity from disk layout. Nested directories can therefore be supported without exposing or deriving IDs from paths.

Three possible grouping strategies were considered:

1. Derive category from directory path. This is easy to author but turns file moves into semantic changes and leaks storage layout into the manifest.
2. Use only `tags`. This supports many-to-many discovery but does not provide one stable primary section for compact presentation.
3. Add one optional frontmatter `group` as the primary section, while retaining `tags` for cross-cutting classification. This keeps storage and semantic organization independent and is the recommended V1 model.

Recommended behavior for Q-07:

- Recursively discover Markdown under `bootstrap/guides/`.
- Hidden files/directories remain ignored.
- Relative path is returned only as diagnostic/source metadata and is never a read selector or identity source.
- Add optional `group` to guide frontmatter, defaulting to `general` or `null` (exact default pending user choice).
- Keep the top-level `guides` manifest flat; each guide carries its `group` value. Do not add a separate group registry or nested response tree in V1.
- Use `tags` for multiple overlapping concepts; a guide has at most one primary `group`.

## Decision rationale: refinement round 2

### D-12 — Flat V1 guide directory

The expected V1 package contains only a small number of capability guides. `id`, `title`, `summary`, `loadPolicy`, `toolBindings`, and `tags` already provide enough routing and discovery metadata. A primary `group` or path-derived category would add naming, ordering, defaults, and compatibility semantics without solving a current problem.

V1 therefore scans only direct `.md` children of `bootstrap/guides/`. Subdirectories and non-Markdown files are ignored. Because `room.bootstrap.read` is keyed by guide ID rather than path, recursive directories can be added later without changing the public read contract.

### Superseded option — nested directories plus `group`

A recursive storage layout with an optional primary `group` and flat manifest was considered but not adopted for V1. It remains a possible future extension if the guide set grows enough to need authoring-time organization. No V1 field or behavior is reserved for it.

## Q-06 discussion: soft content limits versus hard structural limits

The user accepted the general Q-06A direction but challenged fail-closed handling for an oversized `bootstrap.md`. The proposed revision is to distinguish divisible content limits from structural or trust failures.

Candidate revised behavior:

- Oversized UTF-8 entrypoint or guide content may be returned as a prefix with explicit truncation metadata and warnings rather than invalidating the package or excluding the guide.
- Resource metadata should distinguish full file size from returned content size and state `truncated: true`; SHA-256 and package revision should cover the full underlying file, not only the returned prefix.
- Truncation must occur on a valid UTF-8 character boundary.
- Missing files, malformed required frontmatter, non-UTF-8 content, symlinks, duplicate IDs, and invalid IDs are not safely divisible and should retain fail/exclude behavior rather than being “truncated.”
- Candidate-count and package-scan limits need separate treatment. Deterministically dropping guides can alter package semantics, so these should not be treated exactly like content-prefix truncation.
- `process.exec` may provide a power-user escape hatch for full local reads, but the bootstrap API should expose truncation honestly and should not make correctness depend on another tool being available.

A remaining design concern is that truncating an instruction document can omit important rules near the end. Any truncation contract therefore needs conspicuous machine-readable metadata and warnings; it must never present the prefix as complete content.

## Q-06 discussion: line-aware truncation metadata

The user additionally requires truncation to report where it occurs in line terms, not only byte sizes.

Recommended resource metadata when truncation occurs:

```json
{
  "sizeBytes": 98304,
  "returnedSizeBytes": 65210,
  "truncated": true,
  "totalLines": 1204,
  "returnedThroughLine": 812,
  "omittedFromLine": 813,
  "lastLineComplete": true
}
```

Behavioral recommendation:

- Prefer truncating at the last complete newline that fits within the byte limit, so `omittedFromLine` is normally the next line and the returned Markdown does not end mid-rule or mid-sentence solely because of a byte boundary.
- Preserve the UTF-8 byte limit as the hard response bound; line-aware truncation may return fewer bytes than the configured maximum.
- If no newline exists in the available bounded segment after the required frontmatter (for example, one exceptionally long line), truncate at a valid UTF-8 character boundary, set `lastLineComplete: false`, and set `omittedFromLine` to the same line as `returnedThroughLine`.
- `totalLines` is counted from the complete UTF-8 file while computing its full SHA-256; line numbering is one-based.
- Non-truncated resources return `truncated: false`; line metadata may either be present consistently or omitted when not truncated. For a compact public schema, the recommended contract is to include `totalLines` and `returnedThroughLine` consistently, and include `omittedFromLine` only when truncated.
- Warnings should repeat the path, total/returned bytes, and the first omitted line for human-readable clients, while structured fields remain authoritative.

## Decision rationale: refinement round 5

### D-13 — Graceful, line-aware truncation

Size overflow is a response-shaping concern rather than proof that an otherwise valid instruction document is unusable. V1 therefore returns a bounded prefix and makes incompleteness explicit through both structured metadata and warnings.

Complete-line truncation is preferred because Markdown rules and examples are less likely to be cut mid-construct. A single line longer than the ceiling remains representable through UTF-8-safe partial-line truncation, with `lastLineComplete: false` and the same one-based line number used for `returnedThroughLine` and `omittedFromLine`.

The full file remains the identity source: SHA-256 and package revision cover complete bytes, not only the returned prefix. This keeps cache/change detection accurate. Conversely, malformed metadata, invalid identity, non-UTF-8 documents, symlinks, and containment failures are structural or trust violations and are not repaired by truncation.

The prior 4 MiB aggregate limit is removed because the bootstrap call inlines only the bounded entrypoint and guide metadata; guide bodies are read individually. The implementation must stream or otherwise bound scanning/hashing memory rather than loading the whole package at once.


## Implementation handoff compilation evidence

The handoff inventory found all primary and exhaustive integration surfaces:

- Protocol types and command serde: `crates/agentic-gpt-protocol/src/lib.rs`.
- Local module registration and Room dispatch: `crates/agentic-gpt/src/main.rs`, `crates/agentic-gpt/src/hub.rs`, plus a new recommended `bootstrap.rs`.
- Existing resource/frontmatter/security precedent: `crates/agentic-gpt/src/skills.rs`.
- Hub request-ID and command-type exhaustive matches: `crates/agentic-gpt-hub/src/agents.rs`, `crates/agentic-gpt-hub/src/runs.rs`.
- Active Room forwarding/status mapping: `crates/agentic-gpt-hub/src/room.rs`.
- MCP tools, descriptors, static startup instructions, argument schemas, and tool-list tests: `crates/agentic-gpt-hub/src/mcp_server.rs`.
- HTTP route registration and strict OpenAPI regression tests: `crates/agentic-gpt-hub/src/main.rs`.
- Manually maintained Actions contract: `openapi/hub.yaml`.
- Interface documentation: `docs/interfaces.md`, with optional high-level README updates.
- CI-equivalent verification: `cargo fmt --all -- --check`, `cargo check --workspace`, and `cargo test --workspace` from docs and `.github/workflows/ci.yml`.

### Exact schema compilation rationale

A bootstrap-specific text resource is preferred over altering `SkillResource`: bootstrap requires UTF-8-only semantics and line-aware truncation fields that do not naturally apply to arbitrary binary skill resources. Keeping a distinct type avoids broadening existing skill/OpenAPI contracts while preserving familiar path/content/size/hash fields.

Guide summaries deliberately omit raw frontmatter to bound the startup manifest and keep it task-routing focused. Raw unknown guide fields remain available in `BootstrapReadResponse.frontmatter`, satisfying forward-compatible inspection without duplicating every YAML object across `room.bootstrap`.

`room.bootstrap.read` validates the entrypoint/package first because it is a semantic package read, not an alternate arbitrary workspace file reader. Valid guides beyond the 64-item manifest response remain addressable by stable ID so the response ceiling limits context, not package capability.

### Concurrency and compatibility classification

- State/persistence/migration/retention/cleanup/rollback: N/A; reads derive from workspace files and persist nothing.
- Timing/cancellation/retry: no background work; bounded by existing Hub request timeout; calls are safe to retry.
- Concurrency: no global write lock and no atomic multi-file snapshot. Revision exposes the observed package view; callers may retry around concurrent edits.
- Authentication/authorization: existing Hub action auth/OAuth and active Room role checks apply; bootstrap adds no new permission grant.
- Secrets/network: N/A; no network access or secret storage.
- Configuration: N/A in V1; fixed workspace path and constants.
- Backward compatibility: existing APIs remain unchanged; deployments without a bootstrap package receive the explicit new `bootstrap_not_found` response only when invoking the new tools/routes.

### Remaining risks assigned to implementation verification

- Ensure guide metadata/frontmatter does not accidentally make the manifest unbounded through duplicated raw YAML; the frozen summary/detail split prevents this.
- Ensure every exhaustive `HubCommand` match is updated, including helper functions not on the main dispatch path.
- Ensure HTTP 500 mapping for `bootstrap_read_failed` is added explicitly; current generic Room mapping otherwise defaults many errors to 400.
- Ensure revision input uses unambiguous NUL-delimited records and all valid guides, not only returned guides.
- Ensure line accounting tests cover trailing newline, CRLF, multibyte characters, and one overlong line.
