# Slice 02 — node_repl Launch Spec

This is an orchestrator-owned implementation contract. Worker may implement it but must not change its architecture or scope.

## Goal

Add a pure launch-spec builder that combines a selected `BrowserRuntimeDescriptor` with caller-supplied node_repl-specific base environment values. This slice does not read Codex config files and does not spawn a process.

The separation is intentional:

- `BrowserRuntimeDescriptor` owns runtime/version/path facts.
- this launch builder owns deterministic runtime-coupled environment overrides.
- a later source adapter decides where `base_env` comes from (Desktop Codex config, explicit Agentic config, or another deployment source).

## Exact API

Extend `crates/agentic-gpt/src/browser_runtime.rs` with:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NodeReplLaunchSpec {
    pub(crate) program: PathBuf,
    pub(crate) cwd: PathBuf,
    pub(crate) env_overrides: BTreeMap<String, String>,
}

pub(crate) fn build_node_repl_launch_spec(
    runtime: &BrowserRuntimeDescriptor,
    base_env: &BTreeMap<String, String>,
) -> Result<NodeReplLaunchSpec>;
```

No process-wide environment snapshot belongs in this type. `env_overrides` is the bounded environment map later applied on top of the process environment by the process-launch layer.

## Program and cwd

- `program = runtime.node_repl_path.clone()`.
- `cwd` is the Browser bundle root derived from `runtime.browser_client_path.parent().and_then(parent)`.
- Invalid/under-parented client paths return stable `browser_runtime_...` errors.

## Environment merge contract

Start with an exact clone of `base_env`, then enforce runtime-coupled keys from the selected descriptor:

- `NODE_REPL_NODE_PATH` = `runtime.node_path`
- `CODEX_HOME` = `runtime.codex_home`
- `CODEX_CLI_PATH` = `runtime.codex_cli_path`
- `BROWSER_USE_CODEX_APP_VERSION` = `runtime.app_version`
- `BROWSER_USE_CODEX_APP_BUILD_FLAVOR` = `runtime.channel`

The selected runtime must also own its Browser trusted-service path:

- Parse existing `NODE_REPL_TRUSTED_SERVICES` from `base_env` as a JSON object when present.
- If absent, start from an empty JSON object.
- Preserve unrelated existing trusted-service entries.
- Set/replace only the `browser` entry with the UTF-8 string form of `runtime.browser_service_path`.
- Serialize the resulting object back into `NODE_REPL_TRUSTED_SERVICES`.
- Present-but-invalid JSON or a non-object value returns a stable `browser_runtime_trusted_services_invalid` error; do not silently discard it.

If `runtime.node_module_dirs` is non-empty:

- join the descriptor paths using the platform path-list separator via the standard library;
- set/replace `NODE_REPL_NODE_MODULE_DIRS` with that joined value.

If `runtime.node_module_dirs` is empty, leave any `base_env` value for `NODE_REPL_NODE_MODULE_DIRS` unchanged. If neither descriptor nor base supplies it, do not invent one.

Path values that must become string environment/JSON values must be valid UTF-8 or return a stable `browser_runtime_path_not_utf8:<field>` style error.

## Environment values explicitly *not* invented here

The builder preserves caller-supplied values but does not create policy/deployment-specific settings such as:

- `NODE_REPL_TRUSTED_CODE_PATHS`
- `NODE_REPL_NATIVE_PIPE_CONNECT_TIMEOUT_MS`
- `BROWSER_USE_AVAILABLE_BACKENDS`
- `BROWSER_USE_TINYSKY_ENABLED`
- `BROWSER_USE_SECURITY_MODE`
- auth/app-server/backend/native-host settings

This is important: Laptop Desktop, Orange Pi/Neko, and future explicit-runtime deployments may legitimately need different base environment policy. The launch builder only forces values that must stay version/path-consistent with the selected runtime.

## Non-goals

- no reading `~/.codex/config.toml`
- no TOML dependency
- no `BrowserConfig`
- no process spawn / rmcp client
- no AppState manager
- no tool surface
- no lifecycle/session/turn behavior
- no Neko compatibility

## Tests

Unit tests stay in `browser_runtime.rs` and cover at least:

1. program and cwd derive from descriptor;
2. stale runtime-coupled values in `base_env` are overwritten by descriptor values;
3. unrelated base env values are preserved;
4. existing trusted-service entries survive while `browser` is replaced;
5. absent trusted-services creates a browser-only object;
6. malformed/non-object trusted-services is rejected;
7. non-empty descriptor `node_module_dirs` replaces stale base value using standard path joining;
8. empty descriptor `node_module_dirs` preserves base value and does not create one when absent;
9. `BROWSER_USE_SECURITY_MODE` is neither created nor changed except by ordinary base-env preservation.

Run `cargo test -p agentic-gpt browser_runtime`, `cargo fmt --all -- --check`, and `git diff --check`.

## Worker observation rules

- Update existing `progress.md` at start and completion.
- Append to `findings.md` only for a real contract/repository contradiction or blocker.
- Do not modify `PLAN.md`, Slice 01, or this slice contract.
- No commits or push.
