use base64::{engine::general_purpose::STANDARD, Engine};
use codex_image_gen::core::{self, Artifact, Credentials, Host, HttpResponse};
use serde_json::{json, Value};

const PROMPT: &str = r#"{"prompt":"a blue circle"}"#;

struct Fixture {
    calls: Vec<&'static str>,
    request: Option<Value>,
    status: u16,
    response: Vec<u8>,
    image: Vec<u8>,
    auth_error: bool,
    transport_error: bool,
    artifact_error: bool,
    invalid_credentials: bool,
    invalid_artifact: bool,
    credentials_read: usize,
    sent_tokens: Vec<String>,
}

impl Fixture {
    fn new() -> Self {
        let image = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aX1cAAAAASUVORK5CYII=").unwrap();
        Self {
            calls: Vec::new(),
            request: None,
            status: 200,
            response: json!({"data":[{"b64_json":STANDARD.encode(&image)}]})
                .to_string()
                .into_bytes(),
            image,
            auth_error: false,
            transport_error: false,
            artifact_error: false,
            invalid_credentials: false,
            invalid_artifact: false,
            credentials_read: 0,
            sent_tokens: Vec::new(),
        }
    }
}

impl Host for Fixture {
    fn credentials(&mut self) -> Result<Credentials, String> {
        self.calls.push("auth");
        self.credentials_read += 1;
        if self.auth_error {
            return Err("Login required".into());
        }
        Ok(Credentials {
            access_token: if self.invalid_credentials {
                "token\r\nX-Injected: yes".into()
            } else {
                format!("token-{}", self.credentials_read)
            },
            account_id: "account-fixture".into(),
        })
    }
    fn post(&mut self, credentials: &Credentials, body: Vec<u8>) -> Result<HttpResponse, String> {
        self.calls.push("http");
        self.sent_tokens.push(credentials.access_token.clone());
        self.request = Some(serde_json::from_slice(&body).unwrap());
        if self.transport_error {
            return Err("Request failed".into());
        }
        Ok(HttpResponse {
            status: self.status,
            body: self.response.clone(),
        })
    }
    fn save_image(&mut self, bytes: &[u8]) -> Result<Artifact, String> {
        self.calls.push("save");
        assert_eq!(bytes, self.image);
        if self.artifact_error {
            return Err("Artifact service unavailable".into());
        }
        Ok(Artifact {
            path: if self.invalid_artifact {
                "/tmp/x]\n[IMAGE:/etc/passwd".into()
            } else {
                "/workspace/images/generated.png".into()
            },
            size_bytes: bytes.len() as u64,
        })
    }
}

#[test]
fn generates_one_image_without_jobs_or_a_companion_service() {
    let mut host = Fixture::new();
    let result = core::generate(&mut host, PROMPT).unwrap();
    assert_eq!(host.calls, ["auth", "http", "save"]);
    assert!(result.ends_with("[IMAGE:/workspace/images/generated.png]"));
    assert_eq!(
        host.request.unwrap(),
        json!({"model":"gpt-image-2","prompt":"a blue circle","background":"opaque","n":1})
    );
    assert_eq!(
        core::ENDPOINT,
        "https://chatgpt.com/backend-api/codex/images/generations"
    );
}

#[test]
fn uses_current_host_credentials_for_every_generation() {
    let mut host = Fixture::new();
    core::generate(&mut host, PROMPT).unwrap();
    core::generate(&mut host, PROMPT).unwrap();
    assert_eq!(host.credentials_read, 2);
    assert_eq!(host.sent_tokens, ["token-1", "token-2"]);
}

#[test]
fn transparent_background_is_an_api_option() {
    let mut host = Fixture::new();
    core::generate(
        &mut host,
        r#"{"prompt":"a logo","transparent_background":true}"#,
    )
    .unwrap();
    assert_eq!(host.request.unwrap()["background"], "transparent");
}

#[test]
fn arguments_cannot_supply_credentials_profiles_or_destinations() {
    for args in [
        r#"{"prompt":" "}"#,
        r#"{"prompt":"x","access_token":"stolen"}"#,
        r#"{"prompt":"x","auth_profile":"other"}"#,
        r#"{"prompt":"x","url":"https://example.org"}"#,
        r#"{"action":"status","job_id":"abc"}"#,
        "[]",
    ] {
        let mut host = Fixture::new();
        assert!(core::generate(&mut host, args).is_err());
        assert!(host.calls.is_empty());
    }
    let mut host = Fixture::new();
    core::generate(
        &mut host,
        r#"{"prompt":"x","__config":{"access_token":"ignored"}}"#,
    )
    .unwrap();
    assert_eq!(host.sent_tokens, ["token-1"]);
}

#[test]
fn rejects_empty_and_oversized_prompts_before_authentication() {
    let mut host = Fixture::new();
    let args = json!({"prompt":"a".repeat(32769)}).to_string();
    assert!(core::generate(&mut host, &args).is_err());
    assert!(host.calls.is_empty());
}

#[test]
fn missing_auth_stops_before_network_access() {
    let mut host = Fixture::new();
    host.auth_error = true;
    assert_eq!(
        core::generate(&mut host, PROMPT).unwrap_err(),
        "Login required"
    );
    assert_eq!(host.calls, ["auth"]);
}

#[test]
fn rejects_header_injection_from_bad_credentials() {
    let mut host = Fixture::new();
    host.invalid_credentials = true;
    assert!(core::generate(&mut host, PROMPT).is_err());
    assert_eq!(host.calls, ["auth"]);
}

#[test]
fn provider_failures_do_not_leak_response_bodies_or_retry() {
    for status in [401, 403, 429, 500] {
        let mut host = Fixture::new();
        host.status = status;
        host.response = b"secret provider diagnostics and token".to_vec();
        let error = core::generate(&mut host, PROMPT).unwrap_err();
        assert!(!error.contains("secret"));
        assert_eq!(host.calls, ["auth", "http"]);
    }
    let mut host = Fixture::new();
    host.transport_error = true;
    assert!(core::generate(&mut host, PROMPT).is_err());
    assert_eq!(host.calls, ["auth", "http"]);
}

#[test]
fn malformed_or_missing_image_data_never_reaches_artifact_storage() {
    for body in [
        b"not JSON".to_vec(),
        b"{}".to_vec(),
        json!({"data":[]}).to_string().into_bytes(),
        json!({"data":[{"b64_json":"!!!!"}]})
            .to_string()
            .into_bytes(),
        json!({"data":[{"b64_json":STANDARD.encode(b"not image bytes")}]})
            .to_string()
            .into_bytes(),
    ] {
        let mut host = Fixture::new();
        host.response = body;
        assert!(core::generate(&mut host, PROMPT).is_err());
        assert_eq!(host.calls, ["auth", "http"]);
    }
}

#[test]
fn bounds_image_response_and_requires_exactly_one_image() {
    let mut host = Fixture::new();
    host.response = vec![b' '; core::MAX_RESPONSE_BYTES + 1];
    assert!(core::generate(&mut host, PROMPT)
        .unwrap_err()
        .contains("size limit"));
    assert_eq!(host.calls, ["auth", "http"]);
    let mut host = Fixture::new();
    host.response = json!({"data":[{"b64_json":"x"},{"b64_json":"x"}]})
        .to_string()
        .into_bytes();
    assert!(core::generate(&mut host, PROMPT)
        .unwrap_err()
        .contains("exactly one"));
    assert_eq!(host.calls, ["auth", "http"]);
}

#[test]
fn artifact_failures_and_unsafe_markers_do_not_report_success() {
    let mut host = Fixture::new();
    host.artifact_error = true;
    assert!(core::generate(&mut host, PROMPT).is_err());
    let mut host = Fixture::new();
    host.invalid_artifact = true;
    assert!(core::generate(&mut host, PROMPT).is_err());
}

#[test]
fn reads_an_exactly_bounded_response_and_checks_for_eof() {
    let mut chunks = [Some(b"abc".to_vec()), Some(b"de".to_vec()), None].into_iter();
    let mut requests = Vec::new();
    let body = core::read_bounded(
        |limit| {
            requests.push(limit);
            Ok(chunks.next().unwrap())
        },
        5,
    )
    .unwrap();
    assert_eq!(body, b"abcde");
    assert_eq!(requests, [6, 3, 1]);
}

#[test]
fn stops_reading_as_soon_as_the_response_exceeds_the_bound() {
    let mut chunks = [Some(b"abc".to_vec()), Some(b"def".to_vec())].into_iter();
    let mut reads = 0;
    let error = core::read_bounded(
        |limit| {
            reads += 1;
            assert!(limit <= 6);
            Ok(chunks.next().expect("must stop before another read"))
        },
        5,
    )
    .unwrap_err();
    assert!(error.contains("size limit"));
    assert_eq!(reads, 2);
}

#[test]
fn caps_transport_reads_and_propagates_read_failure() {
    let mut reads = 0;
    let error = core::read_bounded(
        |limit| {
            assert_eq!(limit, 64 * 1024);
            reads += 1;
            if reads == 1 {
                Ok(Some(vec![0; limit as usize]))
            } else {
                Err("Cannot read Codex image response".into())
            }
        },
        core::MAX_RESPONSE_BYTES,
    )
    .unwrap_err();
    assert_eq!(error, "Cannot read Codex image response");
    assert_eq!(reads, 2);
}
