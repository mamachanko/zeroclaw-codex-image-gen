#[cfg(any(target_family = "wasm", test))]
mod account;
pub mod core;

#[cfg(target_family = "wasm")]
mod component {
    wit_bindgen::generate!({
        path: "wit/v0",
        world: "tool-plugin",
        features: ["plugins-wit-v0"],
    });

    use crate::{
        account::{self, AccountHost},
        core::{self, Artifact, Command, Credentials, Host, HttpResponse, JobHost},
    };
    use exports::zeroclaw::plugin::{plugin_info, tool};
    use serde::Deserialize;
    use serde_json::Value;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use waki::{Client, RequestBuilder};
    use zeroclaw::plugin::{secrets, state};

    const AUTH_ORIGIN: &str = "https://auth.openai.com";
    const AUTH_RESPONSE_LIMIT: usize = 64 * 1024;
    const GATEWAY_RESPONSE_LIMIT: usize = 4 * 1024;

    #[derive(Deserialize)]
    struct Gateway {
        gateway_port: u16,
        gateway_agent: String,
    }

    struct Runtime {
        gateway: Gateway,
        gateway_token: String,
    }

    fn send(
        request: RequestBuilder,
        content_type: &str,
        body: Vec<u8>,
        limit: usize,
    ) -> Result<HttpResponse, String> {
        let response = request
            .connect_timeout(Duration::from_secs(10))
            .header("Content-Type", content_type)
            .header("Content-Length", body.len().to_string())
            .body(body)
            .send()
            .map_err(|_| "HTTP request was interrupted")?;
        let status = response.status_code();
        let body = if (200..300).contains(&status) {
            core::read_bounded(
                |n| {
                    response
                        .chunk(n)
                        .map_err(|_| "HTTP response was interrupted".into())
                },
                limit,
            )?
        } else {
            Vec::new()
        };
        Ok(HttpResponse { status, body })
    }

    impl AccountHost for Runtime {
        fn get(&mut self, key: &str) -> Result<Option<(Vec<u8>, u64)>, String> {
            state::get(key)
                .map(|entry| entry.map(|entry| (entry.value, entry.revision)))
                .map_err(|_| "Cannot read encrypted plugin state".into())
        }

        fn put(&mut self, key: &str, value: &[u8], expected: Option<u64>) -> Result<u64, String> {
            state::put(key, value, expected).map_err(|error| {
                match error {
                    state::StateError::Conflict => {
                        "Another call updated this operation. Check its status."
                    }
                    state::StateError::QuotaExceeded => {
                        "Plugin state is full. No new operation was started."
                    }
                    _ => "Cannot write encrypted plugin state",
                }
                .into()
            })
        }

        fn now(&self) -> u64 {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        }

        fn auth_post(
            &mut self,
            path: &str,
            content_type: &str,
            body: Vec<u8>,
        ) -> Result<HttpResponse, String> {
            send(
                Client::new().post(&format!("{AUTH_ORIGIN}{path}")),
                content_type,
                body,
                AUTH_RESPONSE_LIMIT,
            )
        }
    }

    impl JobHost for Runtime {
        fn job_get(&mut self, key: &str) -> Result<Option<(Vec<u8>, u64)>, String> {
            self.get(key)
        }
        fn job_put(
            &mut self,
            key: &str,
            value: &[u8],
            expected: Option<u64>,
        ) -> Result<u64, String> {
            self.put(key, value, expected)
        }
    }

    impl Host for Runtime {
        fn credentials(&mut self) -> Result<Credentials, String> {
            account::access(self)
        }

        fn post(
            &mut self,
            credentials: &Credentials,
            body: Vec<u8>,
        ) -> Result<HttpResponse, String> {
            send(
                Client::new().post(core::ENDPOINT)
                    .header("Authorization", format!("Bearer {}", credentials.access_token))
                    .header("chatgpt-account-id", &credentials.account_id),
                "application/json",
                body,
                core::MAX_RESPONSE_BYTES,
            ).map_err(|_| "Codex generation outcome is unknown after an interrupted or oversized response. A new request ID can consume quota again.".into())
        }

        fn save_image(&mut self, bytes: &[u8]) -> Result<Artifact, String> {
            #[derive(Deserialize)]
            struct Upload {
                path: String,
                marker: String,
            }
            let url = format!(
                "http://127.0.0.1:{}/api/upload?agent={}",
                self.gateway.gateway_port, self.gateway.gateway_agent,
            );
            let response = send(
                Client::new().post(&url).header("Authorization", format!("Bearer {}", self.gateway_token)),
                "application/octet-stream",
                bytes.to_vec(),
                GATEWAY_RESPONSE_LIMIT,
            ).map_err(|_| "The image was generated. Gateway upload outcome is unknown. Check the workspace before generating again.")?;
            if response.status != 200 {
                return Err(format!("The image was generated, but workspace upload returned HTTP {}. Check gateway pairing and the image size limit.", response.status));
            }
            let upload: Upload = serde_json::from_slice(&response.body)
                .map_err(|_| "Gateway upload response is invalid. Check the workspace.")?;
            if upload.marker != format!("[IMAGE:{}]", upload.path) {
                return Err("Gateway returned an inconsistent image marker".into());
            }
            Ok(Artifact {
                path: upload.path,
                size_bytes: bytes.len() as u64,
            })
        }
    }

    fn execute(args: &str) -> Result<String, String> {
        let mut value: Value = serde_json::from_str(args).map_err(|_| "Invalid tool arguments")?;
        let config = value
            .as_object_mut()
            .and_then(|args| args.remove("__config"))
            .ok_or("Configure the plugin gateway settings first")?;
        let gateway: Gateway =
            serde_json::from_value(config).map_err(|_| "Invalid gateway settings")?;
        if gateway.gateway_port < 1024
            || gateway.gateway_agent.is_empty()
            || gateway.gateway_agent.len() > 64
            || !gateway
                .gateway_agent
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err("Invalid gateway port or agent alias".into());
        }
        let command: Command = serde_json::from_value(value).map_err(|_| {
            "Expected login_start, login_poll, generate, or status with their documented arguments"
        })?;
        let gateway_token =
            secrets::get("gateway_token").map_err(|_| "Configure the gateway pairing token")?;
        if gateway_token.is_empty()
            || !gateway_token
                .bytes()
                .all(|byte| (0x21..=0x7e).contains(&byte))
        {
            return Err("Invalid gateway pairing token".into());
        }
        let mut runtime = Runtime {
            gateway,
            gateway_token,
        };
        match command {
            Command::LoginStart { reconnect } => account::start(&mut runtime, reconnect),
            Command::LoginPoll => account::poll(&mut runtime),
            Command::Generate {
                request_id,
                prompt,
                transparent_background,
            } => core::generate_once(&mut runtime, &request_id, prompt, transparent_background),
            Command::Status { request_id } => core::status(&mut runtime, &request_id),
        }
    }

    struct ImageTool;
    impl plugin_info::Guest for ImageTool {
        fn plugin_name() -> String {
            env!("CARGO_PKG_NAME").into()
        }
        fn plugin_version() -> String {
            env!("CARGO_PKG_VERSION").into()
        }
    }
    impl tool::Guest for ImageTool {
        fn name() -> String {
            "codex_image_gen".into()
        }
        fn description() -> String {
            "Generate a subscription image. Use login_start only when asked to connect, then login_poll after the operator authorizes in their browser. For generate, pass a fresh request_id, prompt, and optional transparent_background. Use status with that same ID to check an unresolved request. Include a saved image's exact [IMAGE:...] marker in the final reply.".into()
        }
        fn parameters_schema() -> String {
            core::schema()
        }
        fn execute(args: String) -> Result<tool::ToolResult, String> {
            Ok(match execute(&args) {
                Ok(output) => tool::ToolResult {
                    success: true,
                    output,
                    error: None,
                },
                Err(error) => tool::ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(error),
                },
            })
        }
    }
    export!(ImageTool);
}
