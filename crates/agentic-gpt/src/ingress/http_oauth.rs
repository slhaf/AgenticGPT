use std::{collections::HashMap, sync::Arc, time::Duration};

use parking_lot::{Mutex, RwLock};

use axum::{
    extract::{
        rejection::{FormRejection, QueryRejection},
        Form, Query, Request, State,
    },
    http::{header, uri::Authority, HeaderMap, HeaderValue, StatusCode, Uri},
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

const OAUTH_SCOPE: &str = "agentic:mcp";
const CODE_TTL_SECONDS: i64 = 600;
const TOKEN_TTL_SECONDS: i64 = 7 * 24 * 60 * 60;
const CLEANUP_INTERVAL: Duration = Duration::from_secs(60);
const PATH_SPECIFIC_RESOURCE_METADATA: &str = "/.well-known/oauth-protected-resource/mcp";
const ROOT_RESOURCE_METADATA: &str = "/.well-known/oauth-protected-resource";
const AUTHORIZATION_SERVER_METADATA: &str = "/.well-known/oauth-authorization-server";
const OPENID_CONFIGURATION: &str = "/.well-known/openid-configuration";

#[derive(Clone)]
pub(crate) struct HttpMcpAuthState {
    resolved_token: Arc<RwLock<Option<String>>>,
    public_url: Option<String>,
    oauth_codes: Arc<Mutex<HashMap<String, OAuthAuthorizationCode>>>,
    oauth_tokens: Arc<Mutex<HashMap<String, OAuthAccessToken>>>,
    mutation_lock: Arc<Mutex<()>>,
}

#[derive(Clone)]
struct OAuthAuthorizationCode {
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    scope: String,
    resource: String,
    expires_at: DateTime<Utc>,
}

#[derive(Clone)]
struct OAuthAccessToken {
    expires_at: DateTime<Utc>,
    resource: String,
    scope: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
struct AuthorizeParams {
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
struct AuthorizeForm {
    bearer_token: String,
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
struct TokenForm {
    grant_type: Option<String>,
    code: Option<String>,
    redirect_uri: Option<String>,
    client_id: Option<String>,
    code_verifier: Option<String>,
    resource: Option<String>,
    scope: Option<String>,
}

#[derive(Debug)]
struct AuthorizeError {
    code: &'static str,
    message: String,
}

impl AuthorizeError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn required_public_url() -> Self {
        Self::new(
            "http_mcp_public_url_required",
            "A configured HTTPS public URL is required for ChatGPT OAuth.",
        )
    }
}

#[derive(serde::Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: &'static str,
    expires_in: i64,
    scope: String,
}

impl HttpMcpAuthState {
    pub(crate) fn new(resolved_token: String, public_url: Option<String>) -> Self {
        let public_url =
            public_url.and_then(|value| crate::config::normalize_http_mcp_public_url(&value).ok());
        Self {
            resolved_token: Arc::new(RwLock::new(Some(resolved_token))),
            public_url,
            oauth_codes: Arc::new(Mutex::new(HashMap::new())),
            oauth_tokens: Arc::new(Mutex::new(HashMap::new())),
            mutation_lock: Arc::new(Mutex::new(())),
        }
    }

    pub(crate) fn replace_resolved_token(&self, resolved_token: String) -> bool {
        let _mutation = self.mutation_lock.lock();
        let mut current = self.resolved_token.write();
        if current.as_deref() == Some(resolved_token.as_str()) {
            return false;
        }
        *current = Some(resolved_token);
        drop(current);
        self.oauth_codes.lock().clear();
        self.oauth_tokens.lock().clear();
        true
    }

    pub(crate) fn accepts_bearer(&self, presented: &str, request_resource: Option<&str>) -> bool {
        let _mutation = self.mutation_lock.lock();
        let expected = self.resolved_token.read();
        if expected
            .as_deref()
            .is_some_and(|value| constant_time_equal(value, presented))
        {
            return true;
        }
        drop(expected);

        let token_hash = sha256_hex(presented);
        let now = Utc::now();
        let mut tokens = self.oauth_tokens.lock();
        tokens.retain(|_, token| token.expires_at > now);
        tokens.get(&token_hash).is_some_and(|token| {
            token.expires_at > now
                && token.scope == OAUTH_SCOPE
                && request_resource.is_some_and(|resource| resource == token.resource)
        })
    }

    fn public_url(&self) -> Option<&str> {
        self.public_url.as_deref()
    }

    fn resource_url(&self) -> Option<String> {
        self.public_url.as_ref().map(|value| format!("{value}/mcp"))
    }

    fn challenge(&self) -> String {
        match self.public_url() {
            Some(public_url) => format!(
                "Bearer resource_metadata=\"{public_url}{PATH_SPECIFIC_RESOURCE_METADATA}\", scope=\"{OAUTH_SCOPE}\""
            ),
            None => "Bearer".to_string(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct HttpMcpHostPolicy {
    public_url: Option<String>,
    allow_hosts: Option<Vec<String>>,
}

impl HttpMcpHostPolicy {
    pub(crate) fn new(public_url: Option<String>, allow_hosts: Option<Vec<String>>) -> Self {
        let public_url =
            public_url.and_then(|value| crate::config::normalize_http_mcp_public_url(&value).ok());
        Self {
            public_url,
            allow_hosts,
        }
    }
}

pub(crate) fn routes(state: HttpMcpAuthState) -> Router {
    Router::new()
        .route(
            PATH_SPECIFIC_RESOURCE_METADATA,
            get(protected_resource_metadata),
        )
        .route(ROOT_RESOURCE_METADATA, get(protected_resource_metadata))
        .route(
            AUTHORIZATION_SERVER_METADATA,
            get(authorization_server_metadata),
        )
        .route(OPENID_CONFIGURATION, get(authorization_server_metadata))
        .route("/oauth/authorize", get(authorize).post(authorize_submit))
        .route("/oauth/token", post(token))
        .with_state(state)
}

pub(crate) async fn cleanup(state: HttpMcpAuthState, cancellation: CancellationToken) {
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => break,
            _ = sleep(CLEANUP_INTERVAL) => {
                let _mutation = state.mutation_lock.lock();
                let now = Utc::now();
                state
                    .oauth_codes
                    .lock()
                    .retain(|_, code| code.expires_at > now);
                state
                    .oauth_tokens
                    .lock()
                    .retain(|_, token| token.expires_at > now);
            }
        }
    }
}

pub(crate) async fn require_bearer(
    State(state): State<HttpMcpAuthState>,
    request: Request,
    next: Next,
) -> Response {
    let Some(presented) = parse_bearer_token(request.headers()) else {
        return unauthorized_response(&state);
    };
    if state.accepts_bearer(presented, state.resource_url().as_deref()) {
        next.run(request).await
    } else {
        unauthorized_response(&state)
    }
}

pub(crate) async fn require_host_origin(
    State(policy): State<HttpMcpHostPolicy>,
    request: Request,
    next: Next,
) -> Response {
    if let Err(response) = validate_host_origin(request.uri(), request.headers(), &policy) {
        return *response;
    }
    next.run(request).await
}

async fn protected_resource_metadata(State(state): State<HttpMcpAuthState>) -> Response {
    let Some(public_url) = state.public_url() else {
        return oauth_configuration_required();
    };
    json_response(json!({
        "resource": format!("{public_url}/mcp"),
        "authorization_servers": [public_url],
        "scopes_supported": [OAUTH_SCOPE],
        "bearer_methods_supported": ["header"],
        "resource_documentation": format!("{public_url}/mcp"),
    }))
}

async fn authorization_server_metadata(State(state): State<HttpMcpAuthState>) -> Response {
    let Some(public_url) = state.public_url() else {
        return oauth_configuration_required();
    };
    json_response(json!({
        "issuer": public_url,
        "authorization_endpoint": format!("{public_url}/oauth/authorize"),
        "token_endpoint": format!("{public_url}/oauth/token"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code"],
        "token_endpoint_auth_methods_supported": ["none"],
        "code_challenge_methods_supported": ["S256"],
        "client_id_metadata_document_supported": true,
        "scopes_supported": [OAUTH_SCOPE],
    }))
}

async fn authorize(
    State(state): State<HttpMcpAuthState>,
    params: Result<Query<AuthorizeParams>, QueryRejection>,
) -> Response {
    let Query(params) = match params {
        Ok(params) => params,
        Err(_) => {
            return html_error_response(&AuthorizeError::new(
                "invalid_request",
                "Invalid authorization query.",
            ))
        }
    };
    match validate_authorize_params(&state, &params) {
        Ok(_) => authorize_page(None, &params),
        Err(error) => html_error_response(&error),
    }
}

async fn authorize_submit(
    State(state): State<HttpMcpAuthState>,
    form: Result<Form<AuthorizeForm>, FormRejection>,
) -> Response {
    let Form(form) = match form {
        Ok(form) => form,
        Err(_) => {
            return html_error_response(&AuthorizeError::new(
                "invalid_request",
                "Invalid authorization form.",
            ))
        }
    };
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
    let validated = match validate_authorize_params(&state, &params) {
        Ok(validated) => validated,
        Err(error) => return html_error_response(&error),
    };

    let mut redirect = match Url::parse(&validated.redirect_uri) {
        Ok(redirect) => redirect,
        Err(_) => {
            return html_error_response(&AuthorizeError::new(
                "invalid_request",
                "Invalid redirect URI.",
            ))
        }
    };
    let code = opaque_value("code");
    let code_hash = sha256_hex(&code);
    let _mutation = state.mutation_lock.lock();
    let expected = state.resolved_token.read();
    if !expected
        .as_deref()
        .is_some_and(|value| constant_time_equal(value, &form.bearer_token))
    {
        return authorize_page(
            Some(&AuthorizeError::new(
                "invalid_client",
                "The HTTP MCP bearer token was not accepted.",
            )),
            &params,
        );
    }
    drop(expected);
    state.oauth_codes.lock().insert(
        code_hash,
        OAuthAuthorizationCode {
            client_id: validated.client_id,
            redirect_uri: validated.redirect_uri,
            code_challenge: validated.code_challenge,
            scope: validated.scope,
            resource: validated.resource,
            expires_at: Utc::now() + chrono::Duration::seconds(CODE_TTL_SECONDS),
        },
    );
    redirect.query_pairs_mut().append_pair("code", &code);
    if let Some(state_value) = form.state.as_deref() {
        redirect.query_pairs_mut().append_pair("state", state_value);
    }
    let mut response = Redirect::to(redirect.as_str()).into_response();
    add_no_store_headers(&mut response, true);
    response
}

async fn token(
    State(state): State<HttpMcpAuthState>,
    form: Result<Form<TokenForm>, FormRejection>,
) -> Response {
    let Form(form) = match form {
        Ok(form) => form,
        Err(_) => return oauth_error("invalid_request", "Invalid token form."),
    };
    if state.public_url().is_none() {
        return oauth_configuration_required();
    }
    if form.grant_type.as_deref() != Some("authorization_code") {
        return oauth_error(
            "unsupported_grant_type",
            "Only authorization_code is supported.",
        );
    }
    let Some(code) = nonempty(form.code.as_deref()) else {
        return oauth_error("invalid_request", "Missing code.");
    };
    let Some(code_verifier) = nonempty(form.code_verifier.as_deref()) else {
        return oauth_error("invalid_request", "Missing code_verifier.");
    };
    let Some(redirect_uri) = nonempty(form.redirect_uri.as_deref()) else {
        return oauth_error("invalid_request", "Missing redirect_uri.");
    };
    let Some(client_id) = nonempty(form.client_id.as_deref()) else {
        return oauth_error("invalid_request", "Missing client_id.");
    };

    let _mutation = state.mutation_lock.lock();
    let stored = state.oauth_codes.lock().remove(&sha256_hex(code));
    let Some(stored) = stored else {
        return oauth_error("invalid_grant", "Invalid code.");
    };
    if stored.expires_at <= Utc::now() {
        return oauth_error("invalid_grant", "Code expired.");
    }
    if stored.client_id != client_id || stored.redirect_uri != redirect_uri {
        return oauth_error(
            "invalid_grant",
            "Code does not match client or redirect URI.",
        );
    }
    if form
        .resource
        .as_deref()
        .is_some_and(|resource| resource != stored.resource)
    {
        return oauth_error(
            "invalid_target",
            "Resource does not match authorization request.",
        );
    }
    if form
        .scope
        .as_deref()
        .is_some_and(|scope| !scope.trim().is_empty() && scope.trim() != OAUTH_SCOPE)
    {
        return oauth_error("invalid_scope", "Unsupported scope.");
    }
    if !verify_pkce_s256(code_verifier, &stored.code_challenge) {
        return oauth_error("invalid_grant", "PKCE verification failed.");
    }

    let access_token = opaque_value("token");
    state.oauth_tokens.lock().insert(
        sha256_hex(&access_token),
        OAuthAccessToken {
            expires_at: Utc::now() + chrono::Duration::seconds(TOKEN_TTL_SECONDS),
            resource: stored.resource,
            scope: stored.scope.clone(),
        },
    );
    let mut response = json_response(json!(TokenResponse {
        access_token,
        token_type: "Bearer",
        expires_in: TOKEN_TTL_SECONDS,
        scope: stored.scope,
    }));
    add_no_store_headers(&mut response, true);
    response
}

struct ValidatedAuthorize {
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    scope: String,
    resource: String,
}

fn validate_authorize_params(
    state: &HttpMcpAuthState,
    params: &AuthorizeParams,
) -> Result<ValidatedAuthorize, AuthorizeError> {
    let Some(resource) = state.resource_url() else {
        return Err(AuthorizeError::required_public_url());
    };
    if params.response_type.as_deref() != Some("code") {
        return Err(AuthorizeError::new(
            "invalid_request",
            "Only response_type=code is supported.",
        ));
    }
    let client_id = params
        .client_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AuthorizeError::new("invalid_request", "Missing client_id."))?;
    let redirect_uri = params
        .redirect_uri
        .as_deref()
        .filter(|value| is_allowed_chatgpt_redirect_uri(value))
        .ok_or_else(|| {
            AuthorizeError::new(
                "invalid_request",
                "redirect_uri is not an allowed ChatGPT callback.",
            )
        })?;
    if params.code_challenge_method.as_deref() != Some("S256") {
        return Err(AuthorizeError::new(
            "invalid_request",
            "Only PKCE S256 is supported.",
        ));
    }
    let code_challenge = params
        .code_challenge
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AuthorizeError::new("invalid_request", "Missing code_challenge."))?;
    let scope = normalized_scope(params.scope.as_deref()).ok_or_else(|| {
        AuthorizeError::new("invalid_scope", "Only the agentic:mcp scope is supported.")
    })?;
    if params
        .resource
        .as_deref()
        .is_some_and(|value| value != resource)
    {
        return Err(AuthorizeError::new(
            "invalid_target",
            format!("resource must be {resource}"),
        ));
    }
    Ok(ValidatedAuthorize {
        client_id: client_id.to_string(),
        redirect_uri: redirect_uri.to_string(),
        code_challenge: code_challenge.to_string(),
        scope,
        resource,
    })
}

fn normalized_scope(scope: Option<&str>) -> Option<String> {
    match scope.map(str::trim) {
        None | Some("") => Some(OAUTH_SCOPE.to_string()),
        Some(value) if value == OAUTH_SCOPE => Some(OAUTH_SCOPE.to_string()),
        Some(_) => None,
    }
}

fn is_allowed_chatgpt_redirect_uri(value: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    let Some(authority) = value
        .strip_prefix("https://")
        .and_then(|rest| rest.split(['/', '?', '#']).next())
    else {
        return false;
    };
    if authority != "chatgpt.com"
        || url.scheme() != "https"
        || url.host_str() != Some("chatgpt.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    let path = url.path();
    if path == "/connector_platform_oauth_redirect" {
        return true;
    }
    let prefix = "/connector/oauth/";
    path.starts_with(prefix)
        && path[prefix.len()..]
            .chars()
            .any(|character| character != '/')
}

fn verify_pkce_s256(code_verifier: &str, code_challenge: &str) -> bool {
    let digest = Sha256::digest(code_verifier.as_bytes());
    let computed = URL_SAFE_NO_PAD.encode(digest);
    constant_time_equal(&computed, code_challenge)
}

fn opaque_value(kind: &str) -> String {
    format!("ag_{kind}_{}", Uuid::new_v4().simple())
}

fn sha256_hex(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn constant_time_equal(left: &str, right: &str) -> bool {
    let left_digest = Sha256::digest(left.as_bytes());
    let right_digest = Sha256::digest(right.as_bytes());
    let mut difference = left.len() ^ right.len();
    for index in 0..left_digest.len() {
        difference |= usize::from(left_digest[index] ^ right_digest[index]);
    }
    difference == 0
}

fn parse_bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(char::is_whitespace)?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("Bearer") && !token.is_empty()).then_some(token)
}

fn unauthorized_response(state: &HttpMcpAuthState) -> Response {
    let mut response = (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    if let Ok(value) = HeaderValue::from_str(&state.challenge()) {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, value);
    }
    response
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NormalizedAuthority {
    host: String,
    port: Option<u16>,
}

fn normalize_host(host: &str) -> String {
    host.trim_matches('[')
        .trim_matches(']')
        .to_ascii_lowercase()
}

fn normalize_authority(host: &str, port: Option<u16>) -> NormalizedAuthority {
    NormalizedAuthority {
        host: normalize_host(host),
        port,
    }
}

fn parse_allowed_authority(value: &str) -> Option<NormalizedAuthority> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(authority) = Authority::try_from(value) {
        return Some(normalize_authority(authority.host(), authority.port_u16()));
    }
    Some(normalize_authority(value, None))
}

fn parse_host_header(uri: &Uri, headers: &HeaderMap) -> Result<NormalizedAuthority, Box<Response>> {
    if let Some(value) = headers.get(header::HOST) {
        let value = value.to_str().map_err(|_| {
            boxed_response(bad_request_response(
                "Bad Request: Invalid Host header encoding",
            ))
        })?;
        let authority = Authority::try_from(value).map_err(|_| {
            boxed_response(bad_request_response("Bad Request: Invalid Host header"))
        })?;
        if authority.host().is_empty() {
            return Err(boxed_response(bad_request_response(
                "Bad Request: Invalid Host header",
            )));
        }
        return Ok(normalize_authority(authority.host(), authority.port_u16()));
    }
    let authority = uri
        .authority()
        .ok_or_else(|| boxed_response(bad_request_response("Bad Request: missing Host header")))?;
    if authority.host().is_empty() {
        return Err(boxed_response(bad_request_response(
            "Bad Request: Invalid Host header",
        )));
    }
    Ok(normalize_authority(authority.host(), authority.port_u16()))
}

fn host_is_allowed(host: &NormalizedAuthority, allow_hosts: Option<&[String]>) -> bool {
    let Some(allow_hosts) = allow_hosts else {
        return true;
    };
    if allow_hosts.is_empty() || (allow_hosts.len() == 1 && allow_hosts[0].trim() == "*") {
        return true;
    }
    allow_hosts
        .iter()
        .filter_map(|value| parse_allowed_authority(value))
        .any(|allowed| {
            allowed.host == host.host && allowed.port.is_none_or(|port| host.port == Some(port))
        })
}

fn validate_host_origin(
    uri: &Uri,
    headers: &HeaderMap,
    policy: &HttpMcpHostPolicy,
) -> Result<(), Box<Response>> {
    let host = parse_host_header(uri, headers)?;
    if !host_is_allowed(&host, policy.allow_hosts.as_deref()) {
        return Err(boxed_response(forbidden_response(
            "Forbidden: Host header is not allowed",
        )));
    }
    validate_origin_header(headers, policy.public_url.as_deref())
}

fn validate_origin_header(
    headers: &HeaderMap,
    public_url: Option<&str>,
) -> Result<(), Box<Response>> {
    let Some(value) = headers.get(header::ORIGIN) else {
        return Ok(());
    };
    let value = value.to_str().map_err(|_| {
        boxed_response(bad_request_response(
            "Bad Request: Invalid Origin header encoding",
        ))
    })?;
    let origin = Url::parse(value)
        .map_err(|_| boxed_response(bad_request_response("Bad Request: Invalid Origin header")))?;
    if !origin.username().is_empty()
        || origin.password().is_some()
        || origin.query().is_some()
        || origin.fragment().is_some()
        || !(origin.path().is_empty() || origin.path() == "/")
        || origin.host_str().is_none()
    {
        return Err(boxed_response(bad_request_response(
            "Bad Request: Invalid Origin header",
        )));
    }
    let Some(public_url) = public_url else {
        return Err(boxed_response(forbidden_response(
            "Forbidden: Origin header is not allowed",
        )));
    };
    let expected = Url::parse(public_url).map_err(|_| {
        boxed_response(bad_request_response(
            "Bad Request: Invalid configured origin",
        ))
    })?;
    let matches = origin.scheme().eq_ignore_ascii_case(expected.scheme())
        && origin.host_str().is_some_and(|host| {
            expected
                .host_str()
                .is_some_and(|expected_host| host.eq_ignore_ascii_case(expected_host))
        })
        && origin.port_or_known_default() == expected.port_or_known_default();
    if matches {
        Ok(())
    } else {
        Err(boxed_response(forbidden_response(
            "Forbidden: Origin header is not allowed",
        )))
    }
}

fn bad_request_response(message: &str) -> Response {
    let mut response = (StatusCode::BAD_REQUEST, message.to_string()).into_response();
    add_no_store_headers(&mut response, false);
    response
}

fn forbidden_response(message: &str) -> Response {
    let mut response = (StatusCode::FORBIDDEN, message.to_string()).into_response();
    add_no_store_headers(&mut response, false);
    response
}

fn boxed_response(response: Response) -> Box<Response> {
    Box::new(response)
}

fn authorize_page(error: Option<&AuthorizeError>, params: &AuthorizeParams) -> Response {
    html_response(render_authorize_page(error, params))
}

fn render_authorize_page(error: Option<&AuthorizeError>, params: &AuthorizeParams) -> String {
    let error_html = error
        .map(|error| {
            format!(
                "<p class=\"error\"><strong>{}</strong>: {}</p>",
                html_escape(error.code),
                html_escape(&error.message)
            )
        })
        .unwrap_or_default();
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>Authorize standalone HTTP MCP</title>
<style>
body {{ font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; max-width: 560px; margin: 64px auto; padding: 0 20px; line-height: 1.5; }}
label {{ display: block; margin: 18px 0 8px; font-weight: 600; }}
input[type=password] {{ width: 100%; box-sizing: border-box; padding: 10px 12px; font: inherit; }}
button {{ margin-top: 18px; padding: 10px 14px; font: inherit; cursor: pointer; }}
.error {{ color: #b00020; }}
.small {{ color: #666; font-size: 0.92em; }}
</style>
</head>
<body>
<h1>Authorize standalone HTTP MCP</h1>
<p>This standalone HTTP MCP <code>/mcp</code> endpoint is requesting authorization for the ChatGPT connector.</p>
{error_html}
<form method="post" action="/oauth/authorize">
<label for="bearer_token">HTTP MCP bearer token</label>
<input id="bearer_token" name="bearer_token" type="password" autocomplete="current-password" autofocus required />
{hidden_fields}
<button type="submit">Authorize</button>
</form>
<p class="small">The access token lasts 7 days. Tool execution remains subject to local policy, confirmation, and audit.</p>
</body>
</html>"#,
        hidden_fields = hidden_fields(params),
    )
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

fn html_error_response(error: &AuthorizeError) -> Response {
    html_response(format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8" /><title>Standalone OAuth error</title></head>
<body><h1>Authorization failed</h1><p><strong>{}</strong>: {}</p></body></html>"#,
        html_escape(error.code),
        html_escape(&error.message)
    ))
}

fn html_response(body: String) -> Response {
    let mut response = Html(body).into_response();
    add_no_store_headers(&mut response, false);
    response
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn json_response(value: serde_json::Value) -> Response {
    let mut response = axum::Json(value).into_response();
    add_no_store_headers(&mut response, false);
    response
}

fn oauth_configuration_required() -> Response {
    oauth_error(
        "http_mcp_public_url_required",
        "A configured HTTPS public URL is required for ChatGPT OAuth.",
    )
}

fn oauth_error(error: &'static str, description: &'static str) -> Response {
    let mut response = (
        StatusCode::BAD_REQUEST,
        axum::Json(json!({
            "error": error,
            "error_description": description,
        })),
    )
        .into_response();
    add_no_store_headers(&mut response, true);
    response
}

fn add_no_store_headers(response: &mut Response, pragma: bool) {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if pragma {
        response
            .headers_mut()
            .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    }
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn pkce_rfc7636_s256_vector_matches() {
        assert!(verify_pkce_s256(
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        ));
        assert!(!verify_pkce_s256(
            "wrong",
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        ));
    }

    #[test]
    fn chatgpt_redirect_families_are_exact() {
        assert!(is_allowed_chatgpt_redirect_uri(
            "https://chatgpt.com/connector/oauth/example"
        ));
        assert!(is_allowed_chatgpt_redirect_uri(
            "https://chatgpt.com/connector_platform_oauth_redirect"
        ));
        for value in [
            "https://chatgpt.com/connector/oauth/",
            "https://chatgpt.com/connector/oauth/example?x=1",
            "https://chatgpt.com/connector/oauth/example#fragment",
            "https://chatgpt.com:443/connector/oauth/example",
            "https://evil.example/connector/oauth/example",
            "http://chatgpt.com/connector/oauth/example",
            "https://user@chatgpt.com/connector/oauth/example",
        ] {
            assert!(!is_allowed_chatgpt_redirect_uri(value), "{value}");
        }
    }

    #[test]
    fn public_url_normalization_uses_config_contract() {
        assert_eq!(
            crate::config::normalize_http_mcp_public_url(" https://example.com/ ").unwrap(),
            "https://example.com"
        );
        for value in [
            "http://example.com",
            "https://",
            "https://example.com/path",
            "https://user@example.com",
            "https://example.com/..",
            "https://example.com/./",
            "https://example.com//",
            "https://example.com?query=1",
            "https://example.com#fragment",
        ] {
            assert!(
                crate::config::normalize_http_mcp_public_url(value).is_err(),
                "{value}"
            );
        }
    }

    #[test]
    fn host_and_origin_matching_follow_rmcp_policy() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("[::1]:8765"));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://example.com"),
        );
        let policy = HttpMcpHostPolicy::new(
            Some("https://example.com/".to_string()),
            Some(vec!["::1".to_string()]),
        );
        assert!(validate_host_origin(&Uri::from_static("/mcp"), &headers, &policy).is_ok());
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://foreign.example"),
        );
        assert_eq!(
            validate_host_origin(&Uri::from_static("/mcp"), &headers, &policy)
                .unwrap_err()
                .status(),
            StatusCode::FORBIDDEN
        );
        headers.remove(header::ORIGIN);
        assert!(validate_host_origin(&Uri::from_static("/mcp"), &headers, &policy).is_ok());
    }

    #[test]
    fn token_rotation_revokes_oauth_state_but_direct_bearer_changes() {
        let state =
            HttpMcpAuthState::new("first".to_string(), Some("https://example.com".to_string()));
        let resource = state.resource_url().unwrap();
        state.oauth_tokens.lock().insert(
            sha256_hex("oauth"),
            OAuthAccessToken {
                expires_at: Utc::now() + chrono::Duration::minutes(1),
                resource: resource.clone(),
                scope: OAUTH_SCOPE.to_string(),
            },
        );
        assert!(state.accepts_bearer("first", None));
        assert!(state.accepts_bearer("oauth", Some(&resource)));
        assert!(!state.accepts_bearer("oauth", Some("https://evil.example/mcp")));
        assert!(state.replace_resolved_token("second".to_string()));
        assert!(!state.accepts_bearer("first", None));
        assert!(!state.accepts_bearer("oauth", Some(&resource)));
        assert!(state.accepts_bearer("second", None));
    }

    #[test]
    fn oauth_tokens_reject_wrong_resource_binding() {
        let state = HttpMcpAuthState::new(
            "direct".to_string(),
            Some("https://example.com".to_string()),
        );
        let canonical = state.resource_url().unwrap();
        state.oauth_tokens.lock().insert(
            sha256_hex("wrong-audience"),
            OAuthAccessToken {
                expires_at: Utc::now() + chrono::Duration::minutes(1),
                resource: "https://evil.example/mcp".to_string(),
                scope: OAUTH_SCOPE.to_string(),
            },
        );
        assert!(!state.accepts_bearer("wrong-audience", Some(&canonical)));
    }

    #[test]
    fn standalone_page_escapes_hidden_fields_and_avoids_hub_copy() {
        let params = AuthorizeParams {
            response_type: Some("code".to_string()),
            client_id: Some("client<&\"'".to_string()),
            redirect_uri: Some("https://chatgpt.com/connector/oauth/example".to_string()),
            state: Some("state<&\"'".to_string()),
            scope: Some(OAUTH_SCOPE.to_string()),
            code_challenge: Some("challenge".to_string()),
            code_challenge_method: Some("S256".to_string()),
            resource: Some("https://example.com/mcp".to_string()),
        };
        let body = render_authorize_page(None, &params);
        assert!(body.contains("HTTP MCP bearer token"));
        assert!(!body.contains("Hub API key"));
        assert!(!body.contains("name=\"api_key\""));
        assert!(hidden_fields(&params).contains("&lt;"));
    }

    #[test]
    fn expired_oauth_tokens_are_pruned_and_rejected() {
        let state = HttpMcpAuthState::new(
            "direct".to_string(),
            Some("https://example.com".to_string()),
        );
        let resource = state.resource_url().unwrap();
        state.oauth_tokens.lock().insert(
            sha256_hex("expired"),
            OAuthAccessToken {
                expires_at: Utc::now() - chrono::Duration::seconds(1),
                resource: resource.clone(),
                scope: OAUTH_SCOPE.to_string(),
            },
        );
        assert!(!state.accepts_bearer("expired", Some(&resource)));
        assert!(state.oauth_tokens.lock().is_empty());
    }

    #[tokio::test]
    async fn expired_authorization_codes_are_consumed_and_rejected() {
        let state = HttpMcpAuthState::new(
            "direct".to_string(),
            Some("https://example.com".to_string()),
        );
        let code = "expired-code";
        state.oauth_codes.lock().insert(
            sha256_hex(code),
            OAuthAuthorizationCode {
                client_id: "client".to_string(),
                redirect_uri: "https://chatgpt.com/connector/oauth/test".to_string(),
                code_challenge: "challenge".to_string(),
                scope: OAUTH_SCOPE.to_string(),
                resource: state.resource_url().unwrap(),
                expires_at: Utc::now() - chrono::Duration::seconds(1),
            },
        );
        let response = token(
            State(state.clone()),
            Ok(Form(TokenForm {
                grant_type: Some("authorization_code".to_string()),
                code: Some(code.to_string()),
                redirect_uri: Some("https://chatgpt.com/connector/oauth/test".to_string()),
                client_id: Some("client".to_string()),
                code_verifier: Some("verifier".to_string()),
                resource: None,
                scope: None,
            })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"], "invalid_grant");
        assert!(value.get("access_token").is_none());
        assert!(state.oauth_codes.lock().is_empty());
    }

    #[test]
    fn oauth_error_and_html_responses_are_uncacheable() {
        let error = oauth_error("invalid_request", "bad request");
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        assert_eq!(error.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(error.headers()[header::PRAGMA], "no-cache");
        let page = authorize_page(
            None,
            &AuthorizeParams {
                response_type: None,
                client_id: None,
                redirect_uri: None,
                state: None,
                scope: None,
                code_challenge: None,
                code_challenge_method: None,
                resource: None,
            },
        );
        assert_eq!(page.headers()[header::CACHE_CONTROL], "no-store");
    }
}
