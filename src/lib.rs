pub mod core;

#[cfg(target_family = "wasm")]
mod component {
    wit_bindgen::generate!({
        path: ["wit/v0", "wit/direct"],
        world: "zeroclaw:proposed-image-services/direct-image-plugin@0.1.0",
        features: ["plugins-wit-v0"],
        generate_all,
    });

    use crate::core::{self, Artifact, Credentials, Host, HttpResponse};
    use exports::zeroclaw::plugin::{plugin_info, tool};
    use zeroclaw::proposed_image_services::{artifacts, codex_auth};

    struct NativeHost;

    impl Host for NativeHost {
        fn credentials(&mut self) -> Result<Credentials, String> {
            let auth = codex_auth::get().map_err(|error| match error {
                codex_auth::AuthError::AccessDenied => "Codex auth is not granted to this plugin",
                codex_auth::AuthError::LoginRequired => {
                    "Sign in through the host's Codex authentication"
                }
                codex_auth::AuthError::Unavailable => {
                    "The host's Codex auth service is unavailable"
                }
            })?;
            Ok(Credentials {
                access_token: auth.access_token,
                account_id: auth.account_id,
            })
        }

        fn post(
            &mut self,
            credentials: &Credentials,
            body: Vec<u8>,
        ) -> Result<HttpResponse, String> {
            let response = waki::Client::new()
                .post(core::ENDPOINT)
                .header(
                    "Authorization",
                    format!("Bearer {}", credentials.access_token),
                )
                .header("chatgpt-account-id", &credentials.account_id)
                .header("Content-Type", "application/json")
                .header("Content-Length", body.len().to_string())
                .body(body)
                .send()
                .map_err(|_| "Codex image request failed; check the host's egress grant")?;
            let status = response.status_code();
            if status != 200 {
                return Ok(HttpResponse {
                    status,
                    body: Vec::new(),
                });
            }
            let body = core::read_bounded(
                |limit| {
                    response
                        .chunk(limit)
                        .map_err(|_| "Cannot read Codex image response".into())
                },
                core::MAX_RESPONSE_BYTES,
            )?;
            Ok(HttpResponse { status, body })
        }

        fn save_image(&mut self, bytes: &[u8]) -> Result<Artifact, String> {
            let image = artifacts::save_image(bytes)
                .map_err(|_| "The host could not save the generated image")?;
            Ok(Artifact {
                path: image.path,
                size_bytes: image.size_bytes,
            })
        }
    }

    struct DirectImagePlugin;
    impl plugin_info::Guest for DirectImagePlugin {
        fn plugin_name() -> String {
            env!("CARGO_PKG_NAME").into()
        }
        fn plugin_version() -> String {
            env!("CARGO_PKG_VERSION").into()
        }
    }
    impl tool::Guest for DirectImagePlugin {
        fn name() -> String {
            "codex_image_gen".into()
        }
        fn description() -> String {
            "Generate one image with the host's existing Codex login. Provide a prompt and optional transparent_background. Return the [IMAGE:...] result.".into()
        }
        fn parameters_schema() -> String {
            core::schema()
        }
        fn execute(args: String) -> Result<tool::ToolResult, String> {
            let result = core::generate(&mut NativeHost, &args);
            use zeroclaw::plugin::logging::{
                self, LogLevel, PluginAction, PluginEvent, PluginOutcome,
            };
            logging::log_record(
                LogLevel::Info,
                &PluginEvent {
                    function_name: "direct_codex_images::execute".into(),
                    action: if result.is_ok() {
                        PluginAction::Complete
                    } else {
                        PluginAction::Fail
                    },
                    outcome: Some(if result.is_ok() {
                        PluginOutcome::Success
                    } else {
                        PluginOutcome::Failure
                    }),
                    duration_ms: None,
                    attrs: None,
                    message: if result.is_ok() {
                        "Direct image generation completed"
                    } else {
                        "Direct image generation failed"
                    }
                    .into(),
                },
            );
            Ok(match result {
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
    export!(DirectImagePlugin);
}
