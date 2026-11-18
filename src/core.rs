use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

pub const ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/images/generations";
pub const MODEL: &str = "gpt-image-2";
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_RESPONSE_BYTES: usize = MAX_IMAGE_BYTES.div_ceil(3) * 4 + 64 * 1024;

/// Read at most the configured limit plus one sentinel byte. Abort an oversized
/// response before buffering the rest; transport errors never include payloads.
pub fn read_bounded(
    mut next: impl FnMut(u64) -> Result<Option<Vec<u8>>, String>,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    loop {
        let request = (limit - body.len()).saturating_add(1).min(64 * 1024);
        let Some(chunk) = next(request as u64)? else {
            return Ok(body);
        };
        if chunk.len() > limit - body.len() {
            return Err("Codex image response exceeds the size limit".into());
        }
        body.extend_from_slice(&chunk);
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    prompt: String,
    #[serde(default)]
    transparent_background: bool,
}

/// No Debug implementation: credentials must not appear in diagnostics.
pub struct Credentials {
    pub access_token: String,
    pub account_id: String,
}

pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

pub struct Artifact {
    pub path: String,
    pub size_bytes: u64,
}

/// Subscription operations and workspace upload, with sanitized errors.
pub trait Host {
    fn credentials(&mut self) -> Result<Credentials, String>;
    fn post(&mut self, credentials: &Credentials, body: Vec<u8>) -> Result<HttpResponse, String>;
    fn save_image(&mut self, bytes: &[u8]) -> Result<Artifact, String>;
}

pub fn schema() -> String {
    json!({
        "type": "object",
        "properties": {
            "action": {"type": "string", "enum": ["login_start", "login_poll", "generate", "status"]},
            "reconnect": {"type": "boolean"},
            "request_id": {"type": "string", "minLength": 1, "maxLength": 64},
            "prompt": {"type": "string", "minLength": 1, "maxLength": 32768},
            "transparent_background": {"type": "boolean", "default": false}
        },
        "required": ["action"],
        "additionalProperties": false
    })
    .to_string()
}

#[derive(Deserialize)]
struct ImageResponse {
    data: Vec<ImageData>,
}

#[derive(Deserialize)]
struct ImageData {
    b64_json: String,
}

pub fn generate(host: &mut impl Host, arguments: &str) -> Result<String, String> {
    let args = parse_image(arguments)?;
    let credentials = host.credentials()?;
    generate_image(host, &args, &credentials)
}

fn parse_image(arguments: &str) -> Result<Arguments, String> {
    let mut value: serde_json::Value =
        serde_json::from_str(arguments).map_err(|_| "Invalid tool arguments")?;
    // The host reserves this key, even when there is no user config to read.
    value
        .as_object_mut()
        .ok_or("Arguments must be an object")?
        .remove("__config");
    let args: Arguments = serde_json::from_value(value)
        .map_err(|_| "Expected prompt and optional transparent_background")?;
    if args.prompt.trim().is_empty() || args.prompt.len() > 32768 {
        return Err("Prompt must contain 1–32768 UTF-8 bytes".into());
    }
    Ok(args)
}

fn generate_image(
    host: &mut impl Host,
    args: &Arguments,
    credentials: &Credentials,
) -> Result<String, String> {
    for header in [&credentials.access_token, &credentials.account_id] {
        if header.is_empty() || !header.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
            return Err("Host returned invalid Codex credentials".into());
        }
    }
    let body = serde_json::to_vec(&json!({
        "model": MODEL,
        "prompt": args.prompt,
        "background": if args.transparent_background {"transparent"} else {"opaque"},
        "n": 1
    }))
    .map_err(|_| "Cannot encode image request")?;
    // Exactly one HTTP attempt: retrying a generation can consume quota twice.
    let response = host.post(credentials, body)?;
    if response.status != 200 {
        return Err(match response.status {
            401 => "Codex login is unavailable or expired; authorize the plugin again",
            403 => "Codex denied image generation for this account",
            429 => "Codex image generation is rate limited or out of quota",
            _ => "Codex image generation failed; no automatic retry was made",
        }
        .into());
    }
    if response.body.len() > MAX_RESPONSE_BYTES {
        return Err("Codex image response exceeds the size limit".into());
    }
    let response: ImageResponse =
        serde_json::from_slice(&response.body).map_err(|_| "Invalid Codex image response")?;
    if response.data.len() != 1 {
        return Err("Expected exactly one generated image".into());
    }
    let image = &response.data[0].b64_json;
    if image.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4 {
        return Err("Generated image exceeds the 8 MiB limit".into());
    }
    let bytes = STANDARD
        .decode(image)
        .map_err(|_| "Invalid generated image encoding")?;
    if bytes.len() > MAX_IMAGE_BYTES || !is_image(&bytes) {
        return Err("Generated image is invalid or too large".into());
    }
    let artifact = host.save_image(&bytes)?;
    if artifact.size_bytes != bytes.len() as u64
        || !artifact.path.starts_with('/')
        || artifact.path.contains(['\r', '\n', '[', ']'])
    {
        return Err("Host returned an invalid image artifact".into());
    }
    Ok(format!(
        "Image generated successfully.\n[IMAGE:{}]",
        artifact.path
    ))
}

fn is_image(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(b"\xff\xd8\xff")
        || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"))
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    LoginStart {
        #[serde(default)]
        reconnect: bool,
    },
    LoginPoll,
    Generate {
        request_id: String,
        prompt: String,
        #[serde(default)]
        transparent_background: bool,
    },
    Status {
        request_id: String,
    },
}

pub trait JobHost: Host {
    fn job_get(&mut self, key: &str) -> Result<Option<(Vec<u8>, u64)>, String>;
    fn job_put(&mut self, key: &str, value: &[u8], expected: Option<u64>) -> Result<u64, String>;
}

#[derive(Deserialize, Serialize)]
struct Job {
    fingerprint: [u8; 32],
    outcome: Outcome,
}

#[derive(Deserialize, Serialize)]
enum Outcome {
    Pending,
    Saved(String),
    Failed(String),
}

impl Job {
    fn read(host: &mut impl JobHost, key: &str) -> Result<Option<Self>, String> {
        host.job_get(key)?
            .map(|(bytes, _)| {
                serde_json::from_slice(&bytes).map_err(|_| "Stored image request is invalid".into())
            })
            .transpose()
    }

    fn write(
        &self,
        host: &mut impl JobHost,
        key: &str,
        revision: Option<u64>,
    ) -> Result<u64, String> {
        let bytes = serde_json::to_vec(self).map_err(|_| "Cannot encode image request state")?;
        host.job_put(key, &bytes, revision)
    }

    fn result(self) -> Result<String, String> {
        match self.outcome {
            Outcome::Pending => Err("Image request is running or its outcome is unknown. Reuse this request ID to check its status. A new ID can consume quota again.".into()),
            Outcome::Saved(marker) => Ok(marker),
            Outcome::Failed(error) => Err(error),
        }
    }
}

fn job_key(id: &str) -> Result<String, String> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(
            "Use a request ID of 1 to 64 ASCII letters, digits, hyphens, or underscores".into(),
        );
    }
    Ok(format!("image-{id}"))
}

pub fn status(host: &mut impl JobHost, id: &str) -> Result<String, String> {
    Job::read(host, &job_key(id)?)?
        .ok_or_else(|| "Image request ID not found".to_string())?
        .result()
}

pub fn generate_once(
    host: &mut impl JobHost,
    id: &str,
    prompt: String,
    transparent_background: bool,
) -> Result<String, String> {
    let key = job_key(id)?;
    if prompt.trim().is_empty() || prompt.len() > 32768 {
        return Err("Prompt must contain 1–32768 UTF-8 bytes".into());
    }
    let args = Arguments {
        prompt,
        transparent_background,
    };
    let input = serde_json::to_vec(&args).map_err(|_| "Cannot encode image request")?;
    let fingerprint = Sha256::digest(&input).into();
    if let Some(job) = Job::read(host, &key)? {
        if job.fingerprint != fingerprint {
            return Err("This request ID belongs to different image arguments".into());
        }
        return job.result();
    }

    let credentials = host.credentials()?;
    let mut job = Job {
        fingerprint,
        outcome: Outcome::Pending,
    };
    let revision = job.write(host, &key, None)?;
    job.outcome = match generate_image(host, &args, &credentials) {
        Ok(marker) => Outcome::Saved(marker),
        Err(error) => Outcome::Failed(error),
    };
    if job.write(host, &key, Some(revision)).is_err() {
        return Err("Image outcome could not be recorded. Check the workspace and reuse this request ID to avoid repeating generation.".into());
    }
    job.result()
}
