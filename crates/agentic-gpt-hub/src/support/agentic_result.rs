use rmcp::model::{CallToolResult, ContentBlock, MetaObject};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgenticResult {
    #[serde(default)]
    content: Vec<ContentBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    structured_content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_error: Option<bool>,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    meta: Option<MetaObject>,
}

impl AgenticResult {
    pub(crate) fn from_native_value(value: Value) -> Self {
        let is_error = value.get("error").is_some();
        Self {
            content: vec![ContentBlock::text(content_text(&value))],
            structured_content: Some(value),
            is_error: Some(is_error),
            meta: None,
        }
    }

    pub(crate) fn from_native_email_value(mut value: Value) -> Self {
        let images = value
            .as_object_mut()
            .and_then(|value| value.remove("images"));
        let mut content = vec![ContentBlock::text(content_text(&value))];
        if let Some(images) = images.and_then(|images| images.as_array().cloned()) {
            for image in images {
                if let (Some(data), Some(mime_type)) =
                    (image["data"].as_str(), image["mimeType"].as_str())
                {
                    content.push(ContentBlock::image(data.to_owned(), mime_type.to_owned()));
                }
            }
        }
        let is_error = value.get("error").is_some();
        let result = Self {
            content,
            structured_content: Some(value),
            is_error: Some(is_error),
            meta: None,
        };
        // These fields serialize exactly as the CallToolResult built below.
        // Count the escaped JSON in text content, metadata and image blocks.
        if serde_json::to_vec(&result)
            .map(|bytes| bytes.len() > 8 * 1024 * 1024)
            .unwrap_or(true)
        {
            let mut error = serde_json::json!({"error": {
                "code": "email_response_too_large",
                "message": "邮件响应超出 8 MiB 的 MCP 结果上限。"
            }});
            if let Some(events) = result
                .structured_content
                .as_ref()
                .and_then(|value| value.get("events"))
            {
                error["events"] = events.clone();
            }
            return Self::from_native_value(error);
        }
        result
    }

    pub(crate) fn into_call_tool_result(self) -> CallToolResult {
        let mut result = CallToolResult::default();
        result.content = self.content;
        result.structured_content = self.structured_content;
        result.is_error = self.is_error;
        result.meta = self.meta;
        result
    }
}

fn content_text(value: &Value) -> String {
    match serde_json::to_string(value) {
        Ok(text) => text,
        Err(error) => {
            format!(r#"{{"error":"failed_to_serialize_agentic_result","message":"{error}"}}"#)
        }
    }
}
