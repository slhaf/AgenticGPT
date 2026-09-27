use rmcp::model::{CallToolResult, Content, Meta};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgenticResult {
    #[serde(default)]
    content: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    structured_content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_error: Option<bool>,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    meta: Option<Meta>,
}

impl AgenticResult {
    pub(crate) fn from_native_value(value: Value) -> Self {
        let is_error = value.get("error").is_some();
        Self {
            content: vec![Content::text(content_text(&value))],
            structured_content: Some(value),
            is_error: Some(is_error),
            meta: None,
        }
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::AgenticResult;

    #[test]
    fn wraps_native_json_as_structured_success_when_value_has_no_error() {
        let value = json!({ "status": "ok", "count": 2 });

        let result = AgenticResult::from_native_value(value.clone()).into_call_tool_result();
        let serialized = serde_json::to_value(result).expect("serialize result");

        assert_eq!(serialized["structuredContent"], value);
        assert_eq!(serialized["isError"], false);
        assert_eq!(serialized["content"][0]["type"], "text");
    }

    #[test]
    fn wraps_native_json_as_structured_error_when_value_has_error() {
        let value = json!({ "error": { "code": "boom", "message": "failed" } });

        let result = AgenticResult::from_native_value(value.clone()).into_call_tool_result();
        let serialized = serde_json::to_value(result).expect("serialize result");

        assert_eq!(serialized["structuredContent"], value);
        assert_eq!(serialized["isError"], true);
    }
}
