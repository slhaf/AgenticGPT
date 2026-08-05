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
    assert!(init.status.success(), "config init command failed");

    let set = Command::new(&binary)
        .args(["config", "--config"])
        .arg(&config)
        .args(["set", "room.timezone", "Asia/Tokyo"])
        .output()
        .unwrap();
    assert!(set.status.success(), "config set command failed");

    let show = Command::new(&binary)
        .args(["config", "--config"])
        .arg(&config)
        .arg("show")
        .output()
        .unwrap();
    assert!(show.status.success(), "config show command failed");
    let value: Value = serde_json::from_slice(&show.stdout).unwrap();
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
    assert!(value["keys"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| { entry["key"] == "limits.maxActiveJobs" }));
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
        .arg("init")
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
fn non_tty_init_defaults_to_standalone_normal_without_blocking() {
    let root = temp_root("non-tty-default-init");
    fs::create_dir_all(&root).unwrap();
    let config = root.join("config.json");
    let output = Command::new(binary_path())
        .args(["--language", "en", "config", "--config"])
        .arg(&config)
        .arg("init")
        .output()
        .unwrap();
    assert!(output.status.success(), "non-TTY default init failed");
    let value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(value["tunnel"]["tunnelId"], "tunnel_replace-me");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("replace tunnel ID"),
        "non-TTY default init did not report the pending tunnel action"
    );
    let _ = fs::remove_dir_all(root);
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
    assert_eq!(value["hubUrl"], "https://hub.example.com");
    assert_eq!(value["hubTransport"], "sse");
    assert_eq!(value["agentId"], "desk");
    assert!(
        value
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
