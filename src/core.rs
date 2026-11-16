//! Protocol and validation shared by the WASM shim and native tests.
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_port")]
    pub bridge_port: u16,
}

fn default_port() -> u16 {
    8787
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Generate {
        prompt: String,
        #[serde(default)]
        transparent_background: bool,
    },
    Status {
        job_id: String,
    },
}

pub struct Call {
    pub port: u16,
    pub request: Request,
}

pub fn parse_call(args: &str) -> Result<Call, String> {
    let mut value: Value = serde_json::from_str(args).map_err(|_| "Invalid arguments JSON")?;
    let config = value
        .as_object_mut()
        .ok_or("Arguments must be an object")?
        .remove("__config")
        .unwrap_or_else(|| json!({}));
    let config: Config = serde_json::from_value(config).map_err(|_| "Invalid plugin config")?;
    if config.bridge_port < 1024 {
        return Err("Bridge port must be at least 1024".into());
    }
    let request: Request = serde_json::from_value(value)
        .map_err(|_| "Use action=generate with prompt, or action=status with job_id")?;
    match &request {
        Request::Generate { prompt, .. } if prompt.trim().is_empty() || prompt.len() > 32768 => {
            return Err("Prompt must contain 1–32768 UTF-8 bytes".into())
        }
        Request::Status { job_id }
            if job_id.len() != 32 || !job_id.bytes().all(|b| b.is_ascii_hexdigit()) =>
        {
            return Err("Invalid job_id".into())
        }
        _ => (),
    }
    Ok(Call {
        port: config.bridge_port,
        request,
    })
}

pub fn parameters_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "action": {"type": "string", "enum": ["generate", "status"]},
            "prompt": {"type": "string", "description": "Image description; required for generate.", "minLength": 1, "maxLength": 32768},
            "transparent_background": {"type": "boolean", "default": false},
            "job_id": {"type": "string", "description": "Returned job ID; required for status.", "pattern": "^[0-9a-f]{32}$"}
        },
        "required": ["action"],
        "additionalProperties": false
    })
}

/// Do not forward untrusted bridge/proxy error bodies into the conversation.
pub fn response_output(status: u16, body: &[u8]) -> Result<String, String> {
    if !matches!(status, 200 | 202) {
        return Err(match status {
            401 => "Bridge authentication failed; check bridge_token",
            404 => "Image job not found; the bridge may have restarted or expired it",
            429 => "Bridge is busy; retry later",
            _ => "Bridge request failed; check the bridge service",
        }
        .into());
    }
    let value: Value = serde_json::from_slice(body).map_err(|_| "Invalid bridge response")?;
    if !matches!(
        value.get("status").and_then(Value::as_str),
        Some("queued" | "running" | "completed" | "failed")
    ) {
        return Err("Invalid bridge job status".into());
    }
    if value["status"] == "failed" {
        return Err(value["error"]
            .as_str()
            .unwrap_or("Codex image generation failed")
            .to_string());
    }
    let mut output = serde_json::to_string(&value).map_err(|_| "Invalid bridge response")?;
    if value["status"] == "completed" {
        let path = value["path"]
            .as_str()
            .ok_or("Missing generated image path")?;
        if !path.starts_with('/') || path.contains(['\r', '\n', '[', ']']) {
            return Err("Invalid generated image path".into());
        }
        output.push_str(&format!("\n[IMAGE:{path}]"));
    }
    Ok(output)
}
