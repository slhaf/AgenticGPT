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

    pub(crate) fn from_mcp_or_native_value(value: Value) -> Self {
        match serde_json::from_value::<CallToolResult>(value.clone()) {
            Ok(result) => Self::from_call_tool_result(result),
            Err(_) => Self::from_native_value(value),
        }
    }

    pub(crate) fn from_call_tool_result(result: CallToolResult) -> Self {
        Self {
            content: result.content,
            structured_content: result.structured_content,
            is_error: result.is_error,
            meta: result.meta,
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
    use rmcp::model::{CallToolResult, Content, Meta, RawResource};
    use serde_json::{json, Value};

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

    #[test]
    fn passes_through_downstream_mcp_image_content() {
        let value = json!({
            "content": [{ "type": "image", "data": "aW1hZ2U=", "mimeType": "image/png" }],
            "isError": false
        });

        let result = AgenticResult::from_mcp_or_native_value(value).into_call_tool_result();
        let serialized = serde_json::to_value(result).expect("serialize result");

        assert_eq!(serialized["content"][0]["type"], "image");
        assert_eq!(serialized["content"][0]["data"], "aW1hZ2U=");
        assert_eq!(serialized["content"][0]["mimeType"], "image/png");
        assert!(serialized.get("structuredContent").is_none());
    }

    #[test]
    fn passes_through_downstream_mcp_resource_link_content() {
        let value = json!({
            "content": [{
                "type": "resource_link",
                "uri": "https://example.test/report.pdf",
                "name": "report.pdf",
                "mimeType": "application/pdf",
                "size": 1024
            }],
            "isError": false
        });

        let result = AgenticResult::from_mcp_or_native_value(value).into_call_tool_result();
        let serialized = serde_json::to_value(result).expect("serialize result");

        assert_eq!(serialized["content"][0]["type"], "resource_link");
        assert_eq!(
            serialized["content"][0]["uri"],
            "https://example.test/report.pdf"
        );
        assert_eq!(serialized["content"][0]["name"], "report.pdf");
        assert_eq!(serialized["content"][0]["mimeType"], "application/pdf");
        assert_eq!(serialized["content"][0]["size"], 1024);
    }

    #[test]
    fn passes_through_downstream_mcp_result_meta_at_top_level() {
        let value = json!({
            "content": [{ "type": "text", "text": "visible" }],
            "_meta": { "private": "widget-only" },
            "isError": false
        });

        let result = AgenticResult::from_mcp_or_native_value(value).into_call_tool_result();
        let serialized = serde_json::to_value(result).expect("serialize result");

        assert_eq!(serialized["_meta"]["private"], "widget-only");
        assert!(serialized.get("structuredContent").is_none());
    }

    #[test]
    fn falls_back_to_native_wrapping_when_value_is_not_mcp_result_envelope() {
        let value = json!({ "plain": "json" });

        let result = AgenticResult::from_mcp_or_native_value(value.clone()).into_call_tool_result();
        let serialized = serde_json::to_value(result).expect("serialize result");

        assert_eq!(serialized["structuredContent"], value);
        assert_eq!(serialized["content"][0]["type"], "text");
    }

    #[test]
    fn converts_from_call_tool_result_without_losing_fields() {
        let mut meta = Meta::new();
        meta.insert("hidden".to_string(), json!(true));
        let mut resource = RawResource::new("https://example.test/data.json", "data.json");
        resource.mime_type = Some("application/json".to_string());
        resource.size = Some(42);
        let call_tool_result =
            CallToolResult::success(vec![Content::text("ok"), Content::resource_link(resource)])
                .with_meta(Some(meta));

        let result = AgenticResult::from_call_tool_result(call_tool_result).into_call_tool_result();
        let serialized: Value = serde_json::to_value(result).expect("serialize result");

        assert_eq!(serialized["content"][0]["type"], "text");
        assert_eq!(serialized["content"][1]["type"], "resource_link");
        assert_eq!(serialized["_meta"]["hidden"], true);
    }
}
