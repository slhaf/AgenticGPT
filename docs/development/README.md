# Development Guide

This guide is for maintainers and coding agents implementing features and modules. It explains how to locate the relevant code, work within existing boundaries, and verify observable behavior. For installation and usage, start with the [project README](../../README.md). See also the [Chinese development guide](README.zh-CN.md). Release-specific steps are in the [release guide](releasing.md); this page does not duplicate tagging or publishing workflows.

## Before editing: identify behavior and ownership

Translate the request into an observable outcome, then establish:

1. **Ingress:** Does the caller use the CLI, Agent MCP/Unix, Hub HTTP/Apps MCP, Hub-Agent wire protocol, a Console screen, or another entry point? Each ingress adapter retains its own authentication, protocol, errors, and response envelope.
2. **Effects:** Which service, process, database, file, scheduler, or platform adapter actually performs the side effect? Do not simulate behavior by changing only forwarding or presentation code.
3. **State:** Who owns configuration, runtime state, persistent records, cache projections, and audit evidence? Identify responsibility for concurrent writes, recovery, cancellation, expiration, and cleanup.
4. **Contracts and consumers:** Follow the call chain through DTOs/schemas, authorization gates, error projections, CLI/TUI, Hub, and Console consumers. Update affected surfaces only; do not add unused entry points merely for symmetry.
5. **Observation:** Choose return values/errors, persistent state, process lifecycle, UI state, or wire/HTTP projections as verification outcomes. Testing a helper alone does not prove consumer-visible behavior.

`docs/archive/` is historical reference, not a source of current architecture rules or pending tasks. Current code and the corresponding contracts establish implemented behavior; the conventions below do not imply that differing implementations already follow one universal rule. See the [documentation standard](../documentation-standard.md) for writing conventions.

## Module boundaries and code placement

The Rust workspace has five crates, listed in the root [`Cargo.toml`](../../Cargo.toml): `agentic-gpt`, `agentic-gpt-hub`, `agentic-gpt-protocol`, `agentic-apply-patch`, and `agentic-browser-host`. Current workspace dependency directions are Agent → protocol and apply-patch; Hub → protocol. Protocol contains shared wire/domain DTOs and serialization contracts, not database access, networking, authorization, or business side effects. [apply-patch](../../crates/agentic-apply-patch/src/lib.rs) provides pure patch parsing, matching, and text-update algorithms. [browser-host](../../crates/agentic-browser-host/src/main.rs) is an independent extension bridge, not an owner of Agent runtime or Hub state. Check crate manifests and actual calls before changing dependencies; avoid reverse dependencies or moving bridge-process logic into the core crates.

Agent and Hub explicitly assemble many private modules from their binary crate roots. A directory is not a Cargo crate, and some modules are mapped with `#[path]`. This map describes where current code lives; it does not mean every change must touch every layer:

| Subsystem | Current responsibilities and placement |
| --- | --- |
| Agent runtime | [`runtime/`](../../crates/agentic-gpt/src/runtime/state.rs): shared runtime state, startup, supervisor, and instance lock; `state.rs` aggregates state shared by the resource owners. |
| Agent ingress | [`ingress/`](../../crates/agentic-gpt/src/ingress/stdio_server.rs): stdio MCP, Unix local control, Hub/HTTP transport, and OAuth adapters. Each ingress retains its authentication, protocol, and error boundaries. Tool argument schemas live in [`stdio_schema.rs`](../../crates/agentic-gpt/src/ingress/stdio_schema.rs). |
| Agent operations | [`operations/`](../../crates/agentic-gpt/src/operations/operation.rs): admission/authorization based on actual ingress, shared local services, command policy, confirmation, and result mapping. This is not a universal dispatcher through which every tool entry point already passes. |
| Agent process | [`process/`](../../crates/agentic-gpt/src/process/managed.rs): managed process lifecycle, execution preflight, output, and cancellation. The history adapter is in [`storage/process_history.rs`](../../crates/agentic-gpt/src/storage/process_history.rs). |
| Agent files / MCP | [`files/`](../../crates/agentic-gpt/src/files/file_ops.rs) handles file operations; [`mcp/`](../../crates/agentic-gpt/src/mcp/mcp.rs) handles downstream MCP clients, calls, and concurrency. Arbitrary downstream tool requests are not equivalent to local tools. |
| Agent skills / Room / Browser | [`skills/`](../../crates/agentic-gpt/src/skills/skills.rs) and [`skill_installs.rs`](../../crates/agentic-gpt/src/skills/skill_installs.rs) handle Skill discovery, execution, and installation. [`room/`](../../crates/agentic-gpt/src/room/room_repository.rs) handles Agent-local Room reads/maintenance; [`browser/`](../../crates/agentic-gpt/src/browser/browser_runtime.rs) handles Browser runtime discovery and distribution. |
| Agent storage / config / UI | [`storage/`](../../crates/agentic-gpt/src/storage/private_state.rs) handles private state, the transport ledger, and audit; [`config/`](../../crates/agentic-gpt/src/config/config.rs) handles configuration; [`ui/`](../../crates/agentic-gpt/src/ui/cli.rs) contains the CLI, TUI, and configuration TUI. |
| Hub runtime / agents | [`runtime/`](../../crates/agentic-gpt-hub/src/runtime/state.rs) handles Hub runtime state and the server; [`agents/`](../../crates/agentic-gpt-hub/src/agents/dispatch.rs) handles the Agent registry, connection lifecycle, dispatch, and run-receipt coordination. Hub dispatches commands; Agent performs their side effects. |
| Hub ingress / storage | [`ingress/http/`](../../crates/agentic-gpt-hub/src/ingress/http/routes.rs) handles HTTP/OpenAPI routes; [`ingress/mcp/`](../../crates/agentic-gpt-hub/src/ingress/mcp/mcp_server.rs) handles Apps MCP; [`storage/`](../../crates/agentic-gpt-hub/src/storage/runs.rs) and [`db.rs`](../../crates/agentic-gpt-hub/src/storage/db.rs) handle persistent Hub run receipts and the database. The two ingress surfaces have distinct identity and response-envelope contracts. |

The Agent crate root, [`main.rs`](../../crates/agentic-gpt/src/main.rs), lists the full `#[path]` assembly. Follow the enclosing crate's module registration, visibility, and naming conventions. Locate callers and `pub` boundaries before choosing a location. A directory alone is not a reason to create another crate, nor does a single capability justify a generic framework.

Console is a separate Kotlin Multiplatform project with `shared`, `androidApp`, `desktopApp`, and `webApp` Gradle projects. Put platform-independent rules in `shared/src/commonMain` and their pure-rule tests in `shared/src/commonTest`. Keep Android SDK, Room, notification, and receiver code in Android source sets/adapters. Android, Desktop, and Web apps have their own source sets; common code must not depend on the Android SDK. See the [Console development guide](../../console/README.md) for configured targets and tasks.

## From a request to observable behavior

1. **Reproduce and narrow the scope.** Record the ingress, current response/state, and expected difference. Read the interface, configuration, and operations documentation alongside the implementation. Check direct callers, persistence schemas, shared DTOs, and fixed tool-surface tests; archived diagnoses or stale index paths are not substitutes for source code.
2. **Identify owners and contracts.** Determine whether the change affects local behavior, remote wire/HTTP/MCP/API contracts, or only a UI projection. Trace input to its owner, effects to their implementation, and results back to the caller. Reuse existing DTOs, adapters, and error types. Do not expose every local capability remotely or force different transports into an envelope that erases their semantics.
3. **Implement along existing boundaries.** Ingress adapters parse, validate, authenticate/admit, and map input. Domain/service owners implement behavior; persistence or platform adapters own their resources. Migrate all callers and remove replaced paths rather than retaining useless aliases or compatibility layers.
4. **Define the lifecycle.** Specify state creation, concurrent access, expiration, cancellation, restart recovery, and deletion. Enforce resource limits at the actual consumption boundary. Check locking, asynchronous waits, and failure paths when changing runtime configuration, processes/tasks, connection generations, or UI projections.
5. **Update the affected surfaces.** Update Agent-local, Hub wire, HTTP/OpenAPI, Apps MCP, CLI/TUI, Console, and user documentation where applicable. Public contract changes require checking consumers and authoritative schemas; an available remote ingress does not imply that every local capability belongs on Hub.
6. **Verify consumer behavior.** Select meaningful boundary and failure tests, then exercise the real entry point or an isolated smoke scenario. Unit tests prove local rules, not transport behavior, CLI/TUI startup, database migration, or OS effects.

## State, concurrency, lifecycle, and resource safety

State should have a clearly identified write owner. Hold locks only where necessary; do not hold them while waiting for network operations, receivers, task completion, or potentially reentrant helpers. Identify existing lock order and concurrency conventions first. Validate generation and ownership when applying updates across connections or asynchronous requests so late messages cannot overwrite current state.

For asynchronous tasks, child processes, MCP calls, and UI scheduling, identify who starts, holds, and reclaims resources. Distinguish ending a wait from cancelling an operation. Define cooperative cancellation, terminal-state evidence, and state after an owner restarts. A timeout, cache miss, or departing requester does not mean side effects have been cancelled. Apply appropriate bounds to processes, batches, concurrent requests, output/results, file/network payloads, and caches. Rejection or exhaustion must not fabricate success or leave unexplained partial side effects. See the [interface reference](../interfaces.md) and [tool contract matrix](../tool-contract-matrix.md) for established limits; do not apply one arbitrary number to every feature.

Distinguish the actual authorities rather than calling every record a cache or audit log:

- The Agent process owner maintains managed execution state; [`process_history.rs`](../../crates/agentic-gpt/src/storage/process_history.rs) persists process history. Hub's process cache is an expiring state projection, not execution authority, an output/result source, or cancellation evidence.
- Persistent receipts in Hub [`storage/runs.rs`](../../crates/agentic-gpt-hub/src/storage/runs.rs) retain dispatch identity, deduplication, and late-result coordination. They are not a process-state cache and do not prove that unobserved side effects have stopped.
- [`skill_installs.rs`](../../crates/agentic-gpt/src/skills/skill_installs.rs) persists Skill installation records and defines their recovery and terminal states. Skill execution still uses managed processes. Execution holds a per-Skill shared lease, while installation replacement takes an exclusive lease for the same key to prevent overlap; these lifecycles are not interchangeable.
- Audit records provide evidence of operations. For example, [`storage/audit.rs`](../../crates/agentic-gpt/src/storage/audit.rs) must not become mutable business state or a secret store. Check consumers and security boundaries before changing recorded fields, retention, or sensitive information.

Review security at three layers: **ingress, operation, and effect**. Ingress validates identity/authentication and transport origin. Operation admission uses the actual ingress, configuration, profile/toolsets, and capabilities. Effects apply command policy, path limits, confirmation, and resource-owner checks before execution. Tool descriptors, MCP annotations, discovery filtering, and model prompts provide metadata, not authorization. Validate external content, paths, network targets, commands, and redirects at runtime; do not describe existing policy as a general OS sandbox. See the [interface reference](../interfaces.md) and [operations guide](../operations.md).

## Adding a configuration field

Follow the existing structure and neighboring field patterns rather than merely inserting a key into arbitrary JSON:

1. Define the section, type, Serde defaults, constructor defaults, behavior for older sparse configurations, and range/cross-field validation in the [configuration model and loader](../../crates/agentic-gpt/src/config/config.rs). `Config.extra` does not implement a business field. See the [configuration reference](../configuration.md) for user-facing keys and examples.
2. If the field is CLI-editable, add its key, type/description, and validating setter to the [configuration key registry](../../crates/agentic-gpt/src/config/config_keys.rs), following the existing [configuration CLI](../../crates/agentic-gpt/src/config/config_cli.rs) path. Update initialization templates, validation, review, and masking where applicable. Not every internal field needs CLI/TUI controls.
3. All Agent configuration writers must acquire the shared mutation lock through `acquire_config_mutation_lock` and write through `write_config_with_backup`. Do not add configuration writers that bypass the lock or write helper. See [`config.rs`](../../crates/agentic-gpt/src/config/config.rs) for locking, atomic writes, and backups.
4. Determine in [startup and reload](../../crates/agentic-gpt/src/runtime/startup.rs) whether the field is read at startup, per request, through live reload, or requires a restart. Only validated fields included in the live subset are hot-reloaded. Already admitted Process/Skill operations retain their admission-time configuration snapshot; new requests after reload use the new configuration. Do not silently change policy for an operation already in progress.
5. Check CLI/TUI display, safe summaries, logging, diagnostics, audit, and protocol summaries. Use existing secret-reference/resolution mechanisms; do not put plaintext credentials in arguments, logs, summaries, or fixed test data. Test defaults, older files, invalid/boundary values, rejected writes, and actual consumer behavior after activation/reload. Keep full configuration details in the reference rather than duplicating schemas here.

## Local operations, Hub commands, and remote interfaces

An Agent-local operation invokes an Agent owner through a local ingress, such as MCP/Unix or CLI; it does not automatically become a remote command. A Hub wire command is an explicit DTO/envelope shared by Agent and Hub, dispatched by Hub and executed by Agent. Reliable receipts must preserve run/request/hash identities and late-result semantics. Hub HTTP/OpenAPI and Apps MCP each own authentication, input schemas, status/error projections, and response envelopes. Do not conflate these interfaces or confuse Apps `AgenticResult` with flat HTTP responses.

Follow the real call chain when updating a surface and its tests. Wire changes require protocol, Agent/Hub mapping, and serialization/receipt tests. HTTP changes require handlers, `openapi/hub.yaml`, and parity checks. Apps MCP changes require tool schemas, handlers, and response tests. Local MCP changes require local schema, dispatch, admission, and fixed tool-surface tests. Change only affected surfaces and shared owners; do not mechanically expose local operations remotely. See the [interface reference](../interfaces.md), [tool contract matrix](../tool-contract-matrix.md), and [OpenAPI](../../openapi/hub.yaml).

For a new Hub command, check the following in order:

1. Define requests/responses in the appropriate Protocol domain file (for example, process requests belong in [`process.rs`](../../crates/agentic-gpt-protocol/src/process.rs)). Update [`HubCommand`](../../crates/agentic-gpt-protocol/src/envelopes.rs) and Hub ingress mappings. Do not place every domain's DTOs in the process file.
2. Check `hub_command_name`, `operation_namespace`, and operation classifications in Agent [`operation.rs`](../../crates/agentic-gpt/src/operations/operation.rs). Verify that `authorize` admits or rejects the operation correctly for its actual ingress, runtime mode, profile, and toolsets. Room operations also require checking `is_current_room_operation`, `is_hub_room_toolset_operation`, and their admission branches. A matching name or prefix does not mean classification has been updated.
3. Update consumption and result-mapping branches in [`local_service.rs`](../../crates/agentic-gpt/src/operations/local_service.rs), connecting them to the actual resource owner. When exposing an MCP tool, also review its read-only, destructive, and open-world annotations; they do not replace runtime authorization.
4. Cover allowed, denied, and disabled-capability paths through real ingress. New Room commands particularly need verification across normal Agents, disabled toolsets, and an active Room—not merely proof that a DTO serializes.

## Coding and testing

Use `rustfmt` for Rust. Functions, variables, and modules use `snake_case`; types and traits use `PascalCase`; constants use `SCREAMING_SNAKE_CASE`. Keep Protocol within its DTO/serialization boundary, without business side effects. Follow the enclosing crate's error propagation, asynchronous code, and module assembly conventions. Kotlin uses four-space indentation. Common rules belong in common source sets; platform SDKs and resources stay in their platform source sets. Do not introduce platform dependencies into common code. New abstractions must solve actual duplication or ownership problems, not anticipate hypothetical future platforms.

Place Rust unit tests in module-local `#[cfg(test)]` modules or existing test files; use the existing `#[tokio::test]` convention for asynchronous tests. Integration tests belong in the crate's `tests/` directory. Kotlin tests belong in the corresponding `commonTest` or platform test source set, using existing conventions such as `kotlin.test.Test`. Follow the target module's layout rather than establishing a second convention for the same behavior.

Tests should catch consumer-visible behavior, boundaries, errors, state transitions, concurrency invariants, and contract differences—not merely calls, forwarding, mock echoes, nonempty output, or incidental defaults. Test reopening/conflicts/migration for persistence, contention/atomicity for concurrency, ingress differences and rejection for authorization, and serialization shapes plus real handlers for wire/schema changes. Pure tests do not replace smoke checks for high-risk entry points.

Before deleting a test, identify its assertions and the concrete failures it can detect. Record **which named retained tests** detect the same failures, or explain why the old assertions only pin implementation details, non-contract wording, or incidental defaults. Similar inputs or overlapping coverage do not establish redundancy. If a distinct, still-valid failure cannot be ruled out, retain the test or first add equivalent behavioral coverage. Cleanup must not remove detection of consumer-visible regressions.

| Representative scenario | Implementation links | Test/verification entry points and expected observations |
| --- | --- | --- |
| Configuration limits and boundaries | [Configuration model](../../crates/agentic-gpt/src/config/config.rs), [key registry](../../crates/agentic-gpt/src/config/config_keys.rs), [process owner](../../crates/agentic-gpt/src/process/managed.rs) | [`tests/config_cli.rs`](../../crates/agentic-gpt/tests/config_cli.rs): `cargo test -p agentic-gpt --test config_cli`; runtime tests in [`runtime/main_tests.rs`](../../crates/agentic-gpt/src/runtime/main_tests.rs). Check default/older-file parsing, rejection without writes, new values for subsequent requests after reload, snapshots for admitted processes, and actual admission/rejection at the consumer's limit. Select boundaries from the field's existing constraints rather than inventing a universal threshold. |
| `process.exec` across surfaces | [Protocol request DTO](../../crates/agentic-gpt-protocol/src/process.rs), [Agent authorization](../../crates/agentic-gpt/src/operations/operation.rs), [shared local service](../../crates/agentic-gpt/src/operations/local_service.rs), [stdio schema](../../crates/agentic-gpt/src/ingress/stdio_schema.rs), [managed execution](../../crates/agentic-gpt/src/process/managed.rs), [Hub HTTP routes](../../crates/agentic-gpt-hub/src/ingress/http/routes.rs), [Hub Apps MCP](../../crates/agentic-gpt-hub/src/ingress/mcp/mcp_server.rs) | Fixed Agent tool surface: `cargo test -p agentic-gpt normal_and_room_tool_sets_follow_fixed_surface_contract`; Hub receipt replay: `cargo test -p agentic-gpt-hub pending_replay_sends_reliable_envelope`; HTTP/OpenAPI schemas and live handlers: `target/contract-venv/bin/python scripts/check_contract_parity.py`. Fixed-surface and replay tests alone prove neither execution nor every interface envelope. Observe managed execution, actual results/errors, and each surface's projection. HTTP/OpenAPI parity coverage does not replace complete local MCP/Apps MCP coverage. |
| Console Attention due-time and terminal transitions | [Common transition policy](../../console/shared/src/commonMain/kotlin/work/slhaf/agentic/console/domain/attention/AttentionTransitionPolicy.kt), [Android state holder](../../console/androidApp/src/main/kotlin/work/slhaf/agentic/console/attention/AttentionListStateHolder.kt), [Room/runtime coordinator](../../console/androidApp/src/main/kotlin/work/slhaf/agentic/console/platform/attention/AttentionRuntimeCoordinator.kt) | [`AttentionTransitionPolicyTest.kt`](../../console/shared/src/commonTest/kotlin/work/slhaf/agentic/console/AttentionTransitionPolicyTest.kt): run `./gradlew :shared:jvmTest` from `console/`. Check Schedule at `dueAt - 1`, Trigger at `dueAt`, no repeated trigger, and terminal transitions clearing actions without repeated finalization. Changes to Room conditional updates or platform scheduling also require adapter/DAO verification. |

The table identifies existing test names and commands in the source; it does not claim that editing this page ran those tests. A Rust test filter matching no tests is not verification.

## Local verification commands and scope

Run this local verification flow from the repository root. The Rust steps match the current [CI workflow](../../.github/workflows/ci.yml); the Python steps use an isolated virtual environment rather than copying the CI commands verbatim:

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

`fmt` checks formatting, `check` type-checks the workspace, strict `clippy` treats warnings as errors, workspace `test` runs Rust tests, and the Agent/Hub build supplies binaries for live parity. Clippy is a CI gate and may report existing findings. These commands do not establish actual CLI, UI, OS-effect, or production-deployment behavior. The parity virtual environment is an equivalent local setup: CI currently uses `python3 -m pip install ...` directly, not the literal venv sequence above. Parity checks OpenAPI schemas and live behavior separately, using `target/debug/agentic-gpt` and `target/debug/agentic-gpt-hub` by default, or explicit `--agent-bin PATH` and `--hub-bin PATH` arguments. It runs isolated loopback/private-home processes, not production credentials, external deployments, or all platforms.

Other verified targeted Rust test entry points:

```bash
cargo test -p agentic-gpt normal_and_room_tool_sets_follow_fixed_surface_contract
cargo test -p agentic-gpt deterministic_tool_contract_corpus_exercises_public_dispatch
cargo test -p agentic-gpt-hub pending_replay_sends_reliable_envelope
cargo test -p agentic-gpt-hub generation_stale_reliable_messages_do_not_touch_current
cargo test -p agentic-gpt --test config_cli
```

Run Console Gradle tasks from `console/`: `./gradlew :shared:jvmTest`, `:shared:testAndroidHostTest`, `:shared:jsTest`, and `:shared:wasmJsTest`. `:androidApp:assembleDebug` only builds an APK; it does not install or launch the app. See the [Console development guide](../../console/README.md) for Desktop/Web launch tasks. Current Rust CI does not include Gradle tasks.

For a CLI smoke check without application-state changes, run `cargo run -p agentic-gpt -- --help` or `cargo run -p agentic-gpt-hub -- --help`. Do not run initialization or service-start commands for verification unless `HOME`, configuration directories, and other state roots explicitly point to disposable temporary directories with a known cleanup scope. Never write test configuration into the real `~/.agentic_gpt/`. Full CLI/TUI or Console UI verification requires launching the entry point and observing output, interaction, and state. Gradle unit tests or assembly alone do not prove the screen works. If the target platform cannot run, report that platform behavior as unverified.

## Completion checklist and decisions requiring review

- [ ] Ingress, effect owners, state owners, direct callers, and affected contracts are identified.
- [ ] Implementation follows current module boundaries without duplicate paths, artificial dispatchers, or useless compatibility code.
- [ ] Locking, lifecycle, cancellation evidence, capacity, persistence/cache/audit authority, and failure paths are defined.
- [ ] Applicable runtime authorization, secret handling, actual consumers, and user documentation are updated.
- [ ] Tests detect consumer-visible regressions; matching targeted tests and entry-point smoke checks have run.
- [ ] Language counterparts agree on facts and link to authoritative schemas/references.

Small local changes can follow existing owners and contracts. Changes to public wire/HTTP/MCP/CLI contracts, persistence formats or state ownership, trust/permission boundaries, or cross-platform behavior require identifying compatibility, migration/recovery, secure failure semantics, and all consumers before updating the relevant owners and contracts. If existing patterns do not determine a new behavior, identify the unresolved decision rather than inventing an abstraction. See [configuration](../configuration.md), [interfaces](../interfaces.md), the [tool contract matrix](../tool-contract-matrix.md), [operations](../operations.md), and [releases](releasing.md) for their respective details.
