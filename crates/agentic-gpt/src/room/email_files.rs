use crate::{file_ops, state::AppState};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};

pub(crate) async fn read_attachment(
    _state: &AppState,
    _name: &str,
    mime_type: &str,
    bytes: &[u8],
) -> Value {
    match file_ops::read_image_bytes(bytes, file_ops::MAX_IMAGE_RESPONSE_BYTES / 2) {
        Ok(Some(output)) => {
            return json!({"images": output.images.into_iter().map(|image| json!({"mimeType": image.mime_type, "data": STANDARD.encode(image.bytes)})).collect::<Vec<_>>()})
        }
        Err(error) => return error.value(),
        Ok(None) => {}
    }
    if mime_type.starts_with("text/") && bytes.len() <= 64 * 1024 {
        if let Ok(text) = std::str::from_utf8(bytes) {
            if !text
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
            {
                return json!({"body": text, "format": "plain"});
            }
        }
    }
    json!({"error": {"code": "email_attachment_download_required", "message": "附件无法安全内联；请使用 download=true 保存后处理。"}})
}

pub(crate) async fn save_attachment(
    state: &AppState,
    mail_id: &str,
    name: &str,
    path: Option<&str>,
    bytes: &[u8],
) -> Value {
    let safe_name: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':') {
                '_'
            } else {
                c
            }
        })
        .take(180)
        .collect();
    let safe_name = if safe_name.trim().is_empty() || matches!(safe_name.as_str(), "." | "..") {
        "attachment"
    } else {
        &safe_name
    };
    // IDs are generated internally, but reject malformed identities before constructing paths.
    if mail_id.is_empty()
        || !mail_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
    {
        return json!({"error":{"code":"email_id_invalid","message":"invalid mail id"}});
    }
    let default_path = format!("downloads/email/{mail_id}/{safe_name}");
    match file_ops::create_binary_in_workspace(state, path.unwrap_or(&default_path), bytes).await {
        Ok(path) => json!({"path": path}),
        Err(error) => error.value(),
    }
}
