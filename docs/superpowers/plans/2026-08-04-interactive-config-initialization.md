# Interactive Config Initialization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a bilingual, discoverable `agentic-gpt config` experience with a modern interactive initializer that defaults to Standalone + Normal, remains deterministic in scripts, and exposes every supported `config set` key through a typed registry.

**Architecture:** Keep clap as the stable command grammar, add a runtime localization layer around its generated `Command`, move configuration command ownership out of `main.rs`, and separate pure template construction from terminal prompting and file commits. The interactive path uses `inquire 0.9.4`; the non-interactive path uses the same builder and validators, so both routes produce the same canonical `Config` shape.

**Tech Stack:** Rust 2021, rustc/cargo 1.97.1, clap 4.5, inquire 0.9.4, serde/serde_json, anyhow, standard-library filesystem and terminal APIs, existing AgenticGPT config validation and backup helpers.

## Global Constraints

- Default initialization mode is exactly `standalone`; default capability profile is exactly `normal`.
- Command names, argument names, enum values, JSON keys, and stable error codes remain English in every locale.
- User-facing descriptions, help headings, wizard prompts, validation feedback, cancellation messages, and completion summaries are available in `zh-CN` and `en`.
- Language selection precedence is `--language` → `LC_ALL` → `LC_MESSAGES` → `LANG` → English.
- `config init` must never block when stdin/stdout/stderr are not interactive terminals or when `--non-interactive` is present.
- Interactive cancellation or final refusal must not create or modify the config file or tunnel secret file.
- Tunnel secret directories are mode `0700`; secret files are mode `0600`; secret values never appear in logs, summaries, errors, debug output, or test failure messages.
- Keep the canonical JSON schema and existing legacy-load compatibility unchanged.
- `config set` remains a controlled key registry, not a general JSONPath editor.
- MCP servers and command-policy collections remain outside the first-run wizard.
- Run Rust tests serially with `--test-threads=1`; do not run multiple heavy Rust test commands in parallel on the laptop.
- Every behavior change starts with a failing test and ends with a focused commit.

---

## File Structure

### Files to create

- `crates/agentic-gpt/src/config_cli.rs` — owns `ConfigCommand`, `ConfigInitArgs`, policy/path command enums, config key registry, `config keys`, and config command dispatch.
- `crates/agentic-gpt/src/cli_i18n.rs` — owns UI language detection, bilingual text catalog, clap command-tree localization, and localized clap error rendering.
- `crates/agentic-gpt/src/config_templates.rs` — owns runtime mode, initialization input/output types, pure config template construction, mode-specific validation, pending actions, and redacted secret value wrappers.
- `crates/agentic-gpt/src/config_wizard.rs` — owns prompt abstraction, scripted test backend, `inquire` backend, wizard state machine, final summary, and transactional config/secret commit.
- `crates/agentic-gpt/tests/config_cli.rs` — black-box binary coverage for non-TTY init, localized help, `config keys`, and representative `config set` mutations.

### Files to modify

- `Cargo.toml` — add workspace dependency `inquire = "0.9.4"`.
- `crates/agentic-gpt/Cargo.toml` — consume `inquire.workspace = true`.
- `Cargo.lock` — lock the new prompt dependency graph.
- `crates/agentic-gpt/src/main.rs` — register new modules, add global `--language`, build localized clap command at runtime, and delegate config handling.
- `crates/agentic-gpt/src/config.rs` — expose/reuse range validators, add Hub-mode validation, and keep all mode builders on the same canonical validation rules.
- `crates/agentic-gpt/src/policy.rs` — import config command enums from `config_cli` after extraction.
- `README.md`, `README.zh-CN.md` — document default interactive init, explicit non-interactive examples, language selection, and `config keys`.
- `docs/configuration.md`, `docs/configuration.zh-CN.md` — document mode/profile templates, optional wizard sections, secret behavior, registry keys, and script semantics.
### Files reviewed but not expected to change

- `config.example.json` — remains the complete checked-in example. Verify it still parses and validates, but keep explanatory material in Markdown rather than adding invalid JSON comments.

---

### Task 1: Characterize and Extract the Existing Config CLI

**Files:**
- Create: `crates/agentic-gpt/src/config_cli.rs`
- Create: `crates/agentic-gpt/tests/config_cli.rs`
- Modify: `crates/agentic-gpt/src/main.rs:1-220,515-658`
- Modify: `crates/agentic-gpt/src/policy.rs:1-12`

**Interfaces:**
- Consumes: existing `Config`, `write_config_with_backup`, `policy::mutate_rule`, `policy::mutate_path_policy`, and `mcp::mutate_servers`.
- Produces: `pub(crate) enum ConfigCommand`, `pub(crate) enum RuleCommand`, `pub(crate) enum PathCommand`, `pub(crate) enum PathRootCommand`, `pub(crate) enum PathRootKind`, and `pub(crate) async fn handle_config(config_path: PathBuf, command: ConfigCommand) -> Result<()>`.

- [ ] **Step 1: Add a black-box characterization test for current config behavior**

Create `crates/agentic-gpt/tests/config_cli.rs` with a reusable binary helper and a test that proves `init`, `set`, and `show` still round-trip before extraction:

```rust
use serde_json::Value;
use std::{fs, path::PathBuf, process::Command};
use uuid::Uuid;

fn binary_path() -> PathBuf {
    std::env::var("CARGO_BIN_EXE_agentic-gpt")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_agentic_gpt"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/agentic-gpt")
        })
}

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "agentic-config-cli-{label}-{}",
        Uuid::new_v4().simple()
    ))
}

#[test]
fn config_init_set_and_show_round_trip() {
    let root = temp_root("round-trip");
    fs::create_dir_all(&root).unwrap();
    let config = root.join("config.json");
    let binary = binary_path();

    let init = Command::new(&binary)
        .args(["config", "--config"])
        .arg(&config)
        .arg("init")
        .output()
        .unwrap();
    assert!(init.status.success(), "{}", String::from_utf8_lossy(&init.stderr));

    let set = Command::new(&binary)
        .args(["config", "--config"])
        .arg(&config)
        .args(["set", "room.timezone", "Asia/Tokyo"])
        .output()
        .unwrap();
    assert!(set.status.success(), "{}", String::from_utf8_lossy(&set.stderr));

    let show = Command::new(&binary)
        .args(["config", "--config"])
        .arg(&config)
        .arg("show")
        .output()
        .unwrap();
    assert!(show.status.success(), "{}", String::from_utf8_lossy(&show.stderr));
    let value: Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(value["room"]["timezone"], "Asia/Tokyo");

    let _ = fs::remove_dir_all(root);
}
```

- [ ] **Step 2: Run the characterization test before refactoring**

Run:

```bash
cargo test -p agentic-gpt --test config_cli config_init_set_and_show_round_trip -- --test-threads=1
```

Expected: PASS. This establishes a green refactoring baseline rather than a new feature failure.

- [ ] **Step 3: Move config-owned command types and dispatch into `config_cli.rs`**

Move the current `ConfigCommand`, `RuleCommand`, `PathCommand`, `PathRootCommand`, and `PathRootKind` definitions unchanged. Move `handle_config` and `tunnel_config` unchanged except for module-qualified imports.

The new module starts with:

```rust
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::Subcommand;

use crate::{
    config::{self, normalize_confirmation_language, write_config_with_backup, Config, ReportingDetail},
    mcp::{self, McpConfigCommand},
    policy::{self, PolicyDecision},
};

#[derive(Subcommand)]
pub(crate) enum ConfigCommand {
    Init,
    Show,
    Set { key: String, value: String },
    Allow { #[command(subcommand)] command: RuleCommand },
    Confirm { #[command(subcommand)] command: RuleCommand },
    Deny { #[command(subcommand)] command: RuleCommand },
    Path { #[command(subcommand)] command: PathCommand },
    Mcp { #[command(subcommand)] command: McpConfigCommand },
}
```

Update `main.rs` to declare `mod config_cli;`, import `config_cli::ConfigCommand`, and dispatch with `config_cli::handle_config(config_path(config), command).await`.

Update `policy.rs` imports from crate root to:

```rust
use crate::config_cli::{PathCommand, PathRootCommand, PathRootKind, RuleCommand};
```

- [ ] **Step 4: Run focused unit and integration tests after extraction**

Run:

```bash
cargo test -p agentic-gpt --test config_cli config_init_set_and_show_round_trip -- --test-threads=1
cargo test -p agentic-gpt configured_ -- --test-threads=1
cargo test -p agentic-gpt remove_rule_ -- --test-threads=1
```

Expected: both commands PASS; no behavior change.

- [ ] **Step 5: Commit the extraction**

```bash
git add crates/agentic-gpt/src/main.rs \
        crates/agentic-gpt/src/config_cli.rs \
        crates/agentic-gpt/src/policy.rs \
        crates/agentic-gpt/tests/config_cli.rs
git commit -m "refactor: isolate config cli"
```

---

### Task 2: Add Language Detection and the Bilingual Text Catalog

**Files:**
- Create: `crates/agentic-gpt/src/cli_i18n.rs`
- Modify: `crates/agentic-gpt/src/main.rs:1-55,205-260`

**Interfaces:**
- Consumes: process argv and locale environment variables.
- Produces:
  - `pub(crate) enum LanguageChoice { Auto, ZhCn, En }`
  - `pub(crate) enum UiLanguage { ZhCn, En }`
  - `pub(crate) fn resolve_language(choice: LanguageChoice, env: &impl LocaleSource) -> UiLanguage`
  - `pub(crate) fn prescan_language(args: &[OsString]) -> Result<LanguageChoice, String>`
  - `pub(crate) fn text(language: UiLanguage) -> &'static CliText`
  - `pub(crate) trait LocaleSource { fn get(&self, key: &str) -> Option<OsString>; }`

- [ ] **Step 1: Write failing unit tests for language precedence and normalization**

Create `cli_i18n.rs` with tests first. Use an in-memory environment implementation so tests never mutate global process environment:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, ffi::OsString};

    #[derive(Default)]
    struct TestEnv(BTreeMap<String, OsString>);

    impl LocaleSource for TestEnv {
        fn get(&self, key: &str) -> Option<OsString> {
            self.0.get(key).cloned()
        }
    }

    #[test]
    fn explicit_language_overrides_locale_environment() {
        let env = TestEnv(BTreeMap::from([("LC_ALL".into(), "en_US.UTF-8".into())]));
        assert_eq!(resolve_language(LanguageChoice::ZhCn, &env), UiLanguage::ZhCn);
    }

    #[test]
    fn locale_precedence_is_lc_all_then_lc_messages_then_lang() {
        let env = TestEnv(BTreeMap::from([
            ("LANG".into(), "zh_CN.UTF-8".into()),
            ("LC_MESSAGES".into(), "zh_TW.UTF-8".into()),
            ("LC_ALL".into(), "C.UTF-8".into()),
        ]));
        assert_eq!(resolve_language(LanguageChoice::Auto, &env), UiLanguage::En);
    }

    #[test]
    fn prescan_accepts_equals_and_split_forms_anywhere() {
        assert_eq!(
            prescan_language(&["agentic-gpt".into(), "config".into(), "--language=zh-CN".into()]).unwrap(),
            LanguageChoice::ZhCn
        );
        assert_eq!(
            prescan_language(&[
                "agentic-gpt".into(),
                "config".into(),
                "init".into(),
                "--language".into(),
                "en".into(),
            ]).unwrap(),
            LanguageChoice::En
        );
    }
}
```

- [ ] **Step 2: Run the language tests and verify RED**

Run:

```bash
cargo test -p agentic-gpt cli_i18n::tests -- --test-threads=1
```

Expected: compilation FAIL because `cli_i18n` types and functions do not yet exist.

- [ ] **Step 3: Implement language types, locale resolution, and a complete text catalog skeleton**

Define exact stable CLI values:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub(crate) enum LanguageChoice {
    Auto,
    #[value(name = "zh-CN", alias = "zh", alias = "zh_CN")]
    ZhCn,
    #[value(name = "en", alias = "en-US", alias = "en_US")]
    En,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UiLanguage {
    ZhCn,
    En,
}
```

Implement locale normalization by lowercasing, replacing `_` with `-`, and treating values beginning with `zh` as Chinese. Treat `C`, `POSIX`, empty, invalid UTF-8, and all other values as English.

Define a compile-time-complete `CliText` struct rather than scattered literals. The first task only needs fields consumed by later modules, but both language constants must initialize every field:

```rust
pub(crate) struct CliText {
    pub app_about: &'static str,
    pub config_about: &'static str,
    pub config_init_about: &'static str,
    pub config_keys_about: &'static str,
    pub config_set_about: &'static str,
    pub usage_heading: &'static str,
    pub commands_heading: &'static str,
    pub options_heading: &'static str,
    pub arguments_heading: &'static str,
    pub help_flag: &'static str,
    pub version_flag: &'static str,
    pub cancelled: &'static str,
    pub initialized: &'static str,
    pub optional_settings_prompt: &'static str,
    pub safe_defaults_option: &'static str,
    pub choose_sections_option: &'static str,
}
```

Use static `ZH_CN_TEXT` and `EN_TEXT` values and return them through `text(language)`.

- [ ] **Step 4: Add global `--language` to the clap grammar without changing parsing flow yet**

Add to `Cli`:

```rust
#[arg(long, global = true, value_enum, default_value_t = LanguageChoice::Auto)]
language: LanguageChoice,
```

Keep `Cli::parse()` temporarily. This step exposes the stable flag but does not yet localize help.

- [ ] **Step 5: Run focused tests and CLI parsing checks**

Run:

```bash
cargo test -p agentic-gpt cli_i18n::tests -- --test-threads=1
cargo test -p agentic-gpt local_cli_accepts_config_before_or_after_subcommand -- --test-threads=1
```

Expected: PASS.

- [ ] **Step 6: Commit language foundations**

```bash
git add crates/agentic-gpt/src/cli_i18n.rs crates/agentic-gpt/src/main.rs
git commit -m "feat: add cli language selection"
```

---

### Task 3: Replace the Config Set Match with a Typed Registry

**Files:**
- Modify: `crates/agentic-gpt/src/config_cli.rs`
- Modify: `crates/agentic-gpt/src/config.rs:170-220,650-690,850-910`
- Modify: `crates/agentic-gpt/src/cli_i18n.rs`
- Modify: `crates/agentic-gpt/tests/config_cli.rs`

**Interfaces:**
- Consumes: `Config`, `UiLanguage`, existing validators and tunnel helper.
- Produces:
  - `pub(crate) enum ConfigValueKind`
  - `pub(crate) enum ConfigSection`
  - `pub(crate) struct ConfigKeySpec`
  - `pub(crate) static CONFIG_KEYS: &[ConfigKeySpec]`
  - `pub(crate) fn apply_config_key(config: &mut Config, key: &str, value: &str) -> Result<()>`
  - `ConfigCommand::Keys { section: Option<ConfigSection>, json: bool }`

- [ ] **Step 1: Write failing unit tests for the missing keys and nullable notebook root**

Add tests inside `config_cli.rs`:

```rust
#[test]
fn registry_applies_new_scalar_and_list_keys() {
    let mut config = Config::default_config().unwrap();
    apply_config_key(&mut config, "displayName", "Desk Agent").unwrap();
    apply_config_key(&mut config, "backupLimit", "7").unwrap();
    apply_config_key(
        &mut config,
        "sandbox.requiredRuntimePaths",
        r#"["/usr","/opt/runtime"]"#,
    )
    .unwrap();
    apply_config_key(&mut config, "limits.maxConcurrentTasks", "4").unwrap();
    apply_config_key(&mut config, "limits.maxActiveJobs", "auto").unwrap();
    apply_config_key(&mut config, "limits.maxFileSearchContextLines", "12").unwrap();

    assert_eq!(config.display_name, "Desk Agent");
    assert_eq!(config.backup_limit, 7);
    assert_eq!(config.sandbox.required_runtime_paths.len(), 2);
    assert_eq!(config.limits.max_concurrent_tasks, 4);
    assert_eq!(config.limits.max_file_search_context_lines, 12);
}

#[test]
fn registry_clears_nullable_notebook_root() {
    let mut config = Config::default_config().unwrap();
    apply_config_key(&mut config, "room.notebookRoot", "/tmp/notebook").unwrap();
    apply_config_key(&mut config, "room.notebookRoot", "null").unwrap();
    assert!(config.room.notebook_root.is_none());
}

#[test]
fn registry_keys_are_unique_and_have_bilingual_metadata() {
    let mut seen = std::collections::BTreeSet::new();
    for spec in CONFIG_KEYS {
        assert!(seen.insert(spec.key), "duplicate key: {}", spec.key);
        assert!(!spec.description.en.is_empty());
        assert!(!spec.description.zh_cn.is_empty());
        assert!(!spec.example.is_empty());
    }
}
```

- [ ] **Step 2: Run the registry tests and verify RED**

Run:

```bash
cargo test -p agentic-gpt config_cli::tests::registry -- --test-threads=1
```

Expected: FAIL because `apply_config_key`, `CONFIG_KEYS`, and the missing key handlers do not exist.

- [ ] **Step 3: Expose reusable config validators**

In `config.rs`, add small public(crate) validators used by both serde and CLI:

```rust
pub(crate) fn validate_max_file_search_context_lines(value: usize) -> Result<()> {
    if value > MAX_FILE_SEARCH_CONTEXT_LINES {
        return Err(anyhow!(
            "maxFileSearchContextLines must be between 0 and {MAX_FILE_SEARCH_CONTEXT_LINES}"
        ));
    }
    Ok(())
}

pub(crate) fn parse_max_active_jobs(value: &str) -> Result<MaxActiveJobs> {
    if value == "auto" {
        return Ok(MaxActiveJobs::Auto);
    }
    Ok(MaxActiveJobs::Explicit(value.parse::<usize>()?))
}
```

Change the serde deserializer to call `validate_max_file_search_context_lines` so CLI and JSON loading share the same upper bound.

- [ ] **Step 4: Implement the registry as the single source of truth**

Use function pointers for mutation and static bilingual metadata:

```rust
pub(crate) struct LocalizedText {
    pub(crate) en: &'static str,
    pub(crate) zh_cn: &'static str,
}

pub(crate) struct ConfigKeySpec {
    pub(crate) key: &'static str,
    pub(crate) section: ConfigSection,
    pub(crate) kind: ConfigValueKind,
    pub(crate) nullable: bool,
    pub(crate) description: LocalizedText,
    pub(crate) example: &'static str,
    apply: fn(&mut Config, &str) -> Result<()>,
}
```

Move every currently supported `match key.as_str()` arm into a named mutator and list it in `CONFIG_KEYS`. Include aliases such as `workerUrl` as separate specs pointing to the same mutator, but mark aliases in metadata so `config keys` can identify canonical keys.

Add the missing canonical keys:

```text
displayName
backupLimit
sandbox.bubblewrapPath
sandbox.requiredRuntimePaths
limits.maxConcurrentTasks
limits.maxActiveJobs
limits.maxFileSearchContextLines
```

For JSON list values, parse into `Vec<String>` or `Vec<PathBuf>` with `serde_json::from_str`. For `room.notebookRoot`, treat the exact token `null` as `None`.

- [ ] **Step 5: Add `config keys` text and JSON output**

Extend the command grammar:

```rust
Keys {
    #[arg(long, value_enum)]
    section: Option<ConfigSection>,
    #[arg(long)]
    json: bool,
},
```

Derive `ValueEnum` for sections with stable names such as `identity`, `hub`, `confirmation`, `sandbox`, `limits`, `skills`, `room`, and `tunnel`.

Text output groups specs by section and uses `UiLanguage`; JSON output is stable and language-neutral except for separate `description.en` and `description.zhCN` fields:

```json
{
  "keys": [
    {
      "key": "limits.maxActiveJobs",
      "section": "limits",
      "type": "auto-or-non-negative-integer",
      "nullable": false,
      "example": "auto",
      "description": {
        "en": "Total active Job capacity: auto or a non-negative integer.",
        "zhCN": "活动 Job 总容量：auto 或非负整数。"
      }
    }
  ]
}
```

- [ ] **Step 6: Add black-box tests for `config keys` and representative mutations**

Add tests that run:

```rust
let keys = Command::new(&binary)
    .args(["--language", "en", "config", "keys", "--json"])
    .output()
    .unwrap();
assert!(keys.status.success());
let value: Value = serde_json::from_slice(&keys.stdout).unwrap();
assert!(value["keys"].as_array().unwrap().iter().any(|entry| {
    entry["key"] == "limits.maxActiveJobs"
}));
```

Also initialize a temp config and call `config set limits.maxFileSearchContextLines 101`; assert non-zero exit and the canonical bound appears without modifying the file.

- [ ] **Step 7: Run focused registry and CLI tests**

Run:

```bash
cargo test -p agentic-gpt config_cli::tests -- --test-threads=1
cargo test -p agentic-gpt --test config_cli config_keys -- --test-threads=1
cargo test -p agentic-gpt --test config_cli config_set -- --test-threads=1
```

Expected: PASS.

- [ ] **Step 8: Commit the registry**

```bash
git add crates/agentic-gpt/src/config.rs \
        crates/agentic-gpt/src/config_cli.rs \
        crates/agentic-gpt/src/cli_i18n.rs \
        crates/agentic-gpt/tests/config_cli.rs
git commit -m "feat: expose typed config key registry"
```

---

### Task 4: Build Pure Mode/Profile Configuration Templates

**Files:**
- Create: `crates/agentic-gpt/src/config_templates.rs`
- Modify: `crates/agentic-gpt/src/main.rs:1-35`
- Modify: `crates/agentic-gpt/src/config.rs:650-690`

**Interfaces:**
- Consumes: `Config::default_config`, `UiLanguage`, current `WorkerProfile`, `agentic_home`, and config validators.
- Produces:
  - `pub(crate) enum RuntimeMode { Standalone, Hub, Local }`
  - `pub(crate) enum OptionalSection`
  - `pub(crate) struct InitInput`
  - `pub(crate) struct InitBuild`
  - `pub(crate) enum PendingAction`
  - `pub(crate) struct SecretValue`
  - `pub(crate) struct SecretWritePlan`
  - `pub(crate) fn build_config(input: InitInput) -> Result<InitBuild>`
  - `Config::validate_hub()`

- [ ] **Step 1: Write failing template tests for all mode/profile paths**

Create tests in `config_templates.rs`:

```rust
#[test]
fn default_template_is_standalone_normal_with_safe_placeholders() {
    let built = build_config(InitInput::non_interactive_defaults(UiLanguage::En)).unwrap();
    assert_eq!(built.mode, RuntimeMode::Standalone);
    assert_eq!(built.profile, WorkerProfile::Normal);
    let tunnel = built.config.tunnel.as_ref().unwrap();
    assert_eq!(tunnel.tunnel_id, "tunnel_replace-me");
    assert!(tunnel.api_key.starts_with("file:"));
    assert!(built.pending.contains(&PendingAction::ReplaceTunnelId));
    assert!(built.pending.contains(&PendingAction::ProvisionTunnelSecret));
    built.config.validate_standalone().unwrap();
}

#[test]
fn local_template_omits_tunnel_and_validates_locally() {
    let input = InitInput {
        mode: RuntimeMode::Local,
        profile: WorkerProfile::Normal,
        ..InitInput::non_interactive_defaults(UiLanguage::En)
    };
    let built = build_config(input).unwrap();
    assert!(built.config.tunnel.is_none());
    built.config.validate_local().unwrap();
}

#[test]
fn hub_template_uses_supplied_connection_values() {
    let mut input = InitInput::non_interactive_defaults(UiLanguage::ZhCn);
    input.mode = RuntimeMode::Hub;
    input.hub_url = Some("https://hub.example.com".into());
    input.hub_transport = Some("sse".into());
    input.agent_id = Some("desk".into());
    input.agent_secret = Some(SecretValue::new("secret"));
    let built = build_config(input).unwrap();
    assert_eq!(built.config.confirmation_language, "zh-CN");
    built.config.validate_hub().unwrap();
}
```

- [ ] **Step 2: Run template tests and verify RED**

Run:

```bash
cargo test -p agentic-gpt config_templates::tests -- --test-threads=1
```

Expected: compilation FAIL because the template module and Hub validation do not exist.

- [ ] **Step 3: Add `Config::validate_hub()` using existing schema rules**

Implement:

```rust
pub(crate) fn validate_hub(&self) -> Result<()> {
    self.validate_local()?;
    let url = reqwest::Url::parse(&self.hub_url).map_err(|_| anyhow!("hub_url_invalid"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(anyhow!("hub_url_invalid"));
    }
    if !matches!(self.hub_transport.as_str(), "websocket" | "sse") {
        return Err(anyhow!("hub_transport_invalid"));
    }
    if self.agent_id.trim().is_empty() {
        return Err(anyhow!("agent_id_required"));
    }
    if self.agent_secret.trim().is_empty() || self.agent_secret == "change-me" {
        return Err(anyhow!("agent_secret_required"));
    }
    Ok(())
}
```

Do not call this from existing Hub runtime startup in this task; it is initially for generated-template validation and explicit initializer behavior, avoiding an unrelated compatibility change.

- [ ] **Step 4: Implement redacted secret and initialization data types**

Define `SecretValue` without derived `Debug` or `Display`:

```rust
pub(crate) struct SecretValue(String);

impl SecretValue {
    pub(crate) fn new(value: impl Into<String>) -> Self { Self(value.into()) }
    pub(crate) fn expose(&self) -> &str { &self.0 }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretValue([REDACTED])")
    }
}
```

Update `WorkerProfile` to derive `Eq` and `PartialEq`, then define the initialization types explicitly:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub(crate) enum RuntimeMode {
    Standalone,
    Hub,
    Local,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum OptionalSection {
    Identity,
    Workspace,
    Confirmation,
    Limits,
    Sandbox,
    Room,
    TunnelClient,
    HubReporting,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TunnelSecretSource {
    File,
    Environment,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PendingAction {
    ReplaceTunnelId,
    ProvisionTunnelSecret,
    ConfigureHubUrl,
    ReplaceAgentSecret,
}

pub(crate) struct InitInput {
    pub(crate) mode: RuntimeMode,
    pub(crate) profile: WorkerProfile,
    pub(crate) ui_language: UiLanguage,
    pub(crate) tunnel_id: Option<String>,
    pub(crate) tunnel_api_key: Option<String>,
    pub(crate) hub_url: Option<String>,
    pub(crate) hub_transport: Option<String>,
    pub(crate) agent_id: Option<String>,
    pub(crate) agent_secret: Option<SecretValue>,
    pub(crate) display_name: Option<String>,
    pub(crate) workspace_root: Option<PathBuf>,
    pub(crate) path_policy: Option<PathPolicyConfig>,
    pub(crate) confirmation_provider: Option<ConfirmationProviderConfig>,
    pub(crate) confirmation_language: Option<String>,
    pub(crate) limits: Option<LimitsConfig>,
    pub(crate) sandbox: Option<SandboxConfig>,
    pub(crate) room: Option<RoomConfig>,
    pub(crate) tunnel_client: Option<TunnelClientConfig>,
    pub(crate) hub_reporting: Option<HubReportingConfig>,
}

impl InitInput {
    pub(crate) fn non_interactive_defaults(language: UiLanguage) -> Self {
        Self {
            mode: RuntimeMode::Standalone,
            profile: WorkerProfile::Normal,
            ui_language: language,
            tunnel_id: None,
            tunnel_api_key: None,
            hub_url: None,
            hub_transport: None,
            agent_id: None,
            agent_secret: None,
            display_name: None,
            workspace_root: None,
            path_policy: None,
            confirmation_provider: None,
            confirmation_language: None,
            limits: None,
            sandbox: None,
            room: None,
            tunnel_client: None,
            hub_reporting: None,
        }
    }
}

pub(crate) struct InitBuild {
    pub(crate) mode: RuntimeMode,
    pub(crate) profile: WorkerProfile,
    pub(crate) config: Config,
    pub(crate) pending: Vec<PendingAction>,
}

pub(crate) struct SecretWritePlan {
    pub(crate) path: PathBuf,
    pub(crate) value: SecretValue,
}

impl std::fmt::Debug for SecretWritePlan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SecretWritePlan")
            .field("path", &self.path)
            .field("value", &"[REDACTED]")
            .finish()
    }
}
```

Do not derive `Debug` for `InitInput` or `InitBuild`; both can contain Hub credentials through `SecretValue` or the built `Config`.

- [ ] **Step 5: Implement `build_config` as a pure function**

Start from `Config::default_config()`, set `confirmation_language` from `UiLanguage`, then branch by mode:

- Standalone: populate a full `TunnelConfig`; use supplied tunnel ID/reference or safe placeholders; call `validate_standalone()`.
- Hub: set supplied connection fields or placeholders; call `validate_hub()` only when no required placeholder remains; otherwise validate URL shape and MCP separately and return pending actions.
- Local: force `tunnel = None`; call `validate_local()`.

Profile controls Room-related defaults and whether `OptionalSection::Room` is legal, but it does not change the canonical Config schema.

- [ ] **Step 6: Run template and config validation tests**

Run:

```bash
cargo test -p agentic-gpt config_templates::tests -- --test-threads=1
cargo test -p agentic-gpt config::tests -- --test-threads=1
```

Expected: PASS.

- [ ] **Step 7: Commit template construction**

```bash
git add crates/agentic-gpt/src/config.rs \
        crates/agentic-gpt/src/config_templates.rs \
        crates/agentic-gpt/src/main.rs
git commit -m "feat: build mode-aware config templates"
```

---

### Task 5: Add Deterministic Non-Interactive Config Init

**Files:**
- Modify: `crates/agentic-gpt/src/config_cli.rs`
- Modify: `crates/agentic-gpt/src/config_templates.rs`
- Modify: `crates/agentic-gpt/src/cli_i18n.rs`
- Modify: `crates/agentic-gpt/tests/config_cli.rs`

**Interfaces:**
- Consumes: `build_config`, current config path helper, `write_config_with_backup`.
- Produces:
  - `pub(crate) struct ConfigInitArgs`
  - `pub(crate) fn init_non_interactive(config_path: &Path, args: &ConfigInitArgs, language: UiLanguage) -> Result<InitSummary>`
  - `pub(crate) struct InitSummary { mode, profile, config_path, pending }`

Define the summary without secret-bearing fields:

```rust
pub(crate) struct InitSummary {
    pub(crate) mode: RuntimeMode,
    pub(crate) profile: WorkerProfile,
    pub(crate) config_path: PathBuf,
    pub(crate) pending: Vec<PendingAction>,
}
```

- [ ] **Step 1: Write failing black-box tests for non-TTY defaults and explicit modes**

Add three integration tests using `Command::output()` so the child has no interactive TTY:

```rust
#[test]
fn non_tty_init_defaults_to_standalone_normal_without_blocking() {
    let root = temp_root("default-init");
    fs::create_dir_all(&root).unwrap();
    let config = root.join("config.json");
    let output = Command::new(binary_path())
        .args(["--language", "en", "config", "--config"])
        .arg(&config)
        .arg("init")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(value["tunnel"]["tunnelId"], "tunnel_replace-me");
    assert!(String::from_utf8_lossy(&output.stderr).contains("replace tunnel ID"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn explicit_local_init_does_not_emit_tunnel_config() {
    let root = temp_root("local-init");
    fs::create_dir_all(&root).unwrap();
    let config = root.join("config.json");
    let output = Command::new(binary_path())
        .args(["--language", "en", "config", "--config"])
        .arg(&config)
        .args([
            "init",
            "--mode",
            "local",
            "--profile",
            "normal",
            "--non-interactive",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert!(value.get("tunnel").is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn explicit_hub_init_writes_supplied_connection_fields() {
    let root = temp_root("hub-init");
    fs::create_dir_all(&root).unwrap();
    let config = root.join("config.json");
    let secret = "hub-secret-marker-7c6138";
    let output = Command::new(binary_path())
        .args(["--language", "en", "config", "--config"])
        .arg(&config)
        .args([
            "init",
            "--mode",
            "hub",
            "--profile",
            "room",
            "--hub-url",
            "https://hub.example.com",
            "--hub-transport",
            "sse",
            "--agent-id",
            "desk",
            "--agent-secret",
            secret,
            "--non-interactive",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(value["hubUrl"], "https://hub.example.com");
    assert_eq!(value["hubTransport"], "sse");
    assert_eq!(value["agentId"], "desk");
    assert_eq!(value["agentSecret"], secret);
    assert!(!String::from_utf8_lossy(&output.stdout).contains(secret));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(secret));
    let _ = fs::remove_dir_all(root);
}
```

The Hub test must assert stderr/stdout do not contain the agent secret.

- [ ] **Step 2: Run the non-interactive tests and verify RED**

Run:

```bash
cargo test -p agentic-gpt --test config_cli non_tty_init -- --test-threads=1
cargo test -p agentic-gpt --test config_cli explicit_local_init -- --test-threads=1
cargo test -p agentic-gpt --test config_cli explicit_hub_init -- --test-threads=1
```

Expected: FAIL because `config init` has no mode/profile/options and still writes `Config::default_config()`.

- [ ] **Step 3: Define the init argument grammar**

Replace `ConfigCommand::Init` with:

```rust
Init(ConfigInitArgs),
```

Define:

```rust
#[derive(clap::Args, Clone, Default)]
pub(crate) struct ConfigInitArgs {
    #[arg(long, value_enum)]
    pub(crate) mode: Option<RuntimeMode>,
    #[arg(long, value_enum)]
    pub(crate) profile: Option<WorkerProfile>,
    #[arg(long)]
    pub(crate) non_interactive: bool,
    #[arg(long)]
    pub(crate) tunnel_id: Option<String>,
    #[arg(long)]
    pub(crate) tunnel_api_key: Option<String>,
    #[arg(long)]
    pub(crate) hub_url: Option<String>,
    #[arg(long, value_parser = ["websocket", "sse"])]
    pub(crate) hub_transport: Option<String>,
    #[arg(long)]
    pub(crate) agent_id: Option<String>,
    #[arg(long)]
    pub(crate) agent_secret: Option<String>,
}
```

Keep `mode` and `profile` as `Option` so the wizard can distinguish an explicit flag from an omitted value; apply `standalone` and `normal` only when building the effective input. Do not derive `Debug` for `ConfigInitArgs`, because it contains `agent_secret`.

Document in help text that `--agent-secret` is visible to local process inspection and shell history; interactive hidden input is preferred. Do not print the parsed value anywhere.

- [ ] **Step 4: Implement the non-interactive route with localized pending actions**

Add `handle_init` that determines interactivity later, but for now always calls `init_non_interactive`. Convert flags to `InitInput`, call `build_config`, write with `write_config_with_backup`, print a short success line to stdout, and print each `PendingAction` to stderr using the bilingual catalog.

Do not create tunnel secret files in non-interactive mode; `--tunnel-api-key` is a reference such as `file:/path` or `env:NAME`, never the secret itself.

- [ ] **Step 5: Preserve existing integration test semantics**

Update any existing test that depends on exact init output only when necessary. Do not rewrite tests that already replace or clear the tunnel block; the new Standalone placeholder is valid input for those tests.

- [ ] **Step 6: Run focused and existing initializer consumers**

Run:

```bash
cargo test -p agentic-gpt --test config_cli -- --test-threads=1
cargo test -p agentic-gpt --test local_control local_runtime_cli_exercises_real_unix_mcp_surface -- --test-threads=1
cargo test -p agentic-gpt --test standalone_supervisor supervisor_launches_real_worker_and_completes_local_mcp_call -- --test-threads=1
```

Expected: PASS.

- [ ] **Step 7: Commit non-interactive init**

```bash
git add crates/agentic-gpt/src/config_cli.rs \
        crates/agentic-gpt/src/config_templates.rs \
        crates/agentic-gpt/src/cli_i18n.rs \
        crates/agentic-gpt/tests/config_cli.rs
git commit -m "feat: add mode-aware noninteractive config init"
```

---

### Task 6: Implement the Interactive Wizard and Transactional Secret Commit

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/agentic-gpt/Cargo.toml`
- Modify: `Cargo.lock`
- Create: `crates/agentic-gpt/src/config_wizard.rs`
- Modify: `crates/agentic-gpt/src/main.rs:1-35`
- Modify: `crates/agentic-gpt/src/config_cli.rs`
- Modify: `crates/agentic-gpt/src/config_templates.rs`
- Modify: `crates/agentic-gpt/src/cli_i18n.rs`

**Interfaces:**
- Consumes: `ConfigInitArgs`, `InitInput`, `build_config`, `CliText`, config backup writer.
- Produces:
  - `pub(crate) enum PromptRequest`
  - `pub(crate) enum PromptAnswer`
  - `pub(crate) trait PromptBackend`
  - `pub(crate) struct InquirePromptBackend`
  - `pub(crate) fn available_optional_sections(mode: RuntimeMode, profile: WorkerProfile) -> Vec<OptionalSection>`
  - `pub(crate) fn run_wizard(backend: &mut impl PromptBackend, defaults: ConfigInitArgs, language: UiLanguage) -> Result<WizardOutcome>`
  - `pub(crate) fn render_summary(build: &InitBuild, secret_write: Option<&SecretWritePlan>, language: UiLanguage) -> String`
  - `pub(crate) fn commit_wizard_outcome(config_path: &Path, outcome: WizardOutcome) -> Result<InitSummary>`

- [ ] **Step 1: Add `inquire 0.9.4` and verify dependency resolution only**

Add to workspace dependencies:

```toml
inquire = "0.9.4"
```

Add to the binary crate:

```toml
inquire.workspace = true
```

Run:

```bash
cargo check -p agentic-gpt
```

Expected: PASS and `Cargo.lock` updated. Do not write wizard production code before the failing behavior tests.

- [ ] **Step 2: Write failing scripted-backend tests for wizard branching and cancellation**

Use domain-neutral prompt requests so business tests do not need a PTY:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PromptRequest {
    SelectMode { default: RuntimeMode },
    SelectProfile { default: WorkerProfile },
    SelectSecretSource { default: TunnelSecretSource },
    Text { id: PromptId, default: Option<String> },
    Secret { id: PromptId },
    Confirm { id: PromptId, default: bool },
    OptionalSections { available: Vec<OptionalSection> },
}

pub(crate) enum PromptAnswer {
    Mode(RuntimeMode),
    Profile(WorkerProfile),
    SecretSource(TunnelSecretSource),
    Text(String),
    Secret(SecretValue),
    Bool(bool),
    Sections(Vec<OptionalSection>),
    Cancel,
}

pub(crate) trait PromptBackend {
    fn ask(&mut self, request: PromptRequest) -> Result<PromptAnswer>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PromptId {
    TunnelId,
    TunnelSecretPath,
    TunnelSecretEnvironment,
    TunnelSecretValue,
    HubUrl,
    HubTransport,
    AgentId,
    AgentSecret,
    DisplayName,
    WorkspaceRoot,
    ConfirmationProvider,
    ConfirmationLanguage,
    MaxConcurrentTasks,
    MaxActiveJobs,
    MaxFileSearchContextLines,
    SandboxEnabled,
    BubblewrapPath,
    RequiredRuntimePaths,
    RoomTimezone,
    DiaryBoundaryHour,
    NotebookRoot,
    TunnelClientVersion,
    TunnelCacheDir,
    TunnelAutoDownload,
    TunnelExecutable,
    TunnelDownloadUrl,
    TunnelSha256,
    HubReportingEnabled,
    HubReportingDetail,
    ConfigureOptionalSections,
    WriteSecretNow,
    ConfirmWrite,
}

pub(crate) struct WizardOutcome {
    pub(crate) build: InitBuild,
    pub(crate) secret_write: Option<SecretWritePlan>,
    pub(crate) summary: String,
}
```

Do not derive `Debug` for `PromptAnswer` or `WizardOutcome`; either may own a secret. `SecretWritePlan` supplies its own redacted `Debug` solely for targeted assertions.

Add tests:

```rust
#[test]
fn default_wizard_builds_standalone_normal_and_defers_optional_sections() {
    let mut backend = ScriptedPromptBackend::new([
        PromptAnswer::Mode(RuntimeMode::Standalone),
        PromptAnswer::Profile(WorkerProfile::Normal),
        PromptAnswer::Text("tunnel_virtual".into()),
        PromptAnswer::SecretSource(TunnelSecretSource::File),
        PromptAnswer::Text("~/.agentic_gpt/secrets/tunnel-api-key".into()),
        PromptAnswer::Bool(false),
        PromptAnswer::Bool(false),
        PromptAnswer::Bool(true),
    ]);
    let outcome = run_wizard(&mut backend, ConfigInitArgs::default(), UiLanguage::En).unwrap();
    assert_eq!(
        outcome.build.config.tunnel.as_ref().unwrap().tunnel_id,
        "tunnel_virtual"
    );
    assert!(outcome.secret_write.is_none());
}

#[test]
fn cancelled_wizard_returns_cancelled_without_write_plan() {
    let mut backend = ScriptedPromptBackend::new([PromptAnswer::Cancel]);
    let error = match run_wizard(&mut backend, ConfigInitArgs::default(), UiLanguage::En) {
        Ok(_) => panic!("cancelled wizard unexpectedly returned an outcome"),
        Err(error) => error,
    };
    assert_eq!(error.to_string(), "config_init_cancelled");
}

#[test]
fn room_optional_sections_include_room_but_local_excludes_tunnel_sections() {
    let room = available_optional_sections(RuntimeMode::Standalone, WorkerProfile::Room);
    assert!(room.contains(&OptionalSection::Room));
    assert!(room.contains(&OptionalSection::TunnelClient));
    assert!(room.contains(&OptionalSection::HubReporting));

    let local = available_optional_sections(RuntimeMode::Local, WorkerProfile::Normal);
    assert!(!local.contains(&OptionalSection::Room));
    assert!(!local.contains(&OptionalSection::TunnelClient));
    assert!(!local.contains(&OptionalSection::HubReporting));
}

#[test]
fn wizard_summary_never_contains_secret() {
    let marker = "wizard-secret-marker-79b2a4";
    let mut backend = ScriptedPromptBackend::new([
        PromptAnswer::Mode(RuntimeMode::Standalone),
        PromptAnswer::Profile(WorkerProfile::Normal),
        PromptAnswer::Text("tunnel_virtual".into()),
        PromptAnswer::SecretSource(TunnelSecretSource::File),
        PromptAnswer::Text("~/.agentic_gpt/secrets/tunnel-api-key".into()),
        PromptAnswer::Bool(true),
        PromptAnswer::Secret(SecretValue::new(marker)),
        PromptAnswer::Bool(false),
        PromptAnswer::Bool(true),
    ]);
    let outcome = run_wizard(&mut backend, ConfigInitArgs::default(), UiLanguage::En).unwrap();
    assert!(!outcome.summary.contains(marker));
    assert!(!format!("{:?}", outcome.secret_write).contains(marker));
}
```

- [ ] **Step 3: Run wizard tests and verify RED**

Run:

```bash
cargo test -p agentic-gpt config_wizard::tests -- --test-threads=1
```

Expected: compilation FAIL because the wizard module and prompt abstractions do not exist.

- [ ] **Step 4: Implement the pure wizard state machine against `PromptBackend`**

Implement this exact order:

1. Mode selection unless explicitly supplied as a non-default CLI override.
2. Profile selection unless explicitly supplied.
3. Mode-required prompts.
4. Optional-settings yes/no prompt.
5. Mode/profile-filtered section MultiSelect when requested.
6. Section-specific prompts.
7. Redacted summary.
8. Final confirmation.

Return a typed cancellation error such as `anyhow!("config_init_cancelled")`; the caller localizes it. Never call filesystem functions from `run_wizard`.

- [ ] **Step 5: Implement the `inquire` backend without overriding its default render config**

Use `Select`, `MultiSelect`, `Confirm`, `Text`, `CustomType`, and `Password`. Call `.without_filtering()` for short fixed lists. Keep inquire's default render config so `NO_COLOR` continues to work automatically.

Map `InquireError::OperationCanceled` and `InquireError::OperationInterrupted` to `config_init_cancelled`. Wrap other errors as `config_init_prompt_failed: {source}`.

- [ ] **Step 6: Implement staged secret writes and rollback-aware commit**

`commit_wizard_outcome` runs only after final confirmation. For a file secret plan:

1. Expand and validate the target path.
2. Capture prior state as `Absent` or `{ bytes, mode }` without logging bytes.
3. Create parent directories and set mode `0700`.
4. Write a same-directory temporary file with `create_new(true)` and mode `0600`.
5. `sync_all`, then rename over the target.
6. Write config via `write_config_with_backup`.
7. If config write fails, restore the prior secret state atomically or remove the newly created file.
8. Return success only after both operations complete.

Implement Unix permission setup with `std::os::unix::fs::{OpenOptionsExt, PermissionsExt}`. The project is Linux-specific, so no cross-platform fallback is required.

- [ ] **Step 7: Write filesystem tests for permission and rollback behavior**

Add tests using temp directories:

```rust
fn test_outcome_with_secret(secret_path: &Path, value: &str) -> WizardOutcome {
    let mut input = InitInput::non_interactive_defaults(UiLanguage::En);
    input.tunnel_id = Some("tunnel_test".into());
    input.tunnel_api_key = Some(format!("file:{}", secret_path.display()));
    let build = build_config(input).unwrap();
    WizardOutcome {
        build,
        secret_write: Some(SecretWritePlan {
            path: secret_path.to_path_buf(),
            value: SecretValue::new(value),
        }),
        summary: "Configuration ready; secret value is hidden.".into(),
    }
}

#[test]
fn commit_creates_secret_directory_0700_and_file_0600() {
    use std::os::unix::fs::PermissionsExt;

    let root = unique_temp_dir("secret-permissions");
    let config_path = root.join("config").join("config.json");
    let secret_path = root.join("secrets").join("tunnel-api-key");
    let marker = "permission-secret-marker-93d1";
    let outcome = test_outcome_with_secret(&secret_path, marker);

    commit_wizard_outcome(&config_path, outcome).unwrap();

    let dir_mode = fs::metadata(secret_path.parent().unwrap())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    let file_mode = fs::metadata(&secret_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(dir_mode, 0o700);
    assert_eq!(file_mode, 0o600);
    assert_eq!(fs::read(&secret_path).unwrap().len(), marker.len());
    assert!(config_path.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn config_write_failure_restores_existing_secret() {
    let root = unique_temp_dir("secret-rollback");
    let secret_path = root.join("secrets").join("tunnel-api-key");
    fs::create_dir_all(secret_path.parent().unwrap()).unwrap();
    let original = b"original-secret";
    fs::write(&secret_path, original).unwrap();

    let config_path = root.join("config-target");
    fs::create_dir_all(&config_path).unwrap();
    let outcome = test_outcome_with_secret(&secret_path, "replacement-secret");
    let error = match commit_wizard_outcome(&config_path, outcome) {
        Ok(_) => panic!("config write unexpectedly succeeded"),
        Err(error) => error,
    };

    assert!(!error.to_string().is_empty());
    assert!(fs::read(&secret_path).unwrap() == original);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cancellation_does_not_create_config_or_secret() {
    let root = unique_temp_dir("wizard-cancel-files");
    let config_path = root.join("config.json");
    let secret_path = root.join("secrets").join("tunnel-api-key");
    let mut backend = ScriptedPromptBackend::new([PromptAnswer::Cancel]);

    let error = match run_wizard(&mut backend, ConfigInitArgs::default(), UiLanguage::En) {
        Ok(_) => panic!("cancelled wizard unexpectedly returned an outcome"),
        Err(error) => error,
    };

    assert_eq!(error.to_string(), "config_init_cancelled");
    assert!(!config_path.exists());
    assert!(!secret_path.exists());
    let _ = fs::remove_dir_all(root);
}
```

Ensure assertion messages never print the secret marker.

- [ ] **Step 8: Wire TTY detection into `config init`**

Use `std::io::IsTerminal`. Interactive mode requires all of:

```rust
!args.non_interactive
    && std::io::stdin().is_terminal()
    && std::io::stdout().is_terminal()
    && std::io::stderr().is_terminal()
```

Interactive mode calls the wizard; otherwise call the existing non-interactive path. Explicit flags become wizard defaults and skip already-satisfied required prompts.

- [ ] **Step 9: Run wizard, permission, and non-TTY regression tests**

Run:

```bash
cargo test -p agentic-gpt config_wizard::tests -- --test-threads=1
cargo test -p agentic-gpt --test config_cli non_tty_init -- --test-threads=1
cargo test -p agentic-gpt --test config_cli explicit_ -- --test-threads=1
```

Expected: PASS.

- [ ] **Step 10: Commit the wizard**

```bash
git add Cargo.toml Cargo.lock \
        crates/agentic-gpt/Cargo.toml \
        crates/agentic-gpt/src/main.rs \
        crates/agentic-gpt/src/config_cli.rs \
        crates/agentic-gpt/src/config_templates.rs \
        crates/agentic-gpt/src/config_wizard.rs \
        crates/agentic-gpt/src/cli_i18n.rs
git commit -m "feat: add interactive config wizard"
```

---

### Task 7: Localize the Complete CLI Help Tree and Owned Errors

**Files:**
- Modify: `crates/agentic-gpt/src/cli_i18n.rs`
- Modify: `crates/agentic-gpt/src/main.rs:25-55,205-260`
- Modify: `crates/agentic-gpt/src/config_cli.rs`
- Modify: `crates/agentic-gpt/src/mcp.rs:45-80`
- Modify: `crates/agentic-gpt/src/policy.rs` only if command descriptions need explicit metadata hooks
- Modify: `crates/agentic-gpt/tests/config_cli.rs`

**Interfaces:**
- Consumes: `Cli::command()`, `Cli::from_arg_matches`, `CliText`, clap error context.
- Produces:
  - `pub(crate) fn localized_command(language: UiLanguage) -> clap::Command`
  - `pub(crate) fn parse_cli(args: Vec<OsString>, env: &impl LocaleSource) -> Result<(Cli, UiLanguage), clap::Error>`
  - `pub(crate) fn render_cli_error(error: clap::Error, language: UiLanguage) -> RenderedCliError`
  - `pub(crate) fn exit_with_cli_error(error: clap::Error, language: UiLanguage) -> !`

Define the testable rendered form explicitly:

```rust
pub(crate) struct RenderedCliError {
    pub(crate) text: String,
    pub(crate) use_stderr: bool,
    pub(crate) exit_code: i32,
}
```

`exit_with_cli_error` writes `text` to the selected stream and terminates with `exit_code`; tests exercise `render_cli_error` directly and never terminate the test process.

- [ ] **Step 1: Write failing binary tests for Chinese and English help completeness**

Add tests that execute the real binary with explicit language:

```rust
#[test]
fn config_help_is_fully_localized_without_changing_tokens() {
    let zh = Command::new(binary_path())
        .args(["--language", "zh-CN", "config", "--help"])
        .output()
        .unwrap();
    let en = Command::new(binary_path())
        .args(["--language", "en", "config", "--help"])
        .output()
        .unwrap();

    let zh = String::from_utf8(zh.stdout).unwrap();
    let en = String::from_utf8(en.stdout).unwrap();
    assert!(zh.contains("用法："));
    assert!(zh.contains("命令："));
    assert!(zh.contains("初始化配置"));
    assert!(en.contains("Usage:"));
    assert!(en.contains("Commands:"));
    assert!(en.contains("Initialize configuration"));
    for token in ["init", "show", "set", "keys", "allow", "confirm", "deny", "path", "mcp"] {
        assert!(zh.contains(token));
        assert!(en.contains(token));
    }
}
```

Add a table-driven test that visits every visible command path and asserts the rendered help contains no blank `Commands` entry. Include:

```text
--help
run --help
run-as-room --help
run-as-standalone --help
run-as-local --help
local --help
local call --help
config --help
config init --help
config show --help
config set --help
config keys --help
config allow --help
config allow add --help
config allow remove --help
config confirm --help
config deny --help
config path --help
config path list --help
config path write --help
config path write add --help
config path readonly --help
config path deny --help
config mcp --help
config mcp add --help
config mcp remove --help
config mcp enable --help
config mcp disable --help
tmux --help
tmux list --help
tmux attach --help
tmux create --help
tmux close --help
```

- [ ] **Step 2: Run help tests and verify RED**

Run:

```bash
cargo test -p agentic-gpt --test config_cli config_help_is_fully_localized -- --test-threads=1
cargo test -p agentic-gpt --test config_cli every_visible_command_has_help -- --test-threads=1
```

Expected: FAIL because clap still uses derive-time English/blank descriptions.

- [ ] **Step 3: Replace `Cli::parse()` with a localized runtime command tree**

Import `CommandFactory` and `FromArgMatches`. In `main`:

```rust
let args = std::env::args_os().collect::<Vec<_>>();
let choice = cli_i18n::prescan_language(&args)
    .unwrap_or(LanguageChoice::Auto);
let language = cli_i18n::resolve_language(choice, &cli_i18n::ProcessLocale);
let mut command = cli_i18n::localized_command(language);
let matches = match command.clone().try_get_matches_from(args) {
    Ok(matches) => matches,
    Err(error) => return cli_i18n::exit_with_cli_error(error, language),
};
let cli = Cli::from_arg_matches(&matches)?;
```

`localized_command` starts from `Cli::command()`, recursively walks the known command path, and applies:

- command `about` and `long_about`
- argument help and value names
- subcommand help heading
- argument heading
- custom help template with localized heading text
- localized help/version flag descriptions

Build stable path keys such as `config.init` and `config.path.write.add`; fail unit tests when the catalog lacks any visible command or argument.

- [ ] **Step 4: Implement bilingual clap-owned error rendering for common parse failures**

Use `clap::error::ErrorKind` and available error context to localize these owned cases:

- invalid value
- unknown argument
- invalid/missing subcommand
- missing required argument
- wrong number of values

Keep the invalid token and valid English grammar tokens verbatim. Preserve clap's original exit code and stdout/stderr choice. For unrecognized future `ErrorKind` values, fall back to clap's rendered error rather than losing diagnostics.

`DisplayHelp`, `DisplayHelpOnMissingArgumentOrSubcommand`, and `DisplayVersion` print the already-localized command output and exit 0.

- [ ] **Step 5: Add tests for language position, locale auto-detection, and errors**

Add tests proving:

```text
agentic-gpt --language zh-CN config init --help
agentic-gpt config init --language zh-CN --help
```

produce equivalent Chinese help.

Spawn commands with `.env_clear()` and specific locale variables to verify precedence without depending on the developer machine. Add invalid `--mode nope` tests in both languages and assert the error is actionable while `standalone`, `hub`, and `local` remain unchanged.

- [ ] **Step 6: Run the complete CLI help/error test set**

Run:

```bash
cargo test -p agentic-gpt cli_i18n::tests -- --test-threads=1
cargo test -p agentic-gpt --test config_cli help -- --test-threads=1
cargo test -p agentic-gpt --test config_cli language -- --test-threads=1
cargo test -p agentic-gpt --test config_cli invalid_ -- --test-threads=1
```

Expected: PASS.

- [ ] **Step 7: Commit full CLI localization**

```bash
git add crates/agentic-gpt/src/main.rs \
        crates/agentic-gpt/src/cli_i18n.rs \
        crates/agentic-gpt/src/config_cli.rs \
        crates/agentic-gpt/src/mcp.rs \
        crates/agentic-gpt/src/policy.rs \
        crates/agentic-gpt/tests/config_cli.rs
git commit -m "feat: localize cli help and config errors"
```

---

### Task 8: Update User Documentation and Run Final Serial Verification

**Files:**
- Modify: `README.md`
- Modify: `README.zh-CN.md`
- Modify: `docs/configuration.md`
- Modify: `docs/configuration.zh-CN.md`
- Modify: `docs/superpowers/specs/2026-08-04-config-init-wizard-design.md` only if implementation reveals a factual correction; do not rewrite approved decisions for style.

**Interfaces:**
- Consumes: final CLI grammar and behavior.
- Produces: complete user-facing installation/configuration guidance that matches the executable help.

- [ ] **Step 1: Capture the final real help output before editing docs**

Run:

```bash
cargo build -p agentic-gpt
./target/debug/agentic-gpt --language en config init --help > /tmp/agentic-config-init.en.txt
./target/debug/agentic-gpt --language zh-CN config init --help > /tmp/agentic-config-init.zh-CN.txt
./target/debug/agentic-gpt --language en config keys > /tmp/agentic-config-keys.en.txt
./target/debug/agentic-gpt --language zh-CN config keys > /tmp/agentic-config-keys.zh-CN.txt
```

Read these generated outputs and copy only syntax that actually exists.

- [ ] **Step 2: Update both READMEs with the default happy path**

Document:

```bash
agentic-gpt config init
```

as an interactive Standalone + Normal wizard on a TTY. Add deterministic examples:

```bash
agentic-gpt config init --non-interactive
agentic-gpt config init --mode local --profile normal --non-interactive
agentic-gpt config init \
  --mode standalone \
  --profile room \
  --tunnel-id tunnel_<assigned-id> \
  --tunnel-api-key file:"$HOME/.agentic_gpt/secrets/tunnel-api-key" \
  --non-interactive
```

Explain that `--agent-secret` can leak through history/process inspection and hidden interactive input is preferred.

- [ ] **Step 3: Update configuration docs with complete behavioral boundaries**

Cover:

- mode versus profile
- TTY versus non-TTY behavior
- optional section menu and mode/profile filtering
- `--language auto|zh-CN|en` precedence
- CLI language versus `confirmationLanguage`
- `config keys [--section <SECTION>] [--json]`
- list input syntax for `config set`
- nullable `room.notebookRoot`
- secret file permission and rollback semantics
- MCP and policy commands intentionally outside the first-run wizard

Keep English and Chinese documents structurally aligned.

- [ ] **Step 4: Run documentation consistency checks**

Run:

```bash
git diff --check
python - <<'PY'
from pathlib import Path
for name in [
    'README.md',
    'README.zh-CN.md',
    'docs/configuration.md',
    'docs/configuration.zh-CN.md',
]:
    text = Path(name).read_text(encoding='utf-8')
    assert text.count('```') % 2 == 0, name
    assert 'config init' in text, name
    assert 'config keys' in text, name
print('documentation checks: PASS')
PY
```

Expected: PASS.

- [ ] **Step 5: Run formatter and static checks**

Run:

```bash
cargo fmt --all -- --check
cargo check --workspace
```

Expected: PASS with no new warnings attributable to this work.

- [ ] **Step 6: Run the complete AgenticGPT test suite serially**

Run:

```bash
cargo test -p agentic-gpt -- --test-threads=1
```

Expected: all unit and integration tests PASS, including `local_control`, `standalone_supervisor`, config registry, wizard, and help localization tests.

- [ ] **Step 7: Perform manual non-secret smoke checks**

Use temp paths only:

```bash
tmp=$(mktemp -d)
./target/debug/agentic-gpt --language en config --config "$tmp/config.json" init --non-interactive
./target/debug/agentic-gpt --language en config --config "$tmp/config.json" show >/dev/null
./target/debug/agentic-gpt --language zh-CN config keys --section limits >/dev/null
rm -rf "$tmp"
```

Expected: every command exits 0; no secret value is used or printed.

- [ ] **Step 8: Perform one real PTY cancellation smoke check**

Use util-linux `script` to give the child a genuine terminal, feed Escape, guard against hangs, and verify transactional cancellation:

```bash
tmp=$(mktemp -d)
set +e
printf '' | timeout 10s script -qec   "HOME='$tmp/home' ./target/debug/agentic-gpt --language en config --config '$tmp/config.json' init"   /dev/null >"$tmp/pty.log" 2>&1
status=$?
set -e
test "$status" -ne 124
test ! -e "$tmp/config.json"
test ! -e "$tmp/home/.agentic_gpt/secrets/tunnel-api-key"
rm -rf "$tmp"
```

Expected: the command does not time out, the wizard takes the interactive path, cancellation leaves no config or secret file, and the log contains no secret value.

- [ ] **Step 9: Commit documentation and final integration adjustments**

```bash
git add README.md README.zh-CN.md \
        docs/configuration.md docs/configuration.zh-CN.md
git commit -m "docs: document interactive config workflow"
```

- [ ] **Step 10: Verify the final branch state**

Run:

```bash
git status --short --branch
git log --oneline --decorate -10
```

Expected: clean worktree; no push is performed unless explicitly requested.
