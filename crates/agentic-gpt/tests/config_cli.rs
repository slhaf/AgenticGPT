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

#[test]
fn config_help_is_fully_localized_without_changing_tokens() {
    let zh = Command::new(binary_path())
        .args(["--language", "zh-CN", "config", "--help"])
        .output()
        .unwrap();
    assert!(zh.status.success(), "Chinese config help command failed");
    let zh = String::from_utf8(zh.stdout).unwrap();

    let en = Command::new(binary_path())
        .args(["--language", "en", "config", "--help"])
        .output()
        .unwrap();
    assert!(en.status.success(), "English config help command failed");
    let en = String::from_utf8(en.stdout).unwrap();

    assert!(zh.contains("用法："));
    assert!(zh.contains("命令："));
    assert!(zh.contains("初始化配置"));
    assert!(en.contains("Usage:"));
    assert!(en.contains("Commands:"));
    assert!(en.contains("Initialize configuration"));
    for token in [
        "init", "import", "show", "set", "keys", "allow", "confirm", "deny", "path", "mcp",
    ] {
        assert!(zh.contains(token), "Chinese help omitted token {token}");
        assert!(en.contains(token), "English help omitted token {token}");
    }
}

#[test]
fn every_visible_command_has_help() {
    let command_paths: &[&[&str]] = &[
        &["--help"],
        &["run", "--help"],
        &["local", "--help"],
        &["local", "call", "--help"],
        &["config", "--help"],
        &["config", "init", "--help"],
        &["config", "import", "--help"],
        &["config", "show", "--help"],
        &["config", "set", "--help"],
        &["config", "keys", "--help"],
        &["config", "allow", "--help"],
        &["config", "allow", "add", "--help"],
        &["config", "allow", "remove", "--help"],
        &["config", "confirm", "--help"],
        &["config", "deny", "--help"],
        &["config", "path", "--help"],
        &["config", "path", "list", "--help"],
        &["config", "path", "write", "--help"],
        &["config", "path", "write", "add", "--help"],
        &["config", "path", "readonly", "--help"],
        &["config", "path", "deny", "--help"],
        &["config", "mcp", "--help"],
        &["config", "mcp", "add", "--help"],
        &["config", "mcp", "remove", "--help"],
        &["config", "mcp", "enable", "--help"],
        &["config", "mcp", "disable", "--help"],
        &["tmux", "--help"],
        &["tmux", "list", "--help"],
        &["tmux", "attach", "--help"],
        &["tmux", "create", "--help"],
        &["tmux", "close", "--help"],
    ];

    for path in command_paths {
        for language in ["en", "zh-CN"] {
            let mut args = vec!["--language", language];
            args.extend(path.iter().copied());
            let output = Command::new(binary_path()).args(args).output().unwrap();
            assert!(
                output.status.success(),
                "help command failed for {language} {:?}",
                path
            );
            let text = String::from_utf8(output.stdout).unwrap();
            let headings = ["Commands:", "命令："];
            for heading in headings {
                let Some(start) = text.find(heading) else {
                    continue;
                };
                let section = &text[start + heading.len()..];
                let mut saw_entry = false;
                for line in section.lines().take_while(|line| {
                    !matches!(line.trim(), "Options:" | "选项：" | "Arguments:" | "参数：")
                }) {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        if saw_entry {
                            break;
                        }
                        continue;
                    }
                    saw_entry = true;
                    let Some(first_whitespace) = trimmed.find(char::is_whitespace) else {
                        panic!("command entry has no description for {language} {:?}", path);
                    };
                    assert!(
                        !trimmed[first_whitespace..].trim().is_empty(),
                        "blank command entry for {language} {:?}: {trimmed}",
                        path
                    );
                }
            }
        }
    }
}

#[test]
fn language_flag_is_equivalent_before_and_after_subcommand() {
    let before = Command::new(binary_path())
        .args(["--language", "zh-CN", "config", "init", "--help"])
        .output()
        .unwrap();
    let after = Command::new(binary_path())
        .args(["config", "init", "--language", "zh-CN", "--help"])
        .output()
        .unwrap();
    assert!(before.status.success(), "language-before help failed");
    assert!(after.status.success(), "language-after help failed");
    assert_eq!(before.stdout, after.stdout);
    assert!(String::from_utf8_lossy(&before.stdout).contains("用法："));
}

#[test]
fn language_auto_detection_obeys_locale_precedence() {
    let output = Command::new(binary_path())
        .env_clear()
        .env("LANG", "zh_CN.UTF-8")
        .env("LC_MESSAGES", "zh_TW.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .args(["config", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success(), "locale precedence help failed");
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage:"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("用法："));

    let output = Command::new(binary_path())
        .env_clear()
        .env("LANG", "en_US.UTF-8")
        .env("LC_MESSAGES", "zh_CN.UTF-8")
        .args(["config", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success(), "LC_MESSAGES locale help failed");
    assert!(String::from_utf8_lossy(&output.stdout).contains("用法："));

    let output = Command::new(binary_path())
        .env_clear()
        .env("LANG", "zh_CN.UTF-8")
        .args(["--language", "en", "config", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success(), "explicit language help failed");
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage:"));
}

#[test]
fn invalid_mode_is_localized_without_changing_valid_tokens() {
    for language in ["en", "zh-CN"] {
        let output = Command::new(binary_path())
            .args(["--language", language, "config", "init", "--mode", "nope"])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "invalid mode exit code changed"
        );
        let text = String::from_utf8_lossy(&output.stderr);
        assert!(text.contains("nope"), "invalid value was omitted");
        for token in ["standalone", "hub", "local"] {
            assert!(text.contains(token), "valid token {token} was omitted");
        }
        if language == "zh-CN" {
            assert!(text.contains("无效"), "Chinese invalid-value text missing");
        } else {
            assert!(
                text.contains("invalid value"),
                "English invalid-value text missing"
            );
        }
    }
}

#[test]
fn invalid_owned_parse_errors_keep_stream_and_tokens() {
    let cases = [
        (
            ["config", "init", "--bogus"].as_slice(),
            "--bogus",
            "--bogus",
            "意外参数",
            "unexpected argument",
        ),
        (
            ["config", "ninit"].as_slice(),
            "ninit",
            "ninit",
            "无法识别的子命令",
            "unrecognized subcommand",
        ),
        (
            ["config", "set"].as_slice(),
            "<键>",
            "<KEY>",
            "必需参数",
            "required arguments",
        ),
    ];
    for (args, zh_token, en_token, zh_message, en_message) in cases {
        let zh = Command::new(binary_path())
            .args(["--language", "zh-CN"])
            .args(args)
            .output()
            .unwrap();
        assert_eq!(zh.status.code(), Some(2), "Chinese parse exit code changed");
        assert!(zh
            .stderr
            .windows(zh_token.len())
            .any(|window| window == zh_token.as_bytes()));
        assert!(String::from_utf8_lossy(&zh.stderr).contains(zh_message));

        let en = Command::new(binary_path())
            .args(["--language", "en"])
            .args(args)
            .output()
            .unwrap();
        assert_eq!(en.status.code(), Some(2), "English parse exit code changed");
        assert!(String::from_utf8_lossy(&en.stderr).contains(en_token));
        assert!(String::from_utf8_lossy(&en.stderr).contains(en_message));
    }
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
        .args(["init", "--profile", "room", "--non-interactive"])
        .output()
        .unwrap();
    assert!(init.status.success(), "config init command failed");

    let set = Command::new(&binary)
        .args(["config", "--config"])
        .arg(&config)
        .args(["set", "room.timezone", "Asia/Tokyo"])
        .output()
        .unwrap();
    assert!(set.status.success(), "config set command failed");

    let disk: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(disk["mode"], "standalone");
    assert_eq!(disk["profile"], "room");
    assert!(disk.get("limits").is_none());

    let show = Command::new(&binary)
        .args(["config", "--config"])
        .arg(&config)
        .arg("show")
        .output()
        .unwrap();
    assert!(show.status.success(), "config show command failed");
    let value: Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(value["mode"], "standalone");
    assert_eq!(value["profile"], "room");
    assert_eq!(value["limits"]["maxConcurrentTasks"], 2);
    assert_eq!(value["room"]["timezone"], "Asia/Tokyo");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn config_keys_json_lists_registry() {
    let binary = binary_path();
    let keys = Command::new(&binary)
        .args(["--language", "en", "config", "keys", "--json"])
        .output()
        .unwrap();
    assert!(keys.status.success(), "config keys command failed");
    let value: Value = serde_json::from_slice(&keys.stdout).unwrap();
    let entries = value["keys"].as_array().unwrap();
    assert!(entries
        .iter()
        .any(|entry| { entry["key"] == "limits.maxActiveProcesses" }));
    let mode = entries.iter().find(|entry| entry["key"] == "mode").unwrap();
    assert_eq!(
        mode["choices"],
        serde_json::json!(["standalone", "hub", "local"])
    );
    let confirmation = entries
        .iter()
        .find(|entry| entry["key"] == "confirmationProvider.channels")
        .unwrap();
    assert_eq!(
        confirmation["choices"],
        serde_json::json!(["freedesktop", "ntfy"])
    );
    assert!(!entries
        .iter()
        .any(|entry| entry["key"] == "confirmationProvider"));
    let sandbox = entries
        .iter()
        .find(|entry| entry["key"] == "sandbox.enabled")
        .unwrap();
    assert_eq!(sandbox["choices"], serde_json::json!(["true", "false"]));
}

#[test]
fn config_set_rejects_file_search_context_bound_without_writing() {
    let root = temp_root("reject-file-search-context");
    fs::create_dir_all(&root).unwrap();
    let config = root.join("config.json");
    let binary = binary_path();

    let init = Command::new(&binary)
        .args(["config", "--config"])
        .arg(&config)
        .args(["init", "--non-interactive"])
        .output()
        .unwrap();
    assert!(init.status.success(), "config init command failed");
    let before = fs::read(&config).unwrap();

    let set = Command::new(&binary)
        .args(["--language", "en", "config", "--config"])
        .arg(&config)
        .args(["set", "limits.maxFileSearchContextLines", "101"])
        .output()
        .unwrap();
    assert!(!set.status.success());
    assert!(String::from_utf8_lossy(&set.stderr)
        .contains("maxFileSearchContextLines must be between 0 and 100"));
    assert_eq!(fs::read(&config).unwrap(), before);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn bare_non_tty_init_requires_explicit_non_interactive_mode() {
    for (language, marker) in [
        ("en", "Interactive config init"),
        ("zh-CN", "交互式配置初始化"),
    ] {
        let root = temp_root(&format!("non-tty-default-init-{language}"));
        fs::create_dir_all(&root).unwrap();
        let config = root.join("config.json");
        let output = Command::new(binary_path())
            .args(["--language", language, "config", "--config"])
            .arg(&config)
            .arg("init")
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "bare non-TTY init unexpectedly wrote config for {language}"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(marker),
            "bare non-TTY error was not localized for {language}: {stderr}"
        );
        assert!(
            stderr.contains("--non-interactive"),
            "bare non-TTY error did not explain the automation flag for {language}: {stderr}"
        );
        assert!(
            !config.exists(),
            "bare non-TTY init wrote a config for {language}"
        );
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn non_interactive_default_init_does_not_create_tunnel_secret_material() {
    let root = temp_root("default-init");
    fs::create_dir_all(&root).unwrap();
    let home = root.join("home");
    fs::create_dir_all(&home).unwrap();
    let config = root.join("config.json");
    let output = Command::new(binary_path())
        .env("HOME", &home)
        .args(["--language", "en", "config", "--config"])
        .arg(&config)
        .args(["init", "--non-interactive"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "default non-interactive init failed"
    );
    let value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(value["tunnel"]["tunnelId"], "tunnel_replace-me");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("replace tunnel ID"),
        "default init did not report the pending tunnel action"
    );
    let secret_dir = home.join(".agentic_gpt").join("secrets");
    let secret_file = secret_dir.join("tunnel-api-key");
    assert!(
        !secret_dir.exists(),
        "default init created a tunnel secret directory"
    );
    assert!(
        !secret_file.exists(),
        "default init created a tunnel secret file"
    );
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
    assert!(output.status.success(), "local init command failed");
    let value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert!(value.get("tunnel").is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn plaintext_tunnel_api_key_is_rejected_without_writing_config() {
    let root = temp_root("plaintext-tunnel-key");
    fs::create_dir_all(&root).unwrap();
    let home = root.join("home");
    fs::create_dir_all(&home).unwrap();
    let config = root.join("config.json");
    let secret = "plaintext-tunnel-secret-marker";
    let output = Command::new(binary_path())
        .env("HOME", &home)
        .args(["--language", "en", "config", "--config"])
        .arg(&config)
        .args(["init", "--non-interactive", "--tunnel-api-key", secret])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "plaintext tunnel key was accepted"
    );
    assert!(
        !config.exists(),
        "config was written after plaintext tunnel key rejection"
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains(secret),
        "plaintext tunnel key leaked to stdout"
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains(secret),
        "plaintext tunnel key leaked to stderr"
    );
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
    assert!(
        output.status.success(),
        "hub init command failed (child output redacted)"
    );
    let value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(value["hub"]["url"], "https://hub.example.com");
    assert_eq!(value["hub"]["transport"], "sse");
    assert_eq!(value["agentId"], "desk");
    assert!(
        value["hub"]
            .get("agentSecret")
            .and_then(Value::as_str)
            .is_some_and(|value| value == secret),
        "hub agent secret was not persisted"
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains(secret),
        "hub init stdout leaked agent secret"
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains(secret),
        "hub init stderr leaked agent secret"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn run_without_config_reports_init_first_and_does_not_create_a_file() {
    let root = temp_root("run-missing-config");
    fs::create_dir_all(&root).unwrap();
    let config = root.join("missing.json");
    let output = Command::new(binary_path())
        .args(["--language", "en", "run", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(combined.contains("run config init first"), "{combined}");
    assert!(!config.exists(), "run wrote a partial config");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn config_keys_http_mcp_section_exposes_editable_contract() {
    let output = Command::new(binary_path())
        .args([
            "--language",
            "en",
            "config",
            "keys",
            "--section",
            "http-mcp",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "HTTP MCP keys command failed");

    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let keys = value["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 6);
    for key in [
        "httpMcp.enabled",
        "httpMcp.host",
        "httpMcp.port",
        "httpMcp.publicUrl",
        "httpMcp.bearerToken",
        "httpMcp.allowHosts",
    ] {
        assert!(
            keys.iter().any(|entry| entry["key"] == key),
            "HTTP MCP key missing from registry: {key}"
        );
    }

    let enabled = keys
        .iter()
        .find(|entry| entry["key"] == "httpMcp.enabled")
        .unwrap();
    assert_eq!(enabled["type"], "boolean");
    assert_eq!(enabled["nullable"], false);
    let port = keys
        .iter()
        .find(|entry| entry["key"] == "httpMcp.port")
        .unwrap();
    assert_eq!(port["type"], "port");
    let allow_hosts = keys
        .iter()
        .find(|entry| entry["key"] == "httpMcp.allowHosts")
        .unwrap();
    assert_eq!(allow_hosts["type"], "json-string-array-or-null");
    assert_eq!(allow_hosts["nullable"], true);
}

#[test]
fn non_interactive_init_writes_http_mcp_flags_as_references_and_json() {
    let root = temp_root("http-mcp-init");
    fs::create_dir_all(&root).unwrap();
    let config = root.join("config.json");
    let token_reference = "env:CONFIG_CLI_HTTP_MCP_TOKEN";
    let output = Command::new(binary_path())
        .args(["--language", "en", "config", "--config"])
        .arg(&config)
        .args([
            "init",
            "--mode",
            "standalone",
            "--profile",
            "normal",
            "--non-interactive",
            "--http-mcp-enabled",
            "true",
            "--http-mcp-host",
            "localhost",
            "--http-mcp-port",
            "18765",
            "--http-mcp-public-url",
            "https://mcp.example.com/",
            "--http-mcp-bearer-token",
            token_reference,
            "--http-mcp-allow-hosts",
            r#"["localhost","127.0.0.1"]"#,
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "HTTP MCP init command failed");

    let value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(value["httpMcp"]["enabled"], true);
    assert_eq!(value["httpMcp"]["host"], "localhost");
    assert_eq!(value["httpMcp"]["publicUrl"], "https://mcp.example.com");
    assert_eq!(value["httpMcp"]["port"], 18765);
    assert_eq!(value["httpMcp"]["bearerToken"], token_reference);
    assert_eq!(
        value["httpMcp"]["allowHosts"],
        serde_json::json!(["localhost", "127.0.0.1"])
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains(token_reference));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(token_reference));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn config_set_http_mcp_updates_values_and_rejects_invalid_values_without_writing() {
    let root = temp_root("http-mcp-set");
    fs::create_dir_all(&root).unwrap();
    let config = root.join("config.json");
    let binary = binary_path();

    let init = Command::new(&binary)
        .args(["config", "--config"])
        .arg(&config)
        .args([
            "init",
            "--non-interactive",
            "--http-mcp-enabled",
            "true",
            "--http-mcp-bearer-token",
            "env:CONFIG_CLI_HTTP_MCP_TOKEN",
        ])
        .output()
        .unwrap();
    assert!(init.status.success(), "HTTP MCP set fixture init failed");

    for (key, value) in [
        ("httpMcp.host", "localhost"),
        ("httpMcp.publicUrl", "https://mcp.example.com/"),
        ("httpMcp.port", "18766"),
        ("httpMcp.bearerToken", "env:CONFIG_CLI_HTTP_MCP_TOKEN_NEW"),
        ("httpMcp.allowHosts", "null"),
        ("httpMcp.enabled", "false"),
    ] {
        let set = Command::new(&binary)
            .args(["config", "--config"])
            .arg(&config)
            .args(["set", key, value])
            .output()
            .unwrap();
        assert!(set.status.success(), "valid HTTP MCP set failed for {key}");
    }

    let updated: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(updated["httpMcp"]["host"], "localhost");
    assert_eq!(updated["httpMcp"]["port"], 18766);
    assert_eq!(
        updated["httpMcp"]["bearerToken"],
        "env:CONFIG_CLI_HTTP_MCP_TOKEN_NEW"
    );
    assert_eq!(updated["httpMcp"]["publicUrl"], "https://mcp.example.com");
    let clear = Command::new(&binary)
        .args(["config", "--config"])
        .arg(&config)
        .args(["set", "httpMcp.publicUrl", "null"])
        .output()
        .unwrap();
    assert!(clear.status.success(), "publicUrl null clearing failed");
    let cleared: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert!(cleared["httpMcp"]["publicUrl"].is_null());
    assert!(cleared["httpMcp"]["allowHosts"].is_null());

    let show = Command::new(&binary)
        .args(["config", "--config"])
        .arg(&config)
        .arg("show")
        .output()
        .unwrap();
    assert!(show.status.success(), "effective HTTP MCP show failed");
    let effective: Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(effective["httpMcp"]["enabled"], false);

    for (key, value) in [
        ("httpMcp.port", "0"),
        ("httpMcp.bearerToken", "literal-token-is-not-a-reference"),
        ("httpMcp.bearerToken", "file:relative-token"),
        ("httpMcp.allowHosts", "[]"),
        ("httpMcp.allowHosts", r#"["*","localhost"]"#),
        ("httpMcp.publicUrl", "http://mcp.example.com/path"),
        ("httpMcp.allowHosts", r#"["bad/path"]"#),
    ] {
        let before = fs::read(&config).unwrap();
        let set = Command::new(&binary)
            .args(["--language", "en", "config", "--config"])
            .arg(&config)
            .args(["set", key, value])
            .output()
            .unwrap();
        assert!(
            !set.status.success(),
            "invalid HTTP MCP set unexpectedly succeeded for {key}={value}"
        );
        assert_eq!(
            fs::read(&config).unwrap(),
            before,
            "invalid HTTP MCP set modified the config for {key}={value}"
        );
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
fn config_show_redacts_http_mcp_bearer_reference_but_disk_keeps_it() {
    let root = temp_root("http-mcp-show-redacted");
    fs::create_dir_all(&root).unwrap();
    let config = root.join("config.json");
    let token_reference = "file:/tmp/config-cli-http-mcp-token";

    let init = Command::new(binary_path())
        .args(["config", "--config"])
        .arg(&config)
        .args([
            "init",
            "--non-interactive",
            "--http-mcp-enabled",
            "true",
            "--http-mcp-bearer-token",
            token_reference,
        ])
        .output()
        .unwrap();
    assert!(init.status.success(), "HTTP MCP show fixture init failed");

    let disk: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(disk["httpMcp"]["bearerToken"], token_reference);

    let show = Command::new(binary_path())
        .args(["--language", "en", "config", "--config"])
        .arg(&config)
        .arg("show")
        .output()
        .unwrap();
    assert!(show.status.success(), "HTTP MCP show command failed");
    let shown: Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(shown["httpMcp"]["bearerToken"], "[REDACTED]");
    assert!(!String::from_utf8_lossy(&show.stdout).contains(token_reference));
    assert!(!String::from_utf8_lossy(&show.stderr).contains(token_reference));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn config_import_compatible_round_trip_preserves_http_mcp_fields() {
    let root = temp_root("http-mcp-import-round-trip");
    fs::create_dir_all(&root).unwrap();
    let config = root.join("config.json");
    let imported_source = root.join("import-source.json");
    let imported_config = root.join("imported-config.json");
    let token_reference = "env:CONFIG_CLI_IMPORTED_HTTP_MCP_TOKEN";
    let init = Command::new(binary_path())
        .args(["config", "--config"])
        .arg(&config)
        .args([
            "init",
            "--non-interactive",
            "--http-mcp-enabled",
            "true",
            "--http-mcp-host",
            "127.0.0.1",
            "--http-mcp-port",
            "18767",
            "--http-mcp-public-url",
            "https://import.example.com/",
            "--http-mcp-bearer-token",
            token_reference,
            "--http-mcp-allow-hosts",
            r#"["localhost","127.0.0.1"]"#,
        ])
        .output()
        .unwrap();
    assert!(init.status.success(), "HTTP MCP import fixture init failed");
    fs::copy(&config, &imported_source).unwrap();

    // The interactive `config import` wizard intentionally requires a TTY,
    // but it still parses the source before checking the terminal. Exercise
    // that stable import boundary without a fragile PTY transcript.
    let import = Command::new(binary_path())
        .args(["--language", "en", "config", "--config"])
        .arg(&imported_config)
        .args(["import"])
        .arg(&imported_source)
        .output()
        .unwrap();
    assert!(!import.status.success());
    assert!(
        !imported_config.exists(),
        "non-interactive import wrote a partial config"
    );

    // Round-trip the materialized import source through the public file loader,
    // which is the stable non-interactive boundary for this integration suite.
    let show = Command::new(binary_path())
        .args(["--language", "en", "config", "--config"])
        .arg(&imported_source)
        .arg("show")
        .output()
        .unwrap();
    assert!(
        show.status.success(),
        "import-compatible config could not be loaded"
    );
    let imported: Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(imported["httpMcp"]["enabled"], true);
    assert_eq!(imported["httpMcp"]["host"], "127.0.0.1");
    assert_eq!(imported["httpMcp"]["port"], 18767);
    assert_eq!(imported["httpMcp"]["bearerToken"], "[REDACTED]");
    assert_eq!(
        imported["httpMcp"]["publicUrl"],
        "https://import.example.com"
    );
    assert_eq!(
        imported["httpMcp"]["allowHosts"],
        serde_json::json!(["localhost", "127.0.0.1"])
    );
    let source: Value = serde_json::from_slice(&fs::read(&imported_source).unwrap()).unwrap();
    assert_eq!(source["httpMcp"]["bearerToken"], token_reference);

    let _ = fs::remove_dir_all(root);
}
