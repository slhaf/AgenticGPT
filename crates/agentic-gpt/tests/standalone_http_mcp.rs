use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};
use url::{form_urlencoded, Url};
use uuid::Uuid;

const RELOAD_WAIT: Duration = Duration::from_millis(2300);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(8);
const INITIAL_PROTOCOL_VERSION: &str = "2025-06-18";

#[test]
fn standalone_http_mcp_chatgpt_oauth_discovery_authorize_token_and_mcp_use() {
    let root = TempRoot::new("oauth").unwrap();
    if let Err(error) = run_oauth_scenario(&root.path) {
        panic!("{error}");
    }
}

#[test]
fn standalone_http_mcp_env_auth_handshake_parity_rotation_toolset_and_last_good() {
    let root = TempRoot::new("env").unwrap();
    if let Err(error) = run_env_scenario(&root.path) {
        panic!("{error}");
    }
}

#[test]
fn standalone_http_mcp_file_rotation_reconfigure_disable_and_enable() {
    let root = TempRoot::new("file").unwrap();
    if let Err(error) = run_file_scenario(&root.path) {
        panic!("{error}");
    }
}

#[test]
fn standalone_http_mcp_allow_hosts_supports_default_and_explicit_full_allow() {
    let root = TempRoot::new("allow-hosts").unwrap();
    let port = free_port().unwrap();
    let token = "http-allow-hosts-token";
    let env_name = format!("AGENTIC_HTTP_MCP_ALLOW_HOSTS_{}", Uuid::new_v4().simple());
    let environment = vec![(env_name.clone(), token.to_string())];
    let (worker, config_path) =
        spawn_worker(&root.path, port, format!("env:{env_name}"), &environment).unwrap();
    let endpoint = Endpoint::new("127.0.0.1", port);
    wait_for_tcp(&endpoint, true).unwrap();

    let authorization = format!("Bearer {token}");
    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": INITIAL_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "allow-hosts-test", "version": "1" }
        }
    });
    let rejected = http_post_with_host(
        &endpoint,
        "evil.example",
        Some(&authorization),
        None,
        initialize.clone(),
    )
    .unwrap();
    assert_eq!(rejected.status, 403);

    let mut config = read_config(&config_path).unwrap();
    config["httpMcp"]["allowHosts"] = Value::Null;
    write_config(&config_path, &config).unwrap();
    let unrestricted = wait_for_http_status_with_host(
        &endpoint,
        "evil.example",
        Some(&authorization),
        None,
        initialize.clone(),
        200,
    )
    .unwrap();
    assert!(unrestricted
        .header("content-type")
        .is_some_and(|value| value.contains("text/event-stream")));

    config["httpMcp"]["allowHosts"] = json!(["*"]);
    write_config(&config_path, &config).unwrap();
    let wildcard = wait_for_http_status_with_host(
        &endpoint,
        "another.evil.example",
        Some(&authorization),
        None,
        initialize.clone(),
        200,
    )
    .unwrap();
    assert!(wildcard
        .header("content-type")
        .is_some_and(|value| value.contains("text/event-stream")));

    config["httpMcp"]["allowHosts"] = json!([]);
    write_config(&config_path, &config).unwrap();
    thread::sleep(RELOAD_WAIT);
    let retained = wait_for_http_status_with_host(
        &endpoint,
        "still.evil.example",
        Some(&authorization),
        None,
        initialize,
        200,
    )
    .unwrap();
    assert!(retained
        .header("content-type")
        .is_some_and(|value| value.contains("text/event-stream")));
    drop(worker);
}

fn run_env_scenario(root: &Path) -> Result<(), String> {
    let initial_port = free_port()?;
    let suffix = Uuid::new_v4().simple().to_string();
    let initial_name = format!("AGENTIC_HTTP_MCP_INITIAL_{suffix}");
    let rotated_name = format!("AGENTIC_HTTP_MCP_ROTATED_{suffix}");
    let initial_token = "http-env-initial-token";
    let rotated_token = "http-env-rotated-token";
    let env = vec![
        (initial_name.clone(), initial_token.to_string()),
        (rotated_name.clone(), rotated_token.to_string()),
    ];
    let (mut worker, config_path) =
        spawn_worker(root, initial_port, format!("env:{initial_name}"), &env)?;
    let endpoint = Endpoint::new("127.0.0.1", initial_port);

    wait_for_tcp(&endpoint, true)?;
    assert_unauthorized(&endpoint, None, "missing authorization")?;
    assert_unauthorized(&endpoint, Some("Basic dGVzdA=="), "basic authorization")?;
    assert_unauthorized(
        &endpoint,
        Some("Bearer incorrect-http-token"),
        "incorrect bearer authorization",
    )?;

    let stdio_initialize = worker.stdio_initialize(1)?;
    require(
        stdio_initialize["result"]["protocolVersion"].is_string(),
        "stdio initialize did not return a protocol version",
    )?;
    worker.send(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    }))?;
    let stdio_tools = worker.stdio_tools_list(2)?;
    let stdio_tools = sorted_tools(&stdio_tools["result"]["tools"])?;

    let session_id = http_initialize(&endpoint, initial_token, 10)?;
    let (_, http_tools) = http_tools_list(&endpoint, initial_token, &session_id, 11)?;
    let http_tools = sorted_tools(&http_tools["result"]["tools"])?;
    require(
        http_tools == stdio_tools,
        "HTTP and stdio tool descriptors diverged",
    )?;
    assert_surface(&http_tools)?;

    let local_tools = wait_for_local_tools(&worker.binary, &config_path)?;
    require(
        sorted_tools(&local_tools)? == stdio_tools,
        "HTTP/stdio tool descriptors differ from local MCP descriptors",
    )?;

    let stdio_call = worker.stdio_call(
        3,
        "process.exec",
        json!({
            "command": "/usr/bin/printf 'stdio-http-parity'",
            "waitSeconds": 2
        }),
    )?;
    if stdio_call["result"]["structuredContent"]["state"] != "completed" {
        return Err(format!("stdio process.exec did not complete: {stdio_call}"));
    }
    let (_, http_result) = http_call(
        &endpoint,
        initial_token,
        &session_id,
        12,
        "process.exec",
        json!({
            "command": "/usr/bin/printf 'stdio-http-parity'",
            "waitSeconds": 2
        }),
    )?;
    if http_result["result"]["structuredContent"]["state"] != "completed" {
        return Err(format!("HTTP process.exec did not complete: {http_result}"));
    }
    let audit = fs::read_to_string(root.join("workspace/.agentic-gpt-audit.jsonl"))
        .map_err(|error| error.to_string())?;
    require(
        audit.contains("\"requestSource\":\"http:process.exec\""),
        "HTTP process.exec did not write an ingress-attributed audit record",
    )?;

    let mut config = read_config(&config_path)?;
    config["policy"]["deny"] = json!([
        { "program": "/usr/bin/printf", "argsPrefix": [] }
    ]);
    write_config(&config_path, &config)?;
    thread::sleep(RELOAD_WAIT);
    let (_, denied) = http_call(
        &endpoint,
        initial_token,
        &session_id,
        13,
        "process.exec",
        json!({
            "command": "/usr/bin/printf 'policy-must-deny'",
            "waitSeconds": 2
        }),
    )?;
    require(
        denied["result"]["structuredContent"]["error"]["code"] == "policy_denied",
        "HTTP policy reload did not deny process.exec",
    )?;

    config["policy"]["deny"] = json!([]);
    config["httpMcp"]["bearerToken"] = json!(format!("env:{rotated_name}"));
    write_config(&config_path, &config)?;
    thread::sleep(RELOAD_WAIT);

    let old_auth = format!("Bearer {initial_token}");
    let old_token_response = wait_for_http_status(
        &endpoint,
        Some(&old_auth),
        Some(&session_id),
        json!({
            "jsonrpc": "2.0",
            "id": 20,
            "method": "tools/list",
            "params": {}
        }),
        401,
    )?;
    assert_bearer_challenge(
        &old_token_response,
        None,
        "rotated HTTP endpoint did not reject the old token with Bearer challenge",
    )?;
    let new_auth = format!("Bearer {rotated_token}");
    let (_, rotated_tools) = wait_for_http_exchange(
        &endpoint,
        Some(&new_auth),
        Some(&session_id),
        json!({
            "jsonrpc": "2.0",
            "id": 21,
            "method": "tools/list",
            "params": {}
        }),
        200,
    )?;
    require(
        sorted_tools(&rotated_tools["result"]["tools"])? == stdio_tools,
        "token rotation changed the HTTP tool surface",
    )?;

    let process_disabled = config["toolsets"]["enabled"]
        .as_array()
        .ok_or_else(|| "initial toolset list is not an array".to_string())?
        .iter()
        .filter(|namespace| namespace.as_str() != Some("process"))
        .cloned()
        .collect::<Vec<_>>();
    config["toolsets"]["enabled"] = Value::Array(process_disabled);
    write_config(&config_path, &config)?;
    thread::sleep(RELOAD_WAIT);

    let (_, changed_tools) = wait_for_http_tools_presence(
        &endpoint,
        rotated_token,
        &session_id,
        30,
        "process.exec",
        false,
    )?;
    require(
        !tool_names(&changed_tools["result"]["tools"])
            .iter()
            .any(|name| name == "process.exec"),
        "disabled process tool remained advertised over HTTP",
    )?;
    let (_, unavailable_call) = http_call(
        &endpoint,
        rotated_token,
        &session_id,
        31,
        "process.exec",
        json!({
            "command": "/usr/bin/printf 'must-not-run'",
            "waitSeconds": 2
        }),
    )?;
    require(
        unavailable_call["error"].is_object(),
        "disabled HTTP tool call did not return a JSON-RPC error",
    )?;

    config["toolsets"]["enabled"] = json!(["agent", "file", "mcp", "process", "skills", "tmux"]);
    write_config(&config_path, &config)?;
    thread::sleep(RELOAD_WAIT);
    let (_, restored_tools) = wait_for_http_tools_presence(
        &endpoint,
        rotated_token,
        &session_id,
        32,
        "process.exec",
        true,
    )?;
    require(
        tool_names(&restored_tools["result"]["tools"])
            .iter()
            .any(|name| name == "process.exec"),
        "re-enabled process tool was not advertised over HTTP",
    )?;

    config["httpMcp"]["port"] = json!(0);
    write_config(&config_path, &config)?;
    thread::sleep(RELOAD_WAIT);
    let (_, retained) = wait_for_http_exchange(
        &endpoint,
        Some(&new_auth),
        Some(&session_id),
        json!({
            "jsonrpc": "2.0",
            "id": 40,
            "method": "tools/list",
            "params": {}
        }),
        200,
    )?;
    require(
        tool_names(&retained["result"]["tools"])
            .iter()
            .any(|name| name == "process.exec"),
        "invalid endpoint reload did not retain the last-good HTTP listener and toolset",
    )?;
    drop(worker);
    wait_for_tcp(&endpoint, false)?;
    Ok(())
}

fn run_file_scenario(root: &Path) -> Result<(), String> {
    let initial_port = free_port()?;
    let replacement_port = loop {
        let candidate = free_port()?;
        if candidate != initial_port {
            break candidate;
        }
    };
    let initial_file = root.join("http-token-initial");
    let replacement_file = root.join("http-token-replacement");
    fs::write(&initial_file, "http-file-initial-token\r\n").map_err(|error| error.to_string())?;
    fs::write(&replacement_file, "http-file-rotated-token\n").map_err(|error| error.to_string())?;

    let (worker, config_path) = spawn_worker(
        root,
        initial_port,
        format!("file:{}", initial_file.display()),
        &[],
    )?;
    let initial_endpoint = Endpoint::new("127.0.0.1", initial_port);
    let replacement_endpoint = Endpoint::new("127.0.0.1", replacement_port);
    wait_for_tcp(&initial_endpoint, true)?;
    let session_id = http_initialize(&initial_endpoint, "http-file-initial-token", 100)?;
    let (_, initial_tools) = http_tools_list(
        &initial_endpoint,
        "http-file-initial-token",
        &session_id,
        101,
    )?;
    require(
        initial_tools["result"]["tools"]
            .as_array()
            .is_some_and(|tools| !tools.is_empty()),
        "file-token HTTP initialize returned no tools",
    )?;

    let mut config = read_config(&config_path)?;
    config["httpMcp"]["bearerToken"] = json!(format!("file:{}", replacement_file.display()));
    write_config(&config_path, &config)?;
    thread::sleep(RELOAD_WAIT);
    let old_auth = format!("Bearer {}", "http-file-initial-token");
    let old_token_response = wait_for_http_status(
        &initial_endpoint,
        Some(&old_auth),
        Some(&session_id),
        json!({
            "jsonrpc": "2.0",
            "id": 102,
            "method": "tools/list",
            "params": {}
        }),
        401,
    )?;
    assert_bearer_challenge(
        &old_token_response,
        None,
        "file token rotation did not reject the old resolved token with Bearer challenge",
    )?;
    let (_, _) = wait_for_http_exchange(
        &initial_endpoint,
        Some("Bearer http-file-rotated-token"),
        Some(&session_id),
        json!({
            "jsonrpc": "2.0",
            "id": 103,
            "method": "tools/list",
            "params": {}
        }),
        200,
    )?;

    config["httpMcp"]["host"] = json!("0.0.0.0");
    config["httpMcp"]["port"] = json!(replacement_port);
    write_config(&config_path, &config)?;
    thread::sleep(RELOAD_WAIT);
    wait_for_tcp(&replacement_endpoint, true)?;
    wait_for_tcp(&initial_endpoint, false)?;
    let stale_session = wait_for_http_status(
        &replacement_endpoint,
        Some("Bearer http-file-rotated-token"),
        Some(&session_id),
        json!({
            "jsonrpc": "2.0",
            "id": 104,
            "method": "tools/list",
            "params": {}
        }),
        404,
    )?;
    require(
        stale_session.status == 404,
        "host/port reconfigure unexpectedly preserved the old HTTP session",
    )?;
    let replacement_session =
        http_initialize(&replacement_endpoint, "http-file-rotated-token", 105)?;

    config["httpMcp"]["enabled"] = json!(false);
    write_config(&config_path, &config)?;
    thread::sleep(RELOAD_WAIT);
    wait_for_tcp(&replacement_endpoint, false)?;

    config["httpMcp"]["enabled"] = json!(true);
    write_config(&config_path, &config)?;
    thread::sleep(RELOAD_WAIT);
    wait_for_tcp(&replacement_endpoint, true)?;
    let stale_after_enable = wait_for_http_status(
        &replacement_endpoint,
        Some("Bearer http-file-rotated-token"),
        Some(&replacement_session),
        json!({
            "jsonrpc": "2.0",
            "id": 106,
            "method": "tools/list",
            "params": {}
        }),
        404,
    )?;
    require(
        stale_after_enable.status == 404,
        "disable/enable did not require a fresh HTTP session",
    )?;
    let fresh_session = http_initialize(&replacement_endpoint, "http-file-rotated-token", 107)?;
    let (_, fresh_tools) = http_tools_list(
        &replacement_endpoint,
        "http-file-rotated-token",
        &fresh_session,
        108,
    )?;
    require(
        fresh_tools["result"]["tools"]
            .as_array()
            .is_some_and(|tools| !tools.is_empty()),
        "re-enabled file-token HTTP endpoint returned no tools",
    )?;

    drop(worker);
    wait_for_tcp(&replacement_endpoint, false)?;
    Ok(())
}

fn run_oauth_scenario(root: &Path) -> Result<(), String> {
    let port = free_port()?;
    let suffix = Uuid::new_v4().simple().to_string();
    let env_name = format!("AGENTIC_HTTP_MCP_OAUTH_{suffix}");
    let rotated_name = format!("AGENTIC_HTTP_MCP_OAUTH_ROTATED_{suffix}");
    let initial_token = "http-oauth-initial-token";
    let rotated_token = "http-oauth-rotated-token";
    let environment = vec![
        (env_name.clone(), initial_token.to_string()),
        (rotated_name.clone(), rotated_token.to_string()),
    ];
    let public_url = format!("https://127.0.0.1:{port}");
    let (mut worker, config_path) =
        spawn_oauth_worker(root, port, format!("env:{env_name}"), &environment)?;
    let endpoint = Endpoint::new("127.0.0.1", port);
    let host = endpoint.address();
    let resource = format!("{public_url}/mcp");
    wait_for_tcp(&endpoint, true)?;

    let unauthorized = http_post_with_host(
        &endpoint,
        &host,
        None,
        None,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": INITIAL_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "oauth-test", "version": "1" }
            }
        }),
    )?;
    require(
        unauthorized.status == 401,
        "OAuth MCP request was not unauthorized",
    )?;
    assert_bearer_challenge(
        &unauthorized,
        Some(&format!(
            "{public_url}/.well-known/oauth-protected-resource/mcp"
        )),
        "OAuth MCP challenge",
    )?;

    for path in [
        "/.well-known/oauth-protected-resource/mcp",
        "/.well-known/oauth-protected-resource",
    ] {
        let response = http_get(&endpoint, path, Some(&host), &[])?;
        require(
            response.status == 200,
            "protected-resource discovery failed",
        )?;
        let metadata: Value =
            serde_json::from_str(&response.body).map_err(|error| error.to_string())?;
        require(
            metadata["resource"] == resource
                && metadata["authorization_servers"][0] == public_url
                && metadata["scopes_supported"][0] == "agentic:mcp"
                && metadata["bearer_methods_supported"][0] == "header"
                && metadata["resource_documentation"] == resource
                && metadata.get("mcp_profile").is_none(),
            "protected-resource metadata was not standalone",
        )?;
    }
    for path in [
        "/.well-known/oauth-authorization-server",
        "/.well-known/openid-configuration",
    ] {
        let response = http_get(&endpoint, path, Some(&host), &[])?;
        require(
            response.status == 200,
            "authorization-server discovery failed",
        )?;
        let metadata: Value =
            serde_json::from_str(&response.body).map_err(|error| error.to_string())?;
        require(
            metadata["issuer"] == public_url
                && metadata["authorization_endpoint"] == format!("{public_url}/oauth/authorize")
                && metadata["token_endpoint"] == format!("{public_url}/oauth/token")
                && metadata["scopes_supported"][0] == "agentic:mcp"
                && metadata.get("mcp_profile").is_none(),
            "authorization-server metadata was not standalone",
        )?;
    }

    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let challenge = pkce_challenge(verifier);
    require(
        challenge == "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
        "PKCE challenge did not match RFC 7636",
    )?;
    let redirect_uri = "https://chatgpt.com/connector/oauth/oauth-test";
    let client_id = "client<&\"";
    let state = "state<&\"";
    let authorize_path = oauth_authorize_path(
        client_id,
        redirect_uri,
        state,
        "agentic:mcp",
        &challenge,
        "S256",
        &resource,
    );
    let page = http_get(&endpoint, &authorize_path, Some(&host), &[])?;
    require(
        page.status == 200 && page.header("cache-control") == Some("no-store"),
        "valid authorize page did not render with no-store caching",
    )?;
    require(
        page.body.contains("HTTP MCP bearer token")
            && page.body.contains("name=\"bearer_token\"")
            && page.body.contains("&lt;")
            && page.body.contains("&amp;")
            && page.body.contains("&quot;"),
        "authorize page omitted standalone escaped fields",
    )?;
    require(
        !page.body.contains("Hub API key")
            && !page.body.contains("API key")
            && !page.body.to_ascii_lowercase().contains("secret"),
        "authorize page leaked Hub/API-key/secret wording",
    )?;

    for bad_redirect in [
        "https://chatgpt.com/connector/oauth/",
        "http://chatgpt.com/connector/oauth/oauth-test",
        "https://evil.example/connector/oauth/oauth-test",
        "not a URI",
    ] {
        let path = oauth_authorize_path(
            client_id,
            bad_redirect,
            state,
            "agentic:mcp",
            &challenge,
            "S256",
            &resource,
        );
        let response = http_get(&endpoint, &path, Some(&host), &[])?;
        require(
            response.header("location").is_none()
                && response.body.to_ascii_lowercase().contains("error"),
            "invalid redirect was not rendered as a local authorize error",
        )?;
    }
    let wrong_method = http_get(
        &endpoint,
        &oauth_authorize_path(
            client_id,
            redirect_uri,
            state,
            "agentic:mcp",
            &challenge,
            "plain",
            &resource,
        ),
        Some(&host),
        &[],
    )?;
    require(
        wrong_method.header("location").is_none()
            && wrong_method.body.to_ascii_lowercase().contains("error"),
        "unsupported PKCE method was not rejected",
    )?;
    let wrong_scope = http_get(
        &endpoint,
        &oauth_authorize_path(
            client_id,
            redirect_uri,
            state,
            "other",
            &challenge,
            "S256",
            &resource,
        ),
        Some(&host),
        &[],
    )?;
    require(
        wrong_scope.header("location").is_none()
            && wrong_scope
                .body
                .to_ascii_lowercase()
                .contains("invalid_scope"),
        "unsupported OAuth scope was not rejected",
    )?;

    let wrong_form = oauth_authorize_form(
        &endpoint,
        &host,
        OAuthFormRequest {
            authorization: OAuthAuthorization {
                client_id,
                redirect_uri,
                state,
                resource: &resource,
            },
            scope: "agentic:mcp",
            code_challenge: &challenge,
            code_challenge_method: "S256",
        },
        "wrong-bearer",
    )?;
    require(
        wrong_form.status == 200
            && wrong_form.header("location").is_none()
            && wrong_form.body.contains("HTTP MCP bearer token")
            && !wrong_form.body.contains("code="),
        "wrong bearer did not render a retry page",
    )?;
    let submitted = oauth_authorize_form(
        &endpoint,
        &host,
        OAuthFormRequest {
            authorization: OAuthAuthorization {
                client_id,
                redirect_uri,
                state,
                resource: &resource,
            },
            scope: "agentic:mcp",
            code_challenge: &challenge,
            code_challenge_method: "S256",
        },
        initial_token,
    )?;
    require(
        submitted.status == 303,
        "authorize submission did not redirect",
    )?;
    let location = submitted
        .header("location")
        .ok_or_else(|| "authorize submission omitted Location".to_string())?;
    let (code, returned_state) = oauth_code_and_state(location)?;
    let callback = Url::parse(location).map_err(|error| error.to_string())?;
    require(
        callback.scheme() == "https"
            && callback.host_str() == Some("chatgpt.com")
            && callback.path() == "/connector/oauth/oauth-test",
        "authorize submission redirected to an unexpected callback",
    )?;
    require(
        returned_state == state,
        "authorize redirect did not preserve state byte-for-byte",
    )?;

    let token_response = oauth_token(
        &endpoint,
        &host,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("code_verifier", verifier),
            ("resource", &resource),
        ],
        &[],
    )?;
    require(
        token_response.status == 200
            && token_response.header("cache-control") == Some("no-store")
            && token_response.header("pragma") == Some("no-cache"),
        "token response did not have required success headers",
    )?;
    let token_json: Value =
        serde_json::from_str(&token_response.body).map_err(|error| error.to_string())?;
    let access_token = token_json["access_token"]
        .as_str()
        .ok_or_else(|| "token response omitted access_token".to_string())?
        .to_string();
    require(
        token_json["token_type"] == "Bearer"
            && token_json["scope"] == "agentic:mcp"
            && token_json["expires_in"].as_i64().unwrap_or_default() >= 7 * 24 * 60 * 60
            && access_token != initial_token
            && !token_response.body.contains(initial_token),
        "token response did not contain an opaque seven-day Bearer token",
    )?;

    let stdio_initialize = worker.stdio_initialize(2)?;
    require(
        stdio_initialize["result"]["protocolVersion"].is_string(),
        "stdio initialize did not return a protocol version",
    )?;
    worker.send(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    }))?;
    let stdio_tools = sorted_tools(&worker.stdio_tools_list(3)?["result"]["tools"])?;
    let direct_session = http_initialize(&endpoint, initial_token, 10)?;
    let (_, direct_message) = http_tools_list(&endpoint, initial_token, &direct_session, 11)?;
    let direct_tools = sorted_tools(&direct_message["result"]["tools"])?;
    let oauth_session = http_initialize(&endpoint, &access_token, 12)?;
    let (_, oauth_message) = http_tools_list(&endpoint, &access_token, &oauth_session, 13)?;
    let oauth_tools = sorted_tools(&oauth_message["result"]["tools"])?;
    require(
        oauth_tools == direct_tools && oauth_tools == stdio_tools,
        "OAuth MCP tool descriptors diverged from direct bearer/stdio",
    )?;
    let local_tools = wait_for_local_tools(&worker.binary, &config_path)?;
    require(
        oauth_tools == sorted_tools(&local_tools)?,
        "OAuth MCP tool descriptors diverged from Unix/local tools",
    )?;

    let wrong_client_code = oauth_issue_code(
        &endpoint,
        &host,
        OAuthAuthorization {
            client_id,
            redirect_uri,
            state: "wrong-client",
            resource: &resource,
        },
        verifier,
        initial_token,
    )?;
    assert_oauth_error(
        &oauth_token(
            &endpoint,
            &host,
            &[
                ("grant_type", "authorization_code"),
                ("code", &wrong_client_code),
                ("client_id", "different-client"),
                ("redirect_uri", redirect_uri),
                ("code_verifier", verifier),
                ("resource", &resource),
            ],
            &[],
        )?,
        "invalid_grant",
        "wrong client",
    )?;
    let wrong_redirect_code = oauth_issue_code(
        &endpoint,
        &host,
        OAuthAuthorization {
            client_id,
            redirect_uri,
            state: "wrong-redirect",
            resource: &resource,
        },
        verifier,
        initial_token,
    )?;
    assert_oauth_error(
        &oauth_token(
            &endpoint,
            &host,
            &[
                ("grant_type", "authorization_code"),
                ("code", &wrong_redirect_code),
                ("client_id", client_id),
                ("redirect_uri", "https://chatgpt.com/connector/oauth/other"),
                ("code_verifier", verifier),
                ("resource", &resource),
            ],
            &[],
        )?,
        "invalid_grant",
        "redirect mismatch",
    )?;
    let wrong_resource_code = oauth_issue_code(
        &endpoint,
        &host,
        OAuthAuthorization {
            client_id,
            redirect_uri,
            state: "wrong-resource",
            resource: &resource,
        },
        verifier,
        initial_token,
    )?;
    assert_oauth_error(
        &oauth_token(
            &endpoint,
            &host,
            &[
                ("grant_type", "authorization_code"),
                ("code", &wrong_resource_code),
                ("client_id", client_id),
                ("redirect_uri", redirect_uri),
                ("code_verifier", verifier),
                ("resource", "https://evil.example/mcp"),
            ],
            &[],
        )?,
        "invalid_target",
        "resource mismatch",
    )?;
    let wrong_token_scope_code = oauth_issue_code(
        &endpoint,
        &host,
        OAuthAuthorization {
            client_id,
            redirect_uri,
            state: "wrong-token-scope",
            resource: &resource,
        },
        verifier,
        initial_token,
    )?;
    assert_oauth_error(
        &oauth_token(
            &endpoint,
            &host,
            &[
                ("grant_type", "authorization_code"),
                ("code", &wrong_token_scope_code),
                ("client_id", client_id),
                ("redirect_uri", redirect_uri),
                ("code_verifier", verifier),
                ("resource", &resource),
                ("scope", "other"),
            ],
            &[],
        )?,
        "invalid_scope",
        "token scope mismatch",
    )?;
    let wrong_verifier_code = oauth_issue_code(
        &endpoint,
        &host,
        OAuthAuthorization {
            client_id,
            redirect_uri,
            state: "wrong-verifier",
            resource: &resource,
        },
        verifier,
        initial_token,
    )?;
    assert_oauth_error(
        &oauth_token(
            &endpoint,
            &host,
            &[
                ("grant_type", "authorization_code"),
                ("code", &wrong_verifier_code),
                ("client_id", client_id),
                ("redirect_uri", redirect_uri),
                ("code_verifier", "wrong-verifier"),
                ("resource", &resource),
            ],
            &[],
        )?,
        "invalid_grant",
        "PKCE mismatch",
    )?;
    assert_oauth_error(
        &oauth_token(
            &endpoint,
            &host,
            &[
                ("grant_type", "authorization_code"),
                ("code", &wrong_verifier_code),
                ("client_id", client_id),
                ("redirect_uri", redirect_uri),
                ("code_verifier", verifier),
                ("resource", &resource),
            ],
            &[],
        )?,
        "invalid_grant",
        "consumed code",
    )?;
    assert_oauth_error(
        &oauth_token(&endpoint, &host, &[("grant_type", "refresh_token")], &[])?,
        "unsupported_grant_type",
        "unsupported grant",
    )?;
    assert_oauth_error(
        &oauth_token(
            &endpoint,
            &host,
            &[("grant_type", "authorization_code")],
            &[],
        )?,
        "invalid_request",
        "missing code",
    )?;
    assert_oauth_error(
        &oauth_token(
            &endpoint,
            &host,
            &[("grant_type", "authorization_code"), ("code", "unused")],
            &[],
        )?,
        "invalid_request",
        "missing verifier",
    )?;
    assert_oauth_error(
        &oauth_token(
            &endpoint,
            &host,
            &[
                ("grant_type", "authorization_code"),
                ("code", "unused"),
                ("code_verifier", verifier),
            ],
            &[],
        )?,
        "invalid_request",
        "missing redirect",
    )?;
    assert_oauth_error(
        &oauth_token(
            &endpoint,
            &host,
            &[
                ("grant_type", "authorization_code"),
                ("code", "unused"),
                ("code_verifier", verifier),
                ("redirect_uri", redirect_uri),
            ],
            &[],
        )?,
        "invalid_request",
        "missing client",
    )?;
    let reuse_code = oauth_issue_code(
        &endpoint,
        &host,
        OAuthAuthorization {
            client_id,
            redirect_uri,
            state: "reuse",
            resource: &resource,
        },
        verifier,
        initial_token,
    )?;
    let first_reuse = oauth_token(
        &endpoint,
        &host,
        &[
            ("grant_type", "authorization_code"),
            ("code", &reuse_code),
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("code_verifier", verifier),
            ("resource", &resource),
        ],
        &[],
    )?;
    require(first_reuse.status == 200, "first code exchange failed")?;
    assert_oauth_error(
        &oauth_token(
            &endpoint,
            &host,
            &[
                ("grant_type", "authorization_code"),
                ("code", &reuse_code),
                ("client_id", client_id),
                ("redirect_uri", redirect_uri),
                ("code_verifier", verifier),
                ("resource", &resource),
            ],
            &[],
        )?,
        "invalid_grant",
        "code reuse",
    )?;
    assert_oauth_error(
        &oauth_token(
            &endpoint,
            &host,
            &[
                ("grant_type", "authorization_code"),
                ("client_id", client_id),
                ("redirect_uri", redirect_uri),
                ("code_verifier", verifier),
            ],
            &[],
        )?,
        "invalid_request",
        "missing code field",
    )?;

    let rotation_code = oauth_issue_code(
        &endpoint,
        &host,
        OAuthAuthorization {
            client_id,
            redirect_uri,
            state: "rotation",
            resource: &resource,
        },
        verifier,
        initial_token,
    )?;
    let mut config = read_config(&config_path)?;
    config["httpMcp"]["bearerToken"] = json!(format!("env:{rotated_name}"));
    write_config(&config_path, &config)?;
    thread::sleep(RELOAD_WAIT);
    let old_direct = http_post_with_host(
        &endpoint,
        &host,
        Some(&format!("Bearer {initial_token}")),
        Some(&direct_session),
        json!({
            "jsonrpc": "2.0",
            "id": 20,
            "method": "tools/list",
            "params": {}
        }),
    )?;
    require(
        old_direct.status == 401,
        "old direct bearer survived rotation",
    )?;
    assert_bearer_challenge(
        &old_direct,
        Some(&format!(
            "{public_url}/.well-known/oauth-protected-resource/mcp"
        )),
        "rotated direct challenge",
    )?;
    let (_, new_direct_tools) = http_tools_list(&endpoint, rotated_token, &direct_session, 21)?;
    require(
        sorted_tools(&new_direct_tools["result"]["tools"])? == direct_tools,
        "new direct bearer did not preserve the existing MCP session",
    )?;
    let old_oauth = http_post_with_host(
        &endpoint,
        &host,
        Some(&format!("Bearer {access_token}")),
        Some(&oauth_session),
        json!({
            "jsonrpc": "2.0",
            "id": 22,
            "method": "tools/list",
            "params": {}
        }),
    )?;
    require(
        old_oauth.status == 401,
        "OAuth token survived direct bearer rotation",
    )?;
    assert_oauth_error(
        &oauth_token(
            &endpoint,
            &host,
            &[
                ("grant_type", "authorization_code"),
                ("code", &rotation_code),
                ("client_id", client_id),
                ("redirect_uri", redirect_uri),
                ("code_verifier", verifier),
                ("resource", &resource),
            ],
            &[],
        )?,
        "invalid_grant",
        "rotated code",
    )?;

    let audience_code = oauth_issue_code(
        &endpoint,
        &host,
        OAuthAuthorization {
            client_id,
            redirect_uri,
            state: "audience",
            resource: &resource,
        },
        verifier,
        rotated_token,
    )?;
    assert_oauth_error(
        &oauth_token(
            &endpoint,
            &host,
            &[
                ("grant_type", "authorization_code"),
                ("code", &audience_code),
                ("client_id", client_id),
                ("redirect_uri", redirect_uri),
                ("code_verifier", verifier),
                ("resource", "https://evil.example/mcp"),
            ],
            &[],
        )?,
        "invalid_target",
        "audience mismatch",
    )?;
    let random_bearer = http_post_with_host(
        &endpoint,
        &host,
        Some("Bearer audience-mismatch-token"),
        None,
        json!({
            "jsonrpc": "2.0",
            "id": 23,
            "method": "initialize",
            "params": {
                "protocolVersion": INITIAL_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "oauth-test", "version": "1" }
            }
        }),
    )?;
    require(
        random_bearer.status == 401,
        "resource-mismatched bearer authorized MCP",
    )?;

    for path in [
        "/.well-known/oauth-protected-resource/mcp",
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-authorization-server",
        "/.well-known/openid-configuration",
        "/oauth/authorize",
        "/oauth/token",
    ] {
        let response = http_get(&endpoint, path, Some("evil.example"), &[])?;
        require(
            response.status == 403,
            "disallowed Host reached a public OAuth route",
        )?;
    }
    let missing_host = http_get(
        &endpoint,
        "/.well-known/oauth-protected-resource",
        None,
        &[],
    )?;
    require(missing_host.status == 400, "missing Host was not rejected")?;
    let same_origin = http_get(
        &endpoint,
        "/.well-known/oauth-protected-resource",
        Some(&host),
        &[("Origin", &public_url)],
    )?;
    require(
        same_origin.status == 200,
        "configured public Origin was rejected",
    )?;
    let malformed_host = http_get(
        &endpoint,
        "/.well-known/oauth-protected-resource",
        Some(":bad"),
        &[],
    )?;
    require(
        malformed_host.status == 400,
        "malformed Host was not rejected",
    )?;
    let foreign_origin = http_get(
        &endpoint,
        "/.well-known/oauth-protected-resource",
        Some(&host),
        &[("Origin", "https://evil.example")],
    )?;
    require(foreign_origin.status == 403, "foreign Origin was accepted")?;
    let no_origin = http_get(
        &endpoint,
        "/.well-known/oauth-protected-resource",
        Some(&host),
        &[],
    )?;
    require(no_origin.status == 200, "missing Origin was rejected")?;

    config = read_config(&config_path)?;
    config["httpMcp"]["publicUrl"] = json!("http://invalid.example");
    write_config(&config_path, &config)?;
    thread::sleep(RELOAD_WAIT);
    let retained_metadata = http_get(
        &endpoint,
        "/.well-known/oauth-protected-resource/mcp",
        Some(&host),
        &[],
    )?;
    require(
        retained_metadata.status == 200
            && serde_json::from_str::<Value>(&retained_metadata.body)
                .map(|value| value["resource"] == resource)
                .unwrap_or(false),
        "invalid publicUrl candidate did not retain the last-good listener",
    )?;
    config["httpMcp"]["publicUrl"] = json!(public_url);
    config["httpMcp"]["allowHosts"] = Value::Null;
    write_config(&config_path, &config)?;
    thread::sleep(RELOAD_WAIT);
    let unrestricted = http_get(
        &endpoint,
        "/.well-known/oauth-protected-resource",
        Some("evil.example"),
        &[],
    )?;
    require(
        unrestricted.status == 200,
        "null allowHosts did not allow a public OAuth route",
    )?;
    config["httpMcp"]["allowHosts"] = json!(["*"]);
    write_config(&config_path, &config)?;
    thread::sleep(RELOAD_WAIT);
    let wildcard = http_get(
        &endpoint,
        "/.well-known/oauth-protected-resource",
        Some("another.evil.example"),
        &[],
    )?;
    require(
        wildcard.status == 200,
        "wildcard allowHosts did not allow a public OAuth route",
    )?;
    drop(worker);
    wait_for_tcp(&endpoint, false)?;
    Ok(())
}

struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    fn new(label: &str) -> Result<Self, String> {
        let path =
            std::env::temp_dir().join(format!("agentic-http-mcp-{label}-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        Ok(Self { path })
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct WorkerFixture {
    child: Child,
    stdin: Option<std::process::ChildStdin>,
    stdout: Receiver<String>,
    stdout_thread: Option<thread::JoinHandle<()>>,
    stderr_thread: Option<thread::JoinHandle<String>>,
    binary: PathBuf,
    socket_path: PathBuf,
}

impl WorkerFixture {
    fn stdin_mut(&mut self) -> Result<&mut std::process::ChildStdin, String> {
        self.stdin
            .as_mut()
            .ok_or("worker stdin unavailable".to_string())
    }

    fn send(&mut self, value: Value) -> Result<(), String> {
        send_line(self.stdin_mut()?, value)
    }

    fn stdio_initialize(&mut self, id: i64) -> Result<Value, String> {
        self.send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": INITIAL_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "standalone-http-mcp-test", "version": "1" }
            }
        }))?;
        response_for(&self.stdout, id)
    }

    fn stdio_tools_list(&mut self, id: i64) -> Result<Value, String> {
        self.send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/list",
            "params": {}
        }))?;
        response_for(&self.stdout, id)
    }

    fn stdio_call(&mut self, id: i64, name: &str, arguments: Value) -> Result<Value, String> {
        self.send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }))?;
        response_for(&self.stdout, id)
    }
}

impl Drop for WorkerFixture {
    fn drop(&mut self) {
        self.stdin.take();
        stop_child_gracefully(&mut self.child, Duration::from_secs(5));
        if let Some(thread) = self.stdout_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.stderr_thread.take() {
            let _ = thread.join();
        }
        if wait_for_path_absent(&self.socket_path, Duration::from_secs(2)).is_ok() {
            if let Some(parent) = self.socket_path.parent() {
                let _ = fs::remove_dir_all(parent);
            }
        }
    }
}

fn spawn_worker(
    root: &Path,
    port: u16,
    bearer_reference: String,
    environment: &[(String, String)],
) -> Result<(WorkerFixture, PathBuf), String> {
    spawn_worker_with_public_url(root, port, bearer_reference, environment, None)
}

fn spawn_oauth_worker(
    root: &Path,
    port: u16,
    bearer_reference: String,
    environment: &[(String, String)],
) -> Result<(WorkerFixture, PathBuf), String> {
    let public_url = format!("https://127.0.0.1:{port}");
    spawn_worker_with_public_url(root, port, bearer_reference, environment, Some(&public_url))
}

fn spawn_worker_with_public_url(
    root: &Path,
    port: u16,
    bearer_reference: String,
    environment: &[(String, String)],
    public_url: Option<&str>,
) -> Result<(WorkerFixture, PathBuf), String> {
    let binary = binary_path();
    if !binary.exists() {
        return Err(format!("agentic binary not found: {}", binary.display()));
    }
    let config_path = root.join("config.json");
    let workspace = root.join("workspace");
    let init = Command::new(&binary)
        .args([
            "config",
            "--config",
            config_path
                .to_str()
                .ok_or_else(|| "config path is not UTF-8".to_string())?,
            "init",
            "--non-interactive",
        ])
        .output()
        .map_err(|error| format!("config init failed to spawn: {error}"))?;
    if !init.status.success() {
        return Err(format!(
            "config init failed: {}",
            String::from_utf8_lossy(&init.stderr)
        ));
    }
    fs::create_dir_all(&workspace).map_err(|error| error.to_string())?;
    let agent_id = format!("http-mcp-{}", Uuid::new_v4().simple());
    let mut config: Value =
        serde_json::from_slice(&fs::read(&config_path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    config["mode"] = json!("standalone");
    config["profile"] = json!("normal");
    config["agentId"] = json!(agent_id.clone());
    config["workspaceRoot"] = json!(workspace.to_string_lossy().into_owned());
    config["shell"]["initFile"] = Value::Null;
    config["policy"]["allow"] = json!([
        { "program": "/usr/bin/printf", "argsPrefix": [] }
    ]);
    config["toolsets"]["enabled"] = json!(["agent", "file", "mcp", "process", "skills", "tmux"]);
    config["tunnel"] = json!({
        "tunnelId": "tunnel_http_mcp_integration",
        "apiKey": "env:AGENTIC_HTTP_MCP_TUNNEL_KEY",
        "hubReporting": { "enabled": false, "detail": "metadata" }
    });
    let allow_hosts = public_url
        .map(|_| json!([format!("127.0.0.1:{port}")]))
        .unwrap_or_else(|| json!(["localhost", "127.0.0.1", "::1"]));
    let public_url_value = public_url
        .map(|value| Value::String(value.to_string()))
        .unwrap_or(Value::Null);
    config["httpMcp"] = json!({
        "enabled": true,
        "host": "127.0.0.1",
        "port": port,
        "publicUrl": public_url_value,
        "bearerToken": bearer_reference,
        "allowHosts": allow_hosts
    });
    write_config(&config_path, &config)?;

    let supervisor_token = format!("supervisor-http-mcp-{}", Uuid::new_v4().simple());
    let mut command = Command::new(&binary);
    command
        .args([
            "stdio-worker",
            "--config",
            config_path
                .to_str()
                .ok_or_else(|| "config path is not UTF-8".to_string())?,
            "--profile",
            "normal",
            "--supervisor-token",
            &supervisor_token,
        ])
        .env("AGENTIC_GPT_SUPERVISOR_TOKEN", &supervisor_token)
        .env("AGENTIC_HTTP_MCP_TUNNEL_KEY", "unused-test-tunnel-key")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in environment {
        command.env(name, value);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("worker failed to spawn: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "worker stdout unavailable".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "worker stderr unavailable".to_string())?;
    let (stdout_tx, stdout_rx) = mpsc::channel();
    let stdout_thread = thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if stdout_tx.send(line).is_err() {
                break;
            }
        }
    });
    let stderr_thread = thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut logs = String::new();
        let _ = reader.read_to_string(&mut logs);
        logs
    });
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "worker stdin unavailable".to_string())?;
    let socket_path = local_socket_path(&agent_id)?;
    Ok((
        WorkerFixture {
            child,
            stdin: Some(stdin),
            stdout: stdout_rx,
            stdout_thread: Some(stdout_thread),
            stderr_thread: Some(stderr_thread),
            binary,
            socket_path,
        },
        config_path,
    ))
}

fn binary_path() -> PathBuf {
    std::env::var("CARGO_BIN_EXE_agentic-gpt")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_agentic_gpt"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/agentic-gpt")
        })
}

#[derive(Clone)]
struct Endpoint {
    host: String,
    port: u16,
}

impl Endpoint {
    fn new(host: &str, port: u16) -> Self {
        Self {
            host: host.to_string(),
            port,
        }
    }

    fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn oauth_authorize_path(
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    scope: &str,
    code_challenge: &str,
    code_challenge_method: &str,
    resource: &str,
) -> String {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    serializer
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("state", state)
        .append_pair("scope", scope)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", code_challenge_method)
        .append_pair("resource", resource);
    format!("/oauth/authorize?{}", serializer.finish())
}

fn form_body(fields: &[(&str, &str)]) -> String {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (name, value) in fields {
        serializer.append_pair(name, value);
    }
    serializer.finish()
}

struct OAuthAuthorization<'a> {
    client_id: &'a str,
    redirect_uri: &'a str,
    state: &'a str,
    resource: &'a str,
}

struct OAuthFormRequest<'a> {
    authorization: OAuthAuthorization<'a>,
    scope: &'a str,
    code_challenge: &'a str,
    code_challenge_method: &'a str,
}

fn oauth_authorize_form(
    endpoint: &Endpoint,
    host: &str,
    request: OAuthFormRequest<'_>,
    bearer_token: &str,
) -> Result<HttpResponse, String> {
    let OAuthFormRequest {
        authorization:
            OAuthAuthorization {
                client_id,
                redirect_uri,
                state,
                resource,
            },
        scope,
        code_challenge,
        code_challenge_method,
    } = request;
    let body = form_body(&[
        ("response_type", "code"),
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("state", state),
        ("scope", scope),
        ("code_challenge", code_challenge),
        ("code_challenge_method", code_challenge_method),
        ("resource", resource),
        ("bearer_token", bearer_token),
    ]);
    http_form_post(endpoint, "/oauth/authorize", Some(host), &body, &[])
}

fn oauth_issue_code(
    endpoint: &Endpoint,
    host: &str,
    authorization: OAuthAuthorization<'_>,
    verifier: &str,
    bearer_token: &str,
) -> Result<String, String> {
    let OAuthAuthorization {
        client_id,
        redirect_uri,
        state,
        resource,
    } = authorization;
    let challenge = pkce_challenge(verifier);
    let page = http_get(
        endpoint,
        &oauth_authorize_path(
            client_id,
            redirect_uri,
            state,
            "agentic:mcp",
            &challenge,
            "S256",
            resource,
        ),
        Some(host),
        &[],
    )?;
    require(page.status == 200, "fresh authorize request did not render")?;
    let submitted = oauth_authorize_form(
        endpoint,
        host,
        OAuthFormRequest {
            authorization,
            scope: "agentic:mcp",
            code_challenge: &challenge,
            code_challenge_method: "S256",
        },
        bearer_token,
    )?;
    require(submitted.status == 303, "fresh authorize submission failed")?;
    let location = submitted
        .header("location")
        .ok_or_else(|| "fresh authorize submission omitted Location".to_string())?;
    let (code, returned_state) = oauth_code_and_state(location)?;
    require(
        returned_state == state,
        "fresh authorize submission changed state",
    )?;
    Ok(code)
}

fn oauth_code_and_state(location: &str) -> Result<(String, String), String> {
    let url = Url::parse(location).map_err(|error| format!("invalid OAuth redirect: {error}"))?;
    let mut code = None;
    let mut state = None;
    let mut names = Vec::new();
    for (name, value) in url.query_pairs() {
        names.push(name.to_string());
        match name.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            _ => {}
        }
    }
    require(
        names.iter().all(|name| name == "code" || name == "state")
            && names.iter().any(|name| name == "code"),
        "OAuth redirect contained unexpected query fields",
    )?;
    Ok((
        code.ok_or_else(|| "OAuth redirect omitted code".to_string())?,
        state.ok_or_else(|| "OAuth redirect omitted state".to_string())?,
    ))
}

fn oauth_token(
    endpoint: &Endpoint,
    host: &str,
    fields: &[(&str, &str)],
    headers: &[(&str, &str)],
) -> Result<HttpResponse, String> {
    let body = form_body(fields);
    http_form_post(endpoint, "/oauth/token", Some(host), &body, headers)
}

fn assert_oauth_error(response: &HttpResponse, expected: &str, label: &str) -> Result<(), String> {
    require(
        response.status == 400
            && response.header("cache-control") == Some("no-store")
            && response.header("pragma") == Some("no-cache"),
        &format!("{label} returned HTTP/status headers inconsistent with OAuth errors"),
    )?;
    let value: Value =
        serde_json::from_str(&response.body).map_err(|error| format!("{label}: {error}"))?;
    require(
        value["error"] == expected && value.get("access_token").is_none(),
        &format!("{label} returned the wrong flat OAuth error"),
    )
}
fn assert_bearer_challenge(
    response: &HttpResponse,
    expected_resource_metadata: Option<&str>,
    label: &str,
) -> Result<(), String> {
    let challenge = response
        .header("www-authenticate")
        .ok_or_else(|| format!("{label} omitted WWW-Authenticate"))?;
    let params = challenge
        .trim()
        .strip_prefix("Bearer")
        .map(str::trim)
        .ok_or_else(|| format!("{label} did not begin with a Bearer challenge"))?;
    if !params.is_empty() {
        for segment in params.split(',') {
            let segment = segment.trim();
            require(
                !segment.is_empty()
                    && segment.split_once('=').is_some_and(|(_, value)| {
                        value.len() >= 2 && value.starts_with('"') && value.ends_with('"')
                    }),
                &format!("{label} contained an unparsable challenge parameter"),
            )?;
        }
    }
    if let Some(expected) = expected_resource_metadata {
        require(
            challenge.contains(&format!("resource_metadata=\"{expected}\""))
                && challenge.contains("scope=\"agentic:mcp\""),
            &format!("{label} omitted the configured OAuth metadata and scope"),
        )?;
    }
    Ok(())
}

fn http_get(
    endpoint: &Endpoint,
    path: &str,
    host: Option<&str>,
    headers: &[(&str, &str)],
) -> Result<HttpResponse, String> {
    http_request(endpoint, "GET", path, host, headers, None)
}

fn http_form_post(
    endpoint: &Endpoint,
    path: &str,
    host: Option<&str>,
    body: &str,
    headers: &[(&str, &str)],
) -> Result<HttpResponse, String> {
    let mut all_headers = Vec::with_capacity(headers.len() + 1);
    all_headers.push(("Content-Type", "application/x-www-form-urlencoded"));
    all_headers.extend_from_slice(headers);
    http_request(
        endpoint,
        "POST",
        path,
        host,
        &all_headers,
        Some(body.as_bytes()),
    )
}

fn http_request(
    endpoint: &Endpoint,
    method: &str,
    path: &str,
    host: Option<&str>,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
) -> Result<HttpResponse, String> {
    let mut stream = TcpStream::connect(endpoint.address())
        .map_err(|error| format!("HTTP connect to {} failed: {error}", endpoint.address()))?;
    stream
        .set_read_timeout(Some(OPERATION_TIMEOUT))
        .map_err(|error| error.to_string())?;
    let mut head = format!("{method} {path} HTTP/1.1\r\n");
    if let Some(host) = host {
        head.push_str("Host: ");
        head.push_str(host);
        head.push_str("\r\n");
    }
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    if let Some(body) = body {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("Connection: close\r\n\r\n");
    stream
        .write_all(head.as_bytes())
        .and_then(|_| {
            if let Some(body) = body {
                stream.write_all(body)
            } else {
                Ok(())
            }
        })
        .and_then(|_| stream.flush())
        .map_err(|error| error.to_string())?;
    read_http_response(stream)
}

fn http_initialize(endpoint: &Endpoint, token: &str, id: i64) -> Result<String, String> {
    let authorization = format!("Bearer {token}");
    let response = http_post(
        endpoint,
        Some(&authorization),
        None,
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": INITIAL_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "standalone-http-mcp-test", "version": "1" }
            }
        }),
    )?;
    require(
        response.status == 200,
        &format!("HTTP initialize returned status {}", response.status),
    )?;
    require(
        response
            .header("content-type")
            .is_some_and(|value| value.contains("text/event-stream")),
        "HTTP initialize did not return an SSE response",
    )?;
    let session_id = response
        .header("mcp-session-id")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "HTTP initialize omitted Mcp-Session-Id".to_string())?
        .to_string();
    let message = sse_message(&response)?;
    require(
        message["result"]["protocolVersion"].is_string(),
        "HTTP initialize SSE did not contain an initialize result",
    )?;
    let initialized = http_post(
        endpoint,
        Some(&authorization),
        Some(&session_id),
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }),
    )?;
    require(
        initialized.status == 202 || initialized.status == 200,
        &format!(
            "HTTP notifications/initialized returned status {}",
            initialized.status
        ),
    )?;
    Ok(session_id)
}

fn http_tools_list(
    endpoint: &Endpoint,
    token: &str,
    session_id: &str,
    id: i64,
) -> Result<(HttpResponse, Value), String> {
    let authorization = format!("Bearer {token}");
    http_tools_list_with_authorization(endpoint, &authorization, session_id, id)
}

fn http_tools_list_with_authorization(
    endpoint: &Endpoint,
    authorization: &str,
    session_id: &str,
    id: i64,
) -> Result<(HttpResponse, Value), String> {
    wait_for_http_exchange(
        endpoint,
        Some(authorization),
        Some(session_id),
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/list",
            "params": {}
        }),
        200,
    )
}

fn http_call(
    endpoint: &Endpoint,
    token: &str,
    session_id: &str,
    id: i64,
    name: &str,
    arguments: Value,
) -> Result<(HttpResponse, Value), String> {
    let authorization = format!("Bearer {token}");
    wait_for_http_exchange(
        endpoint,
        Some(&authorization),
        Some(session_id),
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }),
        200,
    )
}

fn wait_for_http_exchange(
    endpoint: &Endpoint,
    authorization: Option<&str>,
    session_id: Option<&str>,
    request: Value,
    expected_status: u16,
) -> Result<(HttpResponse, Value), String> {
    let deadline = Instant::now() + OPERATION_TIMEOUT;
    let mut last_error = String::new();
    while Instant::now() < deadline {
        match http_post(endpoint, authorization, session_id, request.clone()) {
            Ok(response) if response.status == expected_status => match sse_message(&response) {
                Ok(message) => return Ok((response, message)),
                Err(error) => last_error = error,
            },
            Ok(response) => {
                last_error = format!(
                    "HTTP request returned status {}, expected {}",
                    response.status, expected_status
                );
            }
            Err(error) => last_error = error,
        }
        thread::sleep(POLL_INTERVAL);
    }
    Err(format!(
        "timed out waiting for HTTP status {expected_status}: {last_error}"
    ))
}

fn wait_for_http_status(
    endpoint: &Endpoint,
    authorization: Option<&str>,
    session_id: Option<&str>,
    request: Value,
    expected_status: u16,
) -> Result<HttpResponse, String> {
    let host_header = endpoint.address();
    wait_for_http_status_with_host(
        endpoint,
        &host_header,
        authorization,
        session_id,
        request,
        expected_status,
    )
}

fn wait_for_http_status_with_host(
    endpoint: &Endpoint,
    host_header: &str,
    authorization: Option<&str>,
    session_id: Option<&str>,
    request: Value,
    expected_status: u16,
) -> Result<HttpResponse, String> {
    let deadline = Instant::now() + OPERATION_TIMEOUT;
    let mut last_error = String::new();
    while Instant::now() < deadline {
        match http_post_with_host(
            endpoint,
            host_header,
            authorization,
            session_id,
            request.clone(),
        ) {
            Ok(response) if response.status == expected_status => return Ok(response),
            Ok(response) => {
                last_error = format!(
                    "HTTP request returned status {}, expected {}",
                    response.status, expected_status
                );
            }
            Err(error) => last_error = error,
        }
        thread::sleep(POLL_INTERVAL);
    }
    Err(format!(
        "timed out waiting for HTTP status {expected_status}: {last_error}"
    ))
}

fn wait_for_http_tools_presence(
    endpoint: &Endpoint,
    token: &str,
    session_id: &str,
    id: i64,
    tool: &str,
    expected_presence: bool,
) -> Result<(HttpResponse, Value), String> {
    let deadline = Instant::now() + OPERATION_TIMEOUT;
    let mut last_error = String::new();
    while Instant::now() < deadline {
        match http_tools_list(endpoint, token, session_id, id) {
            Ok((response, message)) => {
                let present = tool_names(&message["result"]["tools"])
                    .iter()
                    .any(|name| name == tool);
                if present == expected_presence {
                    return Ok((response, message));
                }
                last_error =
                    format!("tool {tool} presence was {present}, expected {expected_presence}");
            }
            Err(error) => last_error = error,
        }
        thread::sleep(POLL_INTERVAL);
    }
    Err(format!(
        "timed out waiting for tool {tool} presence {expected_presence}: {last_error}"
    ))
}

fn assert_unauthorized(
    endpoint: &Endpoint,
    authorization: Option<&str>,
    label: &str,
) -> Result<(), String> {
    let response = http_post(
        endpoint,
        authorization,
        None,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": INITIAL_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "standalone-http-mcp-auth-test", "version": "1" }
            }
        }),
    )?;
    require(
        response.status == 401,
        &format!(
            "{label} returned HTTP status {} instead of 401",
            response.status
        ),
    )?;
    assert_bearer_challenge(&response, None, label)?;
    Ok(())
}

fn http_post(
    endpoint: &Endpoint,
    authorization: Option<&str>,
    session_id: Option<&str>,
    request: Value,
) -> Result<HttpResponse, String> {
    let host_header = endpoint.address();
    http_post_with_host(endpoint, &host_header, authorization, session_id, request)
}

fn http_post_with_host(
    endpoint: &Endpoint,
    host_header: &str,
    authorization: Option<&str>,
    session_id: Option<&str>,
    request: Value,
) -> Result<HttpResponse, String> {
    let body = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
    let mut stream = TcpStream::connect(endpoint.address())
        .map_err(|error| format!("HTTP connect to {} failed: {error}", endpoint.address()))?;
    stream
        .set_read_timeout(Some(OPERATION_TIMEOUT))
        .map_err(|error| error.to_string())?;
    let mut head = format!(
        "POST /mcp HTTP/1.1\r\nHost: {host_header}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(authorization) = authorization {
        head.push_str("Authorization: ");
        head.push_str(authorization);
        head.push_str("\r\n");
    }
    if let Some(session_id) = session_id {
        head.push_str("Mcp-Session-Id: ");
        head.push_str(session_id);
        head.push_str("\r\nMCP-Protocol-Version: ");
        head.push_str(INITIAL_PROTOCOL_VERSION);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    stream
        .write_all(head.as_bytes())
        .and_then(|_| stream.write_all(&body))
        .and_then(|_| stream.flush())
        .map_err(|error| error.to_string())?;
    read_http_response(stream)
}
fn read_http_response(stream: TcpStream) -> Result<HttpResponse, String> {
    let mut reader = BufReader::new(stream);
    let mut header_bytes = Vec::new();
    loop {
        let mut line = Vec::new();
        let read = reader
            .read_until(b'\n', &mut line)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("HTTP response closed before headers".to_string());
        }
        header_bytes.extend_from_slice(&line);
        if line.as_slice() == b"\r\n" || line.as_slice() == b"\n" {
            break;
        }
    }
    let header_text = String::from_utf8_lossy(&header_bytes);
    let mut lines = header_text.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| "HTTP status line missing".to_string())?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| "HTTP status code missing".to_string())?
        .parse::<u16>()
        .map_err(|error| format!("invalid HTTP status: {error}"))?;
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(format!("malformed HTTP response header: {line}"));
        };
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
    }

    let mut body = Vec::new();
    if let Some(content_length) = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .map(|(_, value)| value)
    {
        let length = content_length
            .parse::<usize>()
            .map_err(|error| format!("invalid content length: {error}"))?;
        body.resize(length, 0);
        reader
            .read_exact(&mut body)
            .map_err(|error| error.to_string())?;
    } else if headers
        .iter()
        .find(|(name, _)| name == "transfer-encoding")
        .is_some_and(|(_, value)| value.to_ascii_lowercase().contains("chunked"))
    {
        body = read_chunked_body(&mut reader)?;
    } else {
        reader
            .read_to_end(&mut body)
            .map_err(|error| error.to_string())?;
    }
    Ok(HttpResponse {
        status,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn read_chunked_body(reader: &mut impl BufRead) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    loop {
        let mut size_line = String::new();
        reader
            .read_line(&mut size_line)
            .map_err(|error| error.to_string())?;
        let size_text = size_line
            .split(';')
            .next()
            .ok_or_else(|| "chunk size missing".to_string())?
            .trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|error| format!("invalid chunk size: {error}"))?;
        if size == 0 {
            loop {
                let mut trailer = String::new();
                reader
                    .read_line(&mut trailer)
                    .map_err(|error| error.to_string())?;
                if trailer == "\r\n" || trailer == "\n" || trailer.is_empty() {
                    break;
                }
            }
            break;
        }
        let offset = body.len();
        body.resize(offset + size, 0);
        reader
            .read_exact(&mut body[offset..])
            .map_err(|error| error.to_string())?;
        let mut suffix = [0u8; 2];
        reader
            .read_exact(&mut suffix)
            .map_err(|error| error.to_string())?;
        if suffix != *b"\r\n" {
            return Err("chunk body missing CRLF".to_string());
        }
    }
    Ok(body)
}

fn sse_message(response: &HttpResponse) -> Result<Value, String> {
    for line in response.body.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() {
            continue;
        }
        if let Ok(message) = serde_json::from_str::<Value>(data) {
            return Ok(message);
        }
    }
    Err(format!(
        "SSE response did not contain a JSON message: {}",
        response.body
    ))
}

fn sorted_tools(value: &Value) -> Result<Vec<Value>, String> {
    let mut tools = value
        .as_array()
        .ok_or_else(|| "MCP tools value is not an array".to_string())?
        .clone();
    tools.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    Ok(tools)
}

fn tool_names(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tool| tool["name"].as_str().map(str::to_string))
        .collect()
}

fn assert_surface(tools: &[Value]) -> Result<(), String> {
    require(
        !tools.is_empty(),
        "MCP tools/list returned an empty surface",
    )?;
    for tool in tools {
        require(
            tool["_meta"]["surface"] == "agent-local",
            &format!(
                "tool {} did not advertise agent-local surface",
                tool["name"]
            ),
        )?;
        require(
            tool.get("agentId").is_none() && tool.get("confirmMethod").is_none(),
            &format!("tool {} exposed Hub-only fields", tool["name"]),
        )?;
    }
    Ok(())
}

fn wait_for_local_tools(binary: &Path, config_path: &Path) -> Result<Value, String> {
    let deadline = Instant::now() + OPERATION_TIMEOUT;
    let mut last_error = String::new();
    while Instant::now() < deadline {
        match Command::new(binary)
            .args(["local", "list-tools", "--config"])
            .arg(config_path)
            .output()
        {
            Ok(output) if output.status.success() => {
                return serde_json::from_slice(&output.stdout).map_err(|error| error.to_string());
            }
            Ok(output) => {
                last_error = String::from_utf8_lossy(&output.stderr).into_owned();
            }
            Err(error) => last_error = error.to_string(),
        }
        thread::sleep(POLL_INTERVAL);
    }
    Err(format!(
        "timed out waiting for local MCP tool list: {last_error}"
    ))
}

fn read_config(path: &Path) -> Result<Value, String> {
    serde_json::from_slice(&fs::read(path).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())
}

fn write_config(path: &Path, config: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(config).map_err(|error| error.to_string())?;
    fs::write(path, bytes).map_err(|error| error.to_string())
}

fn free_port() -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| error.to_string())
}

fn wait_for_tcp(endpoint: &Endpoint, should_be_available: bool) -> Result<(), String> {
    let deadline = Instant::now() + OPERATION_TIMEOUT;
    while Instant::now() < deadline {
        match TcpStream::connect(endpoint.address()) {
            Ok(stream) if should_be_available => {
                drop(stream);
                return Ok(());
            }
            Ok(stream) => drop(stream),
            Err(_) if !should_be_available => return Ok(()),
            Err(_) => {}
        }
        thread::sleep(POLL_INTERVAL);
    }
    Err(format!(
        "timed out waiting for {} to be {}",
        endpoint.address(),
        if should_be_available {
            "available"
        } else {
            "closed"
        }
    ))
}

fn send_line(stdin: &mut impl Write, value: Value) -> Result<(), String> {
    writeln!(stdin, "{value}").map_err(|error| error.to_string())?;
    stdin.flush().map_err(|error| error.to_string())
}

fn response_for(reader: &Receiver<String>, id: i64) -> Result<Value, String> {
    let deadline = Instant::now() + OPERATION_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(format!("timed out waiting for stdio response {id}"));
        }
        let line = match reader.recv_timeout(remaining) {
            Ok(line) => line,
            Err(RecvTimeoutError::Timeout) => {
                return Err(format!("timed out waiting for stdio response {id}"));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(format!(
                    "worker stdout closed while waiting for response {id}"
                ));
            }
        };
        let value: Value = serde_json::from_str(line.trim())
            .map_err(|error| format!("invalid worker JSON response: {error}; line={line}"))?;
        if value["id"] == json!(id) {
            return Ok(value);
        }
    }
}

fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.to_string())
    }
}

fn local_socket_path(agent_id: &str) -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or_else(|| "HOME is unavailable".to_string())?;
    Ok(PathBuf::from(home)
        .join(".agentic_gpt/runtime/agent")
        .join(agent_id)
        .join("mcp.sock"))
}

fn wait_for_path_absent(path: &Path, duration: Duration) -> Result<(), String> {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if !path.exists() {
            return Ok(());
        }
        thread::sleep(POLL_INTERVAL);
    }
    Err(format!(
        "runtime path survived shutdown: {}",
        path.display()
    ))
}

fn stop_child_gracefully(child: &mut Child, duration: Duration) {
    if child.try_wait().ok().flatten().is_none() {
        unsafe {
            libc::kill(child.id() as i32, libc::SIGINT);
        }
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            if child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(POLL_INTERVAL);
        }
        let _ = child.kill();
    }
    let _ = child.wait();
}
