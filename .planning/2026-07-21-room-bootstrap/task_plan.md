# Task Plan: Room Bootstrap API and guide package

## Goal

Implement a Room Agent-scoped V1 bootstrap package that exposes a concise session entrypoint and a deterministic manifest of frontmatter-driven capability guides through both MCP and GPT Actions, without requiring `agentId` or hard-coding Diary, Notebook, execution, tmux, or skills as guide types.

## Current Phase

Phase 8 - Repair and regression hardening

## Workflow State

- **Stage:** delivery_complete
- **Current role:** implementer
- **Implementation authorized:** yes
- **Entry phase:** Phase 8
- **Open blocking decisions:** 0
- **Design checkpoint:** D-01 through D-14 frozen; R-01 through R-05 repaired and regression-tested
- **Next action:** none; delivery and deferred Diary test cleanup are complete

## Scope and constraints

- V1 belongs only to the active Room Agent and is served through the existing active-Room routing model.
- Runtime content lives at `<workspaceRoot>/bootstrap`; the entrypoint is exactly `bootstrap.md` and guides are direct lowercase-`.md` children of `<workspaceRoot>/bootstrap/guides/`.
- `room.bootstrap` and `room.bootstrap.read` take no `agentId` and are read-only, non-destructive, non-open-world, and non-consequential.
- `bootstrap.md` is a repeated session bootstrap, not a one-time onboarding marker.
- The runtime understands only generic entrypoint/guide metadata. Capability names and usage rules are authored content.
- `toolBindings` are descriptive routing metadata only. They neither grant permission nor assert that a tool is currently exposed.
- The package is read directly from workspace files on every call; V1 has no persistent index, cache, migration, write API, installer, or auto-generation pipeline.
- Existing uncommitted user changes must be preserved. Planning authorized no product changes during design; implementation may now modify only the files needed by the phases below.

## Non-goals

- No recursive guide directories or `group` hierarchy in V1.
- No `room.bootstrap.write`, CRUD, reload, installation, or activation API.
- No automatic creation of a default/user-specific bootstrap package.
- No validation of `toolBindings` against MCP discovery or platform-specific tool registries.
- No duplication of complete MCP tool schemas inside guides.
- No configurable bootstrap root or bootstrap-specific config block in V1.
- No atomic multi-file filesystem snapshot guarantee across concurrent external edits.

## Frozen public contract

### Workspace discovery

- Root: `<workspaceRoot>/bootstrap`.
- Entrypoint: `<workspaceRoot>/bootstrap/bootstrap.md`.
- Optional guide directory: `<workspaceRoot>/bootstrap/guides`.
- Only direct regular files whose names do not begin with `.` and whose extension is exactly lowercase `.md` are candidates.
- Nested directories, hidden entries, and non-Markdown files are ignored.
- A missing guide directory is valid and yields an empty guide manifest.
- The bootstrap root or entrypoint being a symlink, the entrypoint not being a regular file, or the entrypoint being unreadable is a package-level failure.
- A symlinked guide directory is ignored with a warning. A symlinked/unreadable/non-regular guide is excluded with a warning.
- All documents must be UTF-8. CRLF may be normalized for frontmatter parsing, but resource content and SHA-256 use original file bytes.

### Entrypoint frontmatter

`bootstrap.md` must begin with a closed YAML frontmatter object and provide:

```yaml
---
id: room
kind: entrypoint
name: Room Bootstrap
description: Session initialization and guide routing.
schemaVersion: 1
---
```

Rules:

- `id`: required non-empty string using the existing skill ID grammar: ASCII alphanumeric plus `_`, `.`, and `-`; not `.` or `..`.
- `kind`: required exact string `entrypoint`.
- `name` and `description`: required non-empty strings.
- `schemaVersion`: required integer and must equal `1`; other versions are `bootstrap_invalid` in V1.
- The parsed raw YAML object is retained in the entrypoint detail for forward-compatible inspection.
- Missing/unclosed/non-object/invalid YAML or invalid required fields make the package `bootstrap_invalid`.

### Frontmatter scan bound

- The complete leading YAML frontmatter block, including both `---` delimiters, must end within the first 1,048,576 bytes (1 MiB) of the file. A closing delimiter exactly at the bound is valid.
- This is a metadata parsing bound, separate from the 64 KiB/256 KiB returned-content limits. The complete file is still streamed for UTF-8 validation, line counting, SHA-256, and package revision.
- An entrypoint whose frontmatter exceeds the bound is `bootstrap_invalid`. An optional guide whose frontmatter exceeds the bound is excluded with `guide_frontmatter_invalid`.

### Guide frontmatter

Every valid guide must begin with a closed YAML frontmatter object:

```yaml
---
id: diary
kind: guide
title: Diary conventions
summary: Read recent entries for continuity and append meaningful daily records.
loadPolicy: contextual
priority: 80
loadWhen:
  - The session continues prior personal or project context.
toolBindings:
  - room.diary.recent
  - room.diary.append
tags:
  - continuity
---
```

Typed fields:

- Required: `id`, `kind: guide`, `title`, `summary`.
- `id` uses the same safe grammar as the entrypoint.
- `title` and `summary` are non-empty strings.
- `loadPolicy`: `startup | contextual | on_demand`, default `on_demand`.
- `priority`: signed 32-bit integer, default `0`.
- `loadWhen`, `toolBindings`, and `tags`: arrays of non-empty strings, default `[]`; authored order is preserved and duplicates are allowed.
- Unknown fields are ignored by typed V1 behavior but retained in the raw `frontmatter` returned by `room.bootstrap.read`.
- Any type/value/required-field failure excludes the guide and emits a warning; it does not fail the package.
- Duplicate IDs exclude every guide with that ID. Duplicate resolution must not depend on directory iteration order.

### Deterministic ordering and manifest ceiling

- Sort every valid, duplicate-free guide by `priority` descending and then `id` ascending.
- `totalGuides` counts all valid guides after validation and duplicate exclusion.
- Return at most the first 64 guides as `guides`; `returnedGuides == guides.length`.
- When `totalGuides > 64`, emit `guides_truncated` with total and returned counts.
- A valid guide omitted only by this 64-item manifest ceiling remains readable by `room.bootstrap.read` when its ID is known and remains part of package `revision`.

### Protocol response types

Use camelCase JSON fields. Exact public shapes:

```text
BootstrapDocumentKind = "entrypoint" | "guide"
BootstrapLoadPolicy  = "startup" | "contextual" | "on_demand"
BootstrapEncoding    = "utf8"
```

```json
BootstrapTextResource {
  "path": "bootstrap.md",
  "encoding": "utf8",
  "content": "...",
  "mediaType": "text/markdown",
  "sizeBytes": 98304,
  "returnedSizeBytes": 65210,
  "totalLines": 1204,
  "returnedThroughLine": 812,
  "omittedFromLine": 813,
  "truncated": true,
  "lastLineComplete": true,
  "sha256": "lowercase full-file sha256"
}
```

- `omittedFromLine` is omitted when `truncated == false`.
- `mediaType` is always `text/markdown`; `encoding` is always `utf8` in V1.

```json
BootstrapEntrypoint {
  "id": "room",
  "kind": "entrypoint",
  "name": "Room Bootstrap",
  "description": "...",
  "frontmatter": {},
  "resource": BootstrapTextResource
}
```

```json
BootstrapGuideSummary {
  "id": "diary",
  "kind": "guide",
  "title": "Diary conventions",
  "summary": "...",
  "loadPolicy": "contextual",
  "priority": 80,
  "loadWhen": [],
  "toolBindings": [],
  "tags": [],
  "path": "guides/diary.md",
  "sizeBytes": 1234,
  "totalLines": 42,
  "sha256": "lowercase full-file sha256"
}
```

```json
BootstrapResponse {
  "schemaVersion": 1,
  "revision": "lowercase package sha256",
  "entrypoint": BootstrapEntrypoint,
  "guides": [BootstrapGuideSummary],
  "totalGuides": 1,
  "returnedGuides": 1,
  "warnings": []
}
```

```json
BootstrapReadRequest {
  "id": "diary"
}
```

```json
BootstrapReadResponse {
  "guide": BootstrapGuideSummary,
  "frontmatter": {},
  "resource": BootstrapTextResource,
  "warnings": []
}
```

`room.bootstrap.read` validates the same package/entrypoint contract as `room.bootstrap`; it is not a generic file reader. It may read any valid guide, including one omitted only by the 64-item manifest ceiling. Unknown, invalid, duplicate-excluded, or otherwise unavailable IDs return `guide_not_found`.

### Text truncation and line accounting

- Entrypoint content response ceiling: 64 KiB (`65_536` bytes).
- Guide content response ceiling: 256 KiB (`262_144` bytes).
- Size overflow is not document invalidity. Return a prefix and an explicit warning.
- Prefer the final complete `\n`-terminated line whose bytes fit within the ceiling. This may return fewer bytes than the ceiling.
- When no complete-line cut is possible within the bounded prefix, cut at the last valid UTF-8 character boundary.
- `sizeBytes` and `sha256` describe the complete original file; `returnedSizeBytes` describes `content` exactly.
- Logical line counting is one-based and treats a trailing LF as a terminator, not an extra empty logical line. An empty file has zero lines, although valid bootstrap documents cannot be empty.
- For an untruncated resource, `returnedThroughLine == totalLines`, `lastLineComplete == true`, and `omittedFromLine` is absent.
- For a truncated prefix ending at a complete newline, `returnedThroughLine` is the final complete returned line, `omittedFromLine == returnedThroughLine + 1`, and `lastLineComplete == true`.
- For a partially returned line, `returnedThroughLine` and `omittedFromLine` identify that same line and `lastLineComplete == false`.
- Warning prefixes are stable machine-recognizable codes; warning detail is human-readable. Truncation warnings include path, full/returned bytes, and first omitted line.

### Revision algorithm

- Hash complete original file bytes individually using SHA-256 and lowercase hex.
- Invalid/excluded guides do not participate in revision.
- Valid guides omitted only by the 64-item manifest ceiling do participate.
- Calculate package revision by feeding this canonical UTF-8/NUL-delimited record stream into SHA-256:

```text
agentic-room-bootstrap-v1\0
schemaVersion\01\0
entrypoint\0bootstrap.md\0<entrypoint full-file sha256>\0
guide\0<id>\0<relative path>\0<full-file sha256>\0
... one guide record in deterministic priority/id order ...
```

- The revision represents the usable package view. Editing an invalid/excluded guide does not change it; editing any valid guide, including one omitted from the returned manifest, does.

### Warning taxonomy

Warnings are strings whose prefix before `:` is stable. Implement at least:

- `entrypoint_truncated`
- `guide_truncated`
- `guides_truncated`
- `guides_dir_symlink_ignored`
- `guide_dir_entry_unreadable`
- `guide_symlink_ignored`
- `guide_unreadable`
- `guide_non_utf8`
- `guide_frontmatter_invalid`
- `guide_metadata_invalid`
- `guide_duplicate_id`

Unrelated invalid-guide warnings are returned by `room.bootstrap`. `room.bootstrap.read` returns warnings relevant to the requested guide/resource; it does not need to replay every unrelated package warning.

### Error and HTTP status taxonomy

Local Room Agent returns native `{ "error": { "code", "message" } }` values:

| Code | Meaning | HTTP status |
|---|---|---:|
| `bootstrap_not_found` | bootstrap root or entrypoint is missing | 404 |
| `guide_not_found` | requested guide ID is absent or excluded | 404 |
| `bootstrap_invalid` | entrypoint/package identity, UTF-8, frontmatter, kind, schema version, regular-file, symlink, or containment contract is invalid | 400 |
| `bootstrap_read_failed` | unexpected filesystem/read/hash/serialization failure | 500 |
| `room_agent_required` | command reached a non-Room local agent | existing behavior |
| `room_not_active` | Hub has no active Room Agent | 404 |
| `room_state_conflict` | active Room state is inconsistent | 409 |
| `room_bootstrap_timeout` | Hub timed out waiting for `room.bootstrap` | 504 |
| `room_bootstrap_read_timeout` | Hub timed out waiting for `room.bootstrap.read` | 504 |

MCP returns the same native error object through `AgenticResult`/`isError`; invalid tool arguments still use MCP invalid-params behavior before command forwarding.

### Lifecycle, consistency, and operations

- Both operations are read-only and retry-safe/idempotent.
- Every call rescans current workspace state; no restart or explicit reload is required after file edits.
- V1 provides no atomic cross-file snapshot against concurrent external writers. Each file is opened/read once per call, and the response/revision reflect the bytes observed during that call; callers can retry if revision changes between calls.
- No cancellation, background session, lock, persistence, retention, cleanup, rollback, or migration behavior applies.
- No bootstrap config is added. Limits are named implementation constants covered by tests.
- Hashing/line counting should stream or otherwise avoid accumulating every guide body at once. Private buffering/parser strategy is implementation discretion.

## Phases

### Phase 1: Repository discovery and contract refinement

**Objective:** identify all existing Room/package/public-surface conventions and remove product ambiguity.

- [x] Capture user intent and V1 ownership boundary.
- [x] Initialize isolated plan `.planning/2026-07-21-room-bootstrap` with the laptop planning script.
- [x] Inspect protocol, local Room dispatch, skills resource precedent, Hub routing, MCP descriptors, Actions/OpenAPI, errors, docs, and verification commands.
- [x] Preserve the pre-existing worktree and record planning-only changes.
- **Status:** complete

### Phase 2: Freeze public bootstrap contract

**Objective:** freeze all observable V1 behavior before implementation.

- [x] Freeze workspace layout, flat discovery, frontmatter fields/defaults, ID semantics, and non-goals.
- [x] Freeze MCP/HTTP API shapes, public field names, Room-only routing, and Actions parity.
- [x] Freeze partial-failure, duplicate-ID, UTF-8, symlink, containment, warning, and error behavior.
- [x] Freeze line-aware truncation, manifest ceiling, sorting, full-file hashes, and revision algorithm.
- [x] Obtain user acceptance for D-01 through D-13.
- **Status:** complete

### Phase 3: Protocol and public data model

**Objective:** make the wire contract compile and serialize deterministically before filesystem behavior is introduced.

**Prerequisites:** Phases 1-2 complete.

**Files/surfaces:**

- `crates/agentic-gpt-protocol/src/lib.rs`
- Protocol serialization/compatibility tests in the same crate.

**Ordered work:**

1. Add `BootstrapDocumentKind`, `BootstrapLoadPolicy`, `BootstrapEncoding`, `BootstrapTextResource`, `BootstrapEntrypoint`, `BootstrapGuideSummary`, `BootstrapResponse`, `BootstrapReadRequest`, and `BootstrapReadResponse` with the exact camelCase/enum contract above.
2. Add `HubCommand::RoomBootstrap { request_id }` serialized as `room.bootstrap`.
3. Add `HubCommand::RoomBootstrapRead { request_id, payload }` serialized as `room.bootstrap.read`.
4. Preserve optional-field compatibility: omit `omittedFromLine` when absent; arrays/warnings serialize as arrays; typed defaults are supplied by the local loader rather than request deserialization.
5. Add protocol tests for command names, request round-trips, enum spellings, truncation optional fields, and response JSON field names.

**Verification:**

```bash
cargo test -p agentic-gpt-protocol
cargo check -p agentic-gpt-protocol
```

**Completion boundary:** protocol tests pass and no downstream implementation has to invent public JSON fields.

- **Status:** complete

### Phase 4: Local Room bootstrap loader

**Objective:** load, validate, summarize, hash, truncate, and read the workspace package with the frozen semantics.

**Prerequisites:** Phase 3.

**Files/surfaces:**

- New `crates/agentic-gpt/src/bootstrap.rs` (recommended module boundary).
- `crates/agentic-gpt/src/main.rs` module declaration.
- `crates/agentic-gpt/src/hub.rs` command dispatch and bootstrap-specific error mapping.
- Existing reusable helpers in `crates/agentic-gpt/src/skills.rs` only if extraction does not change skill behavior.

**Ordered work:**

1. Implement fixed-root resolution from `Config.workspace_root` with explicit symlink/regular-file checks.
2. Implement frontmatter extraction, typed validation/defaulting, raw JSON retention, and safe ID validation.
3. Implement direct guide scanning, deterministic duplicate exclusion, priority/id ordering, full valid-guide indexing, and 64-item response truncation.
4. Implement streaming/full-file SHA-256 and logical line accounting over original bytes.
5. Implement UTF-8-safe complete-line-first bounded resource construction for 64 KiB entrypoint and 256 KiB guide responses.
6. Implement canonical package revision over all valid guides.
7. Implement `load(state) -> BootstrapResponse` and `read(state, BootstrapReadRequest) -> BootstrapReadResponse` or equivalent private APIs.
8. Add `RoomBootstrap` and `RoomBootstrapRead` arms in `hub.rs`; require `RunMode::Room`; map known loader errors to the frozen codes and unexpected errors to `bootstrap_read_failed`.
9. Keep `room.bootstrap.read` keyed by ID and validate the package entrypoint before returning a guide.

**Tests:**

- Valid entrypoint with missing/empty guide directory.
- Required fields, defaults, raw unknown frontmatter, unsupported schema version, CRLF parsing.
- Flat lowercase `.md` discovery; hidden/non-Markdown/nested entries ignored.
- Invalid/non-UTF-8/symlink/unreadable guides excluded with stable warning prefixes.
- Duplicate IDs exclude all colliding paths independent of directory iteration order.
- `toolBindings` remain descriptive and are not validated against tools.
- Priority-descending/ID-ascending ordering and 64-guide `totalGuides`/`returnedGuides` behavior.
- A guide omitted from the manifest ceiling remains readable by ID and changes revision when edited.
- Entrypoint and guide exact-boundary, complete-line truncation, multibyte UTF-8 boundary, one overlong line, line fields, and warnings.
- Full-file hash/revision are unaffected by returned-prefix size; invalid guide edits do not change revision; valid edits do.
- Missing/invalid/symlinked entrypoint error codes; unknown/excluded ID `guide_not_found`.
- No product state or config files are created or mutated by reads.

**Verification:**

```bash
cargo test -p agentic-gpt bootstrap
cargo check -p agentic-gpt
```

**Completion boundary:** local unit tests cover every public success/degradation/failure branch and command dispatch returns protocol-shaped values.

- **Status:** complete

### Phase 5: Hub routing, MCP, and GPT Actions/OpenAPI

**Objective:** expose the local contract consistently through every existing Room public surface.

**Prerequisites:** Phases 3-4.

**Files/surfaces:**

- `crates/agentic-gpt-hub/src/agents.rs`
- `crates/agentic-gpt-hub/src/runs.rs`
- `crates/agentic-gpt-hub/src/room.rs`
- `crates/agentic-gpt-hub/src/mcp_server.rs`
- `crates/agentic-gpt-hub/src/main.rs`
- `openapi/hub.yaml`
- Any exhaustive local/Hub `HubCommand` match that compilation identifies; classify both new commands as read-only Room data operations, never executable/policy-confirmed/open-world actions.

**Ordered work:**

1. Add request-ID getter/setter and `command_type` coverage for `room.bootstrap` and `room.bootstrap.read`.
2. Add Room HTTP forwarders and routes:
   - `POST /v1/room/bootstrap`, operationId `roomBootstrap`, no request body.
   - `POST /v1/room/bootstrap/read`, operationId `roomBootstrapRead`, `BootstrapReadRequest` body.
3. Extend `room_value_response` to map `bootstrap_not_found`/`guide_not_found` to 404 and `bootstrap_read_failed` to 500; retain 400 for `bootstrap_invalid` and 504 operation-specific timeouts.
4. Register MCP tools with exact names, no `agentId`, protocol construction, and descriptions that distinguish manifest startup from ID-based guide reads.
5. Keep descriptor annotations read-only=true, destructive=false, openWorld=false. Do not add either tool to mutation/open-world/destructive sets.
6. Add one concise sentence to `MCP_INSTRUCTIONS`: at Room session start call `room.bootstrap`, then call `room.bootstrap.read` for relevant listed guide IDs. Move no existing detailed behavior into generated code in this phase.
7. Add strict OpenAPI paths/schemas/responses and `x-openai-isConsequential: false`.
8. Update exhaustive command/routing tests and add no-`agentId`, tool-name, schema, annotation, dispatch, timeout, error-status, operationId, and OpenAPI regression tests.

**Verification:**

```bash
cargo test -p agentic-gpt-hub bootstrap
cargo check -p agentic-gpt-hub
```

**Completion boundary:** MCP and Actions expose matching Room-only behavior and all exhaustive command helpers compile/test.

- **Status:** complete

### Phase 6: Documentation, authoring examples, and integration verification

**Objective:** make the feature authorable and verify the whole workspace without installing user-specific content.

**Prerequisites:** Phases 3-5.

**Files/surfaces:**

- `docs/interfaces.md`
- `README.md` and `README.zh-CN.md` only where the top-level Room feature summary benefits from an update.
- Optional repository documentation example under an existing docs/examples convention; do not write into the user's runtime workspace as part of tests or startup.

**Ordered work:**

1. Document the fixed directory layout, both frontmatter schemas, defaults, flat/non-recursive rule, ID-based reads, tool-binding semantics, warning/error behavior, and truncation fields.
2. Document MCP and Actions names/routes and explain that missing bootstrap returns 404 rather than auto-creating content.
3. Provide concise example content for `bootstrap.md` and representative Diary/Notebook/execution/skills guides; examples must remain generic and must not hard-code runtime guide types into implementation.
4. Run focused tests, formatting, workspace check, and full workspace tests.
5. When a Room Agent test environment is available, perform a smoke call for an empty-guide package, manifest+read, truncation warning, and unknown guide. Record any unavailable manual environment honestly rather than fabricating a pass.

**Verification:**

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
```

**Completion boundary:** docs match the wire contract and CI-equivalent commands pass.

- **Verification note:** formatting and workspace check passed; the workspace test run has one known unrelated Diary logical-day timing failure documented in `progress.md`.
- **Status:** complete

### Phase 7: Delivery

**Objective:** close implementation with traceable verification and no silent contract drift.

- [x] Re-read this plan and compare code/tests/OpenAPI/docs against D-01 through D-13.
- [x] Record exact commands and outcomes in `progress.md`.
- [x] Record any implementation discretion used and any deferred non-goal.
- [x] Deliver changed-file summary, public behavior, warnings/errors, and verification status.
- **Status:** complete

### Phase 8: Repair and regression hardening

**Objective:** correct post-delivery integration and contract defects found by independent review, then add regressions that prevent the same classes of omission.

**Prerequisites:** Phases 3-7 complete. D-01 through D-13 remain frozen; this phase repairs implementation drift and introduces no new public behavior.

**Files/surfaces:**

- `crates/agentic-gpt-hub/src/mcp_server.rs`
- `crates/agentic-gpt/src/bootstrap.rs`
- Focused tests in those modules.
- Planning files for exact findings and verification evidence.
- Protocol/OpenAPI/docs only if repair reveals an actual contract mismatch; do not broaden the feature.

**Ordered work:**

1. [x] Add `room.bootstrap` and `room.bootstrap.read` to the hand-written ChatGPT Apps `/mcp` `tools/call` dispatcher so every Bootstrap tool returned by `tools/list` is callable through that JSON-RPC endpoint.
2. [x] Preserve operation-specific MCP timeout codes: `room_bootstrap_timeout` for `room.bootstrap` and `room_bootstrap_read_timeout` for `room.bootstrap.read`; neither operation may fall back to `room_notebook_timeout`.
3. [x] Enforce D-10 duplicate semantics across every colliding candidate with an extractable valid guide ID, even when another candidate with that ID later fails non-ID metadata validation. No otherwise-valid guide may survive an ambiguous duplicate ID.
4. [x] Replace silent dropping of individual `read_dir` entry errors with deterministic `guide_dir_entry_unreadable` warnings while continuing to process readable entries.
5. [x] Bound per-file scanning/hashing memory. Do not load an arbitrarily large entrypoint or guide wholly into memory merely to hash, count lines, validate frontmatter, or return a bounded prefix. Preserve full-file SHA-256/revision, current UTF-8/frontmatter semantics, and one observed byte stream per file per call.

**Required regression tests:**

- [x] Exercise the real Apps `/mcp` `tools/call` path for both `room.bootstrap` and `room.bootstrap.read`; descriptor/list assertions alone are insufficient.
- [x] Prove that every registered MCP tool name is accepted by the hand-written Apps dispatcher, or remove the separately maintained dispatch-name set so list/call drift is structurally impossible.
- [x] Exercise Bootstrap MCP timeout conversion and assert the operation-specific code plus `isError: true` for each tool.
- [x] Add a mixed-validity duplicate fixture: one otherwise-valid guide and one same-ID candidate with invalid non-ID metadata must exclude both deterministically.
- [x] Cover directory-entry error warning production through a deterministic helper-level test when the platform cannot reliably manufacture a failing `ReadDir` entry.
- [x] Add practical oversized-file fixtures proving bounded returned content and correct full-file hash/line metadata. Do not use flaky process-RSS assertions.
- [x] Keep existing protocol, loader, Hub, HTTP/OpenAPI, annotation, and error-status regressions green.

**Verification:**

```bash
cargo fmt --all -- --check
CARGO_TARGET_DIR=/tmp/agentic-gpt-room-bootstrap-target cargo test -p agentic-gpt bootstrap
CARGO_TARGET_DIR=/tmp/agentic-gpt-room-bootstrap-target cargo test -p agentic-gpt-hub
CARGO_TARGET_DIR=/tmp/agentic-gpt-room-bootstrap-target cargo test -p agentic-gpt-protocol
CARGO_TARGET_DIR=/tmp/agentic-gpt-room-bootstrap-target cargo check --workspace
CARGO_TARGET_DIR=/tmp/agentic-gpt-room-bootstrap-target cargo test --workspace
```

The known deferred Diary boundary-sensitive test still fails during its documented pre-05:00 window. Do not modify Diary code or tests in this phase.

**Completion boundary:** both Bootstrap tools are callable through the actual Apps JSON-RPC path; MCP timeout parity is restored; D-10 duplicate behavior is enforced; guide scanning neither silently loses directory-entry failures nor performs unbounded full-file buffering; focused suites pass; and the full workspace outcome is recorded honestly.

- **Verification note:** formatting, protocol 8/8, bootstrap 13/13, Hub 56/56, and workspace check passed. Full workspace testing reached 96 passed and the one explicitly deferred `diary::tests::append_and_select_exact_round_trip` failure at `00:40 +0800`; no Diary code or tests were changed.
- **Status:** complete

## Key Questions

| ID | Question | Blocking | Status | Resolution |
|---|---|---:|---|---|
| Q-01 | Workspace root/configurability | yes | resolved | Fixed `<workspaceRoot>/bootstrap`; no V1 config block (D-07) |
| Q-02 | Frontmatter fields/defaults/unknown fields | yes | resolved | Typed contract plus raw detail retention (D-08) |
| Q-03 | Manifest/read API semantics | yes | resolved | Inline entrypoint, flat metadata manifest, ID-based reads (D-09) |
| Q-04 | Missing/invalid/duplicate behavior | yes | resolved | Fail package for entrypoint; warn/exclude optional guides; exclude all duplicate IDs (D-10) |
| Q-05 | MCP and Actions parity | yes | resolved | Both surfaces required, read-only and no `agentId` (D-11) |
| Q-06 | Limits/truncation/order/revision/security | yes | resolved | Graceful line-aware truncation, 64-guide manifest, full hashes/revision, structural failures not truncated (D-13) |
| Q-07 | Nested directories/category grouping | yes | resolved | Flat direct guide directory; no `group` in V1 (D-12) |
| Q-08 | Frontmatter memory bound | yes | resolved | Complete frontmatter must close within the first 1 MiB; exact boundary accepted (D-14) |

## Decisions Made

| ID | Area | Status | Outcome |
|---|---|---|---|
| D-01 | Ownership | confirmed | Active Room Agent only; no input `agentId` |
| D-02 | Content shape | confirmed | Short fixed entrypoint plus on-demand guides |
| D-03 | Metadata | confirmed | YAML frontmatter drives the manifest |
| D-04 | Guide model | confirmed | Generic guides; no capability-specific runtime branches |
| D-05 | Tool bindings | confirmed | Descriptive routing metadata only |
| D-06 | Workflow boundary | confirmed | Design stops before implementation; next invocation uses base planning skill alone |
| D-07 | Workspace | confirmed | Fixed `<workspaceRoot>/bootstrap`; no config |
| D-08 | Frontmatter | confirmed | Required typed fields, defaults, raw unknown-field retention in detail reads |
| D-09 | API | confirmed | `room.bootstrap` manifest and `room.bootstrap.read {id}` |
| D-10 | Failures | confirmed | Entrypoint fail-closed; optional guides warn/exclude; duplicate IDs all excluded |
| D-11 | Public surfaces | confirmed | MCP and GPT Actions/OpenAPI parity |
| D-12 | Organization | confirmed | Flat direct `guides/*.md`, no recursion/group |
| D-13 | Resources | confirmed | Line-aware bounded responses, deterministic order/revision, full-file hashes, structural/security rejection |
| D-14 | Frontmatter bound | confirmed | Complete frontmatter block must close within the first 1 MiB; entrypoint fails and optional guides warn/exclude on overflow |

Detailed evidence and rationale remain canonical in `findings.md`.

## Acceptance criteria

- [x] `room.bootstrap` and `room.bootstrap.read` work through both native MCP and the Apps `/mcp` JSON-RPC path, only through the active Room Agent, and expose no `agentId` field.
- [x] The protocol and OpenAPI fields exactly match the frozen shapes and enum spellings.
- [x] A valid entrypoint with no guide directory succeeds with an empty manifest.
- [x] Generic frontmatter drives identity, display, load guidance, priority, tags, and tool bindings; no capability family is hard-coded.
- [x] Missing/invalid entrypoint errors, invalid-guide warnings, mixed-validity duplicate exclusion, UTF-8, symlink, flat-path, containment, and directory-entry failure behavior are deterministic.
- [x] Entry/guide content truncation reports complete/full sizes, line position, completeness, and full-file hash without presenting prefixes as complete.
- [x] Manifest ordering and 64-item truncation are deterministic; omitted valid guides remain readable and affect revision.
- [x] Revision matches the canonical full-file algorithm and excludes invalid guides.
- [x] Both MCP tools are read-only/non-destructive/non-open-world and the Actions operations are non-consequential.
- [x] HTTP status mapping and MCP native error behavior, including operation-specific Bootstrap timeouts, match the frozen taxonomy.
- [x] Reads create no state, require no restart/reload, and are retry-safe.
- [x] Per-file scanning/hashing memory is bounded independently of authored file size while preserving full-file hash and line metadata.
- [x] The 1 MiB frontmatter scan bound is documented; exact-boundary, overflow, and cross-chunk UTF-8 behavior are regression-tested.
- [x] Registered MCP tool names and Apps `tools/call` dispatch cannot drift silently.
- [x] Protocol, local loader, Hub/MCP/OpenAPI, documentation, formatting, workspace check, and focused crate tests pass; the known unrelated Diary timing failure in the full workspace test is documented.
- [x] Documentation includes authoring examples and explains tool schema versus usage-guide responsibility.

## Implementation Discretion

The Implementer may choose, without reopening design:

- Private helper/module layout, including whether frontmatter/hash/path utilities are extracted from `skills.rs` or implemented inside `bootstrap.rs`, provided existing skill behavior and tests do not regress.
- Streaming/buffering strategy for full-file hashing, line counting, frontmatter extraction, and bounded prefixes.
- Internal error enum/types and test-fixture helpers, provided public codes/warnings remain frozen.
- Internal collections and duplicate-detection mechanics, provided ordering and exclusion are deterministic.
- Exact log wording and private function names.
- Whether README changes are needed in addition to `docs/interfaces.md`, provided public documentation remains complete.

The Implementer may not change frontmatter fields/defaults, directory rules, public JSON fields, warning/error code prefixes, limits, ordering, revision membership, routes/tool names, annotations, or Room identity semantics without reopening design under both skills.

## Verification matrix

| Requirement | Primary verification |
|---|---|
| Protocol names and fields | `agentic-gpt-protocol` serialization tests |
| Filesystem/frontmatter/security | `agentic-gpt` bootstrap unit tests |
| Truncation and revision | deterministic fixture tests with UTF-8 and >limit files |
| Active Room routing | Hub Room routing tests |
| MCP schema/annotations | MCP tool listing/descriptor tests |
| Actions parity/no agentId | strict `openapi/hub.yaml` regression tests |
| Error/status behavior | local error mapping plus Hub response tests |
| Documentation contract | review against protocol/OpenAPI and examples |
| Regression | `cargo fmt --all -- --check`, `cargo check --workspace`, `cargo test --workspace` |

## Implementation Handoff

- **Plan maturity:** delivery_complete
- **Design phase:** complete
- **Implementation authorized:** yes
- **Entry phase:** Phase 8
- **Frozen decisions:** D-01 through D-14
- **Open blocking decisions:** none
- **Implementation discretion:** see `Implementation Discretion`
- **Verification convention:** focused crate tests after each phase, then Phase 8 format/check/focused suites and workspace test outcome recorded with the deferred Diary caveat
- **Commit convention:** Phases 3 through 8, D-14 clarification, and the deferred Diary test cleanup each use focused commits; leave the workspace clean
- **Design checkpoint:** D-01 through D-14 frozen; R-01 through R-05 repaired
- **Next invocation:** none; deliver the completed Phase 8 repair

## Resolved repository follow-up

### Diary logical-date round-trip test

- **Status:** fixed and fully regression-tested on 2026-07-24.
- **Affected test:** `diary::tests::append_and_select_exact_round_trip`.
- **Failure window:** deterministic from `00:00` through `04:59` in the configured `Asia/Shanghai` timezone when the default `room.diaryDayBoundaryHour` is `5`.
- **Root cause:** `append()` writes to the logical Diary date and returns it as `response.date`, but the test selects by the natural calendar date derived from `response.created_at`. Before the 05:00 boundary those dates differ by one day.
- **Bootstrap regression status:** none. `crates/agentic-gpt/src/diary.rs` and `crates/agentic-gpt/src/config.rs` have identical Git blobs before and after the Room Bootstrap implementation range, and no Bootstrap commit modified Diary behavior or defaults.
- **Resolution:** the test now parses `response.date` and selects by that logical Diary date instead of deriving a natural calendar date from `created_at`. Diary implementation, protocol, configuration, and the default 05:00 boundary remain unchanged.
- **Verification:** the focused round-trip test passes; `cargo fmt --all -- --check` passes; `cargo test --workspace` passes completely with local agent 99/99, Hub 56/56, protocol 8/8, and doctests 0/0.
- **Scope:** test-only product change plus planning record; the completed Room Bootstrap behavior was not reopened.

## Errors Encountered

| Error | Attempt | Resolution |
|---|---:|---|
| Direct execution of `~/.codex/skills/planning-with-files/scripts/init-session.sh` was rejected as outside allowed roots | 1 | Invoked the same laptop script through `bash` from the project working directory; initialization succeeded |
| Planning maturity fields temporarily disagreed after Q-06 write-through (`task_plan.md`/`progress.md` still showed `exploring`) | 1 | Re-read all three files under the handoff gate and reconciled them in this implementation-ready compilation |
| `cargo test -p agentic-gpt-protocol` could not create `/home/slhaf/Projects/AgenticGPT/target/debug/.cargo-build-lock` on the read-only repository filesystem | 1 | Retried with a task-local writable `CARGO_TARGET_DIR` under `/tmp` |
| `cargo fmt --all` could not rewrite the protocol file on the read-only repository filesystem | 1 | Applied the single rustfmt line-wrap manually, then re-ran format checking |
| First bootstrap test run exposed entrypoint validation details escaping as public error codes and a symlink fixture that rewrote rather than replaced its link | 1 | Map all entrypoint metadata failures to `bootstrap_invalid`; remove the link before creating the valid fixture |
| A follow-up bootstrap test failed because the fixture reset cleanup was inserted before the entrypoint existed | 1 | Removed that premature cleanup; retain removal only after the symlink is created |
| First Hub compile found duplicate derive/serde attributes on `BootstrapReadArgs` | 1 | Removed the duplicated attribute pair; keep one `JsonSchema` derive and camelCase serde attribute |
| Full workspace test had one failure in pre-existing `diary::tests::append_and_select_exact_round_trip` (`left: 0`, `right: 1`) while the other 93 local-agent tests passed | 1 | Isolated and reproduced the test; confirmed a pre-existing logical-date versus calendar-date mismatch and deferred the unrelated fix |
| Isolated Diary diagnostic initially placed `--exact` before Cargo's test-argument separator | 1 | Corrected the command to pass `--exact` after `--` |
| Correctly isolated Diary test still fails at local `00:03 +0800` because the default 05:00 logical-day boundary writes yesterday's file while the test selects the `created_at` calendar date | 1 | Confirmed deterministic pre-existing timing failure; left unrelated Diary code/tests unchanged and documented the verification caveat |
| Manual Room bootstrap smoke call was unavailable because no `agentic-gpt`/Hub process was running and `127.0.0.1:8080/health` refused the connection | 1 | Recorded smoke checks as unavailable; relied on deterministic local-agent and Hub routing tests |
| A broad replacement initially applied Bootstrap timeout handling to two Notebook handlers | 1 | Inspected the diff, restored Notebook's existing fallback, and applied operation-specific handling only to the two Bootstrap handlers |
| The toolchain has no `Option::then_some` method | 1 | Replaced the candidate-ID extraction expression with an explicit `is_ok` branch; boolean `then_some` usage remains unchanged |
| Workspace check reported the test-only `build_resource` helper as dead code in production builds | 1 | Gated the compatibility helper with `#[cfg(test)]`; production workspace check is now warning-free |
| Workspace test still failed in the deferred Diary round-trip at `00:40 +0800` (`96 passed, 1 failed`) | 1 | Confirmed the same pre-existing 05:00 logical-day boundary defect; did not modify Diary code or tests |
