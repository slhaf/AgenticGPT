//! 输出 Config API 的 OpenAPI components.schemas；请求字段只在共享 DTO 中定义。
use agentic_gpt_protocol::{
    ConfigAgentRequest, ConfigGetRequest, ConfigGetResponse, ConfigManualRequest,
    ConfigManualResponse, ConfigUpdateRequest, ConfigUpdateResponse, EventPanel,
};
use schemars::JsonSchema;
use serde_json::{json, Map, Value};

fn rewrite_references(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get_mut("$ref") {
                if let Some(name) = reference.as_str().and_then(|s| s.strip_prefix("#/$defs/")) {
                    *reference = Value::String(format!("#/components/schemas/{name}"));
                }
            }
            object.values_mut().for_each(rewrite_references);
        }
        Value::Array(values) => values.iter_mut().for_each(rewrite_references),
        _ => {}
    }
}

fn add<T: JsonSchema>(schemas: &mut Map<String, Value>, name: &str, public_frame: bool) {
    let mut schema = serde_json::to_value(schemars::schema_for!(T)).expect("schema serialization");
    let object = schema.as_object_mut().expect("object schema");
    object.remove("$schema");
    object.remove("title");
    if let Some(Value::Object(definitions)) = object.remove("$defs") {
        for (key, mut value) in definitions {
            rewrite_references(&mut value);
            schemas.insert(key, value);
        }
    }
    if public_frame {
        object
            .get_mut("properties")
            .and_then(Value::as_object_mut)
            .expect("response properties")
            .insert(
                "events".to_owned(),
                json!({"$ref":"#/components/schemas/EventPanel"}),
            );
    }
    rewrite_references(&mut schema);
    schemas.insert(name.to_owned(), schema);
}

fn main() {
    let mut schemas = Map::new();
    add::<ConfigAgentRequest<ConfigManualRequest>>(&mut schemas, "ConfigManualHttpRequest", false);
    add::<ConfigAgentRequest<ConfigGetRequest>>(&mut schemas, "ConfigGetHttpRequest", false);
    add::<ConfigAgentRequest<ConfigUpdateRequest>>(&mut schemas, "ConfigUpdateHttpRequest", false);
    add::<ConfigManualResponse>(&mut schemas, "ConfigManualResponse", true);
    add::<ConfigGetResponse>(&mut schemas, "ConfigGetResponse", true);
    add::<ConfigUpdateResponse>(&mut schemas, "ConfigUpdateResponse", true);
    add::<EventPanel>(&mut schemas, "EventPanel", false);
    println!(
        "{}",
        serde_json::to_string_pretty(&schemas).expect("schema JSON")
    );
}
