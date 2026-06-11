use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::Json;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::time::{sleep, Duration};
use url::Url;

use crate::routes::{api_error, parse_bearer_token};
use crate::state::HubState;
use crate::utils::{constant_time_equal, random_token, sha256_hex};

const OAUTH_SCOPE: &str = "agentic:mcp";
const CODE_TTL_SECONDS: i64 = 600;
const TOKEN_TTL_SECONDS: i64 = 7 * 24 * 60 * 60;

#[derive(Clone, Debug)]
pub(crate) struct OAuthAuthorizationCode {
    pub(crate) client_id: String,
    pub(crate) redirect_uri: String,
    pub(crate) code_challenge: String,
    pub(crate) scope: String,
    pub(crate) resource: Option<String>,
    pub(crate) expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub(crate) struct OAuthAccessToken {
    pub(crate) expires_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct AuthorizeParams {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    state: Option<String>,
    scope: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    resource: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct AuthorizeForm {
    api_key: String,
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    state: Option<String>,
    scope: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    resource: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct TokenForm {
    grant_type: Option<String>,
    code: Option<String>,
    redirect_uri: Option<String>,
    client_id: Option<String>,
    code_verifier: Option<String>,
    resource: Option<String>,
}

#[derive(Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: &'static str,
    expires_in: i64,
    scope: String,
}

pub(crate) async fn protected_resource_metadata(
    State(state): State<HubState>,
    headers: HeaderMap,
) -> Response {
    let base_url = public_base_url(&state, &headers);
    Json(json!({
        "resource": mcp_resource_url(&state, &headers),
        "authorization_servers": [base_url],
        "scopes_supported": [OAUTH_SCOPE],
        "resource_documentation": format!("{}/mcp", public_base_url(&state, &headers))
    }))
    .into_response()
}

pub(crate) async fn authorization_server_metadata(
    State(state): State<HubState>,
    headers: HeaderMap,
) -> Response {
    let base_url = public_base_url(&state, &headers);
    Json(json!({
        "issuer": base_url,
        "authorization_endpoint": format!("{base_url}/oauth/authorize"),
        "token_endpoint": format!("{base_url}/oauth/token"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code"],
        "token_endpoint_auth_methods_supported": ["none"],
        "code_challenge_methods_supported": ["S256"],
        "client_id_metadata_document_supported": true,
        "scopes_supported": [OAUTH_SCOPE]
    }))
    .into_response()
}

pub(crate) async fn authorize(
    State(state): State<HubState>,
    headers: HeaderMap,
    Query(params): Query<AuthorizeParams>,
) -> Response {
    match validate_authorize_params(&state, &headers, &params) {
        Ok(()) => authorize_page(None, &params),
        Err(message) => Html(error_page(&message)).into_response(),
    }
}

pub(crate) async fn authorize_submit(
    State(state): State<HubState>,
    headers: HeaderMap,
    Form(form): Form<AuthorizeForm>,
) -> Response {
    let params = AuthorizeParams {
        response_type: form.response_type.clone(),
        client_id: form.client_id.clone(),
        redirect_uri: form.redirect_uri.clone(),
        state: form.state.clone(),
        scope: form.scope.clone(),
        code_challenge: form.code_challenge.clone(),
        code_challenge_method: form.code_challenge_method.clone(),
        resource: form.resource.clone(),
    };
    if let Err(message) = validate_authorize_params(&state, &headers, &params) {
        return Html(error_page(&message)).into_response();
    }
    if !constant_time_equal(form.api_key.trim(), state.api_key.trim()) {
        return authorize_page(Some("API key 不对，再看一眼。"), &params);
    }

    let client_id = form.client_id.unwrap_or_default();
    let redirect_uri = form.redirect_uri.unwrap_or_default();
    let code_challenge = form.code_challenge.unwrap_or_default();
    let code = format!("oauth_code_{}", random_token());
    let code_hash = sha256_hex(&code);
    let scope = normalized_scope(form.scope);
    let expires_at = Utc::now() + chrono::Duration::seconds(CODE_TTL_SECONDS);
    state.oauth_codes.lock().await.insert(
        code_hash,
        OAuthAuthorizationCode {
            client_id,
            redirect_uri: redirect_uri.clone(),
            code_challenge,
            scope,
            resource: form.resource,
            expires_at,
        },
    );

    let mut redirect = match Url::parse(&redirect_uri) {
        Ok(value) => value,
        Err(_) => return Html(error_page("redirect_uri 无效。")).into_response(),
    };
    {
        let mut pairs = redirect.query_pairs_mut();
        pairs.append_pair("code", &code);
        if let Some(state_value) = form.state.as_deref() {
            pairs.append_pair("state", state_value);
        }
    }
    Redirect::to(redirect.as_str()).into_response()
}

pub(crate) async fn token(State(state): State<HubState>, Form(form): Form<TokenForm>) -> Response {
    if form.grant_type.as_deref() != Some("authorization_code") {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "Only authorization_code is supported",
        );
    }
    let Some(code) = form.code.as_deref() else {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_request", "Missing code");
    };
    let Some(code_verifier) = form.code_verifier.as_deref() else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Missing code_verifier",
        );
    };
    let Some(redirect_uri) = form.redirect_uri.as_deref() else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Missing redirect_uri",
        );
    };
    let Some(client_id) = form.client_id.as_deref() else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Missing client_id",
        );
    };

    let code_hash = sha256_hex(code);
    let Some(stored) = state.oauth_codes.lock().await.remove(&code_hash) else {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "Invalid code");
    };
    if stored.expires_at <= Utc::now() {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "Code expired");
    }
    if stored.redirect_uri != redirect_uri || stored.client_id != client_id {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "Code does not match client or redirect_uri",
        );
    }
    if form.resource.is_some() && form.resource != stored.resource {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_target",
            "resource does not match authorization request",
        );
    }
    if !verify_pkce_s256(code_verifier, &stored.code_challenge) {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "PKCE verification failed",
        );
    }

    let access_token = format!("ag_oauth_{}", random_token());
    let token_hash = sha256_hex(&access_token);
    let expires_at = Utc::now() + chrono::Duration::seconds(TOKEN_TTL_SECONDS);
    state
        .oauth_tokens
        .lock()
        .await
        .insert(token_hash, OAuthAccessToken { expires_at });
    Json(TokenResponse {
        access_token,
        token_type: "Bearer",
        expires_in: TOKEN_TTL_SECONDS,
        scope: stored.scope,
    })
    .into_response()
}

pub(crate) async fn is_valid_mcp_bearer(state: &HubState, headers: &HeaderMap) -> bool {
    let auth = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let Some(token) = parse_bearer_token(auth) else {
        return false;
    };
    if constant_time_equal(token.as_str(), state.api_key.trim()) {
        return true;
    }
    let token_hash = sha256_hex(&token);
    let now = Utc::now();
    let mut tokens = state.oauth_tokens.lock().await;
    tokens.retain(|_, token| token.expires_at > now);
    tokens.contains_key(&token_hash)
}

pub(crate) fn mcp_unauthorized_response(state: &HubState, headers: &HeaderMap) -> Response {
    let metadata_url = format!(
        "{}/.well-known/oauth-protected-resource",
        public_base_url(state, headers)
    );
    let mut response = api_error(StatusCode::UNAUTHORIZED, "unauthorized", "Unauthorized");
    let header_value =
        format!("Bearer resource_metadata=\"{metadata_url}\", scope=\"{OAUTH_SCOPE}\"");
    if let Ok(value) = HeaderValue::from_str(&header_value) {
        response
            .headers_mut()
            .insert(axum::http::header::WWW_AUTHENTICATE, value);
    }
    response
}

pub(crate) async fn cleanup_oauth(state: HubState) {
    loop {
        sleep(Duration::from_secs(60)).await;
        let now = Utc::now();
        state
            .oauth_codes
            .lock()
            .await
            .retain(|_, code| code.expires_at > now);
        state
            .oauth_tokens
            .lock()
            .await
            .retain(|_, token| token.expires_at > now);
    }
}

fn validate_authorize_params(
    state: &HubState,
    headers: &HeaderMap,
    params: &AuthorizeParams,
) -> std::result::Result<(), String> {
    if params.response_type.as_deref() != Some("code") {
        return Err("只支持 response_type=code。".to_string());
    }
    let client_id = params
        .client_id
        .as_deref()
        .ok_or_else(|| "缺少 client_id。".to_string())?;
    if client_id.trim().is_empty() {
        return Err("client_id 为空。".to_string());
    }
    let redirect_uri = params
        .redirect_uri
        .as_deref()
        .ok_or_else(|| "缺少 redirect_uri。".to_string())?;
    if !is_allowed_chatgpt_redirect_uri(redirect_uri) {
        return Err("redirect_uri 不在 ChatGPT OAuth 回调白名单。".to_string());
    }
    let method = params.code_challenge_method.as_deref().unwrap_or_default();
    if method != "S256" {
        return Err("只支持 PKCE S256。".to_string());
    }
    if params
        .code_challenge
        .as_deref()
        .map(str::is_empty)
        .unwrap_or(true)
    {
        return Err("缺少 code_challenge。".to_string());
    }
    if let Some(resource) = params.resource.as_deref() {
        let expected = mcp_resource_url(state, headers);
        if resource != expected {
            return Err(format!("resource 不匹配，预期 {expected}"));
        }
    }
    Ok(())
}

fn is_allowed_chatgpt_redirect_uri(value: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    if url.scheme() != "https" || url.host_str() != Some("chatgpt.com") {
        return false;
    }
    let path = url.path();
    path.starts_with("/connector/oauth/") || path == "/connector_platform_oauth_redirect"
}

fn verify_pkce_s256(code_verifier: &str, code_challenge: &str) -> bool {
    let digest = Sha256::digest(code_verifier.as_bytes());
    let computed = URL_SAFE_NO_PAD.encode(digest);
    constant_time_equal(&computed, code_challenge)
}

fn normalized_scope(scope: Option<String>) -> String {
    let value = scope.unwrap_or_else(|| OAUTH_SCOPE.to_string());
    if value.trim().is_empty() {
        OAUTH_SCOPE.to_string()
    } else {
        value
    }
}

fn public_base_url(state: &HubState, headers: &HeaderMap) -> String {
    if let Some(value) = state.public_base_url.as_deref() {
        return value.trim_end_matches('/').to_string();
    }
    let host = first_header(headers, "x-forwarded-host")
        .or_else(|| first_header(headers, "host"))
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let proto = first_header(headers, "x-forwarded-proto").unwrap_or_else(|| {
        if host.starts_with("127.") || host.starts_with("localhost") || host.starts_with("[") {
            "http".to_string()
        } else {
            "https".to_string()
        }
    });
    format!(
        "{}://{}",
        proto.trim_end_matches('/'),
        host.trim_end_matches('/')
    )
}

fn mcp_resource_url(state: &HubState, headers: &HeaderMap) -> String {
    format!("{}/mcp", public_base_url(state, headers))
}

fn first_header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn authorize_page(error: Option<&str>, params: &AuthorizeParams) -> Response {
    let error_html = error
        .map(|message| format!("<p class=\"error\">{}</p>", html_escape(message)))
        .unwrap_or_default();
    Html(format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>Authorize Agentic GPT</title>
<style>
body {{ font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; max-width: 520px; margin: 64px auto; padding: 0 20px; line-height: 1.5; }}
label {{ display: block; margin: 18px 0 8px; font-weight: 600; }}
input[type=password] {{ width: 100%; box-sizing: border-box; padding: 10px 12px; font: inherit; }}
button {{ margin-top: 18px; padding: 10px 14px; font: inherit; cursor: pointer; }}
.error {{ color: #b00020; }}
.small {{ color: #666; font-size: 0.92em; }}
</style>
</head>
<body>
<h1>授权 Agentic GPT</h1>
<p>输入 Hub API key 后，ChatGPT 会拿到一个短期访问令牌，用来连接你的 <code>/mcp</code>。</p>
{error_html}
<form method="post" action="/oauth/authorize">
<label for="api_key">Hub API key</label>
<input id="api_key" name="api_key" type="password" autocomplete="current-password" autofocus required />
{hidden_fields}
<button type="submit">授权</button>
</form>
<p class="small">令牌有效期 7 天。真正的工具执行仍然走 Agentic 本地确认和审计。</p>
</body>
</html>"#,
        hidden_fields = hidden_fields(params),
    ))
    .into_response()
}

fn hidden_fields(params: &AuthorizeParams) -> String {
    [
        ("response_type", params.response_type.as_deref()),
        ("client_id", params.client_id.as_deref()),
        ("redirect_uri", params.redirect_uri.as_deref()),
        ("state", params.state.as_deref()),
        ("scope", params.scope.as_deref()),
        ("code_challenge", params.code_challenge.as_deref()),
        (
            "code_challenge_method",
            params.code_challenge_method.as_deref(),
        ),
        ("resource", params.resource.as_deref()),
    ]
    .into_iter()
    .filter_map(|(name, value)| value.map(|value| (name, value)))
    .map(|(name, value)| {
        format!(
            "<input type=\"hidden\" name=\"{}\" value=\"{}\" />",
            html_escape(name),
            html_escape(value)
        )
    })
    .collect::<Vec<_>>()
    .join("\n")
}

fn error_page(message: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8" /><title>Agentic OAuth Error</title></head>
<body><h1>授权失败</h1><p>{}</p></body></html>"#,
        html_escape(message)
    )
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn oauth_error(status: StatusCode, error: &'static str, description: &'static str) -> Response {
    (
        status,
        Json(json!({ "error": error, "error_description": description })),
    )
        .into_response()
}
