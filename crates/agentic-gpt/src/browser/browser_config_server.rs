use anyhow::Result;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

pub(crate) fn run() -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    for line in stdin.lock().lines() {
        let request: Value = serde_json::from_str(&line?)?;
        let Some(id) = request.get("id") else {
            continue;
        };
        let result = match request.get("method").and_then(Value::as_str) {
            Some("initialize") => Some(json!({
                "userAgent": concat!("agentic-gpt/", env!("CARGO_PKG_VERSION")),
                "codexHome": std::env::var("CODEX_HOME")?,
                "platformFamily": std::env::consts::FAMILY,
                "platformOs": std::env::consts::OS,
            })),
            Some("config/read") => Some(json!({"config": {}, "origins": {}})),
            Some("configRequirements/read") => Some(json!({"requirements": null})),
            _ => None,
        };
        let response = match result {
            Some(result) => json!({"id": id, "result": result}),
            None => json!({"id": id, "error": {"code": -32601, "message": "Method not found"}}),
        };
        serde_json::to_writer(&mut output, &response)?;
        writeln!(output)?;
        output.flush()?;
    }
    Ok(())
}
