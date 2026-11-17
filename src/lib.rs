pub mod core;

#[cfg(target_family = "wasm")]
mod component {
    wit_bindgen::generate!({
        path: "wit/v0",
        world: "tool-plugin",
        features: ["plugins-wit-v0"],
    });
    use crate::core::{self, Request};
    use exports::zeroclaw::plugin::plugin_info::Guest as PluginInfo;
    use exports::zeroclaw::plugin::tool::{Guest as Tool, ToolResult};
    use zeroclaw::plugin::{logging, secrets};

    struct CodexImageGen;

    impl PluginInfo for CodexImageGen {
        fn plugin_name() -> String {
            "codex-image-gen".into()
        }
        fn plugin_version() -> String {
            env!("CARGO_PKG_VERSION").into()
        }
    }

    fn execute_request(args: &str) -> Result<String, String> {
        let call = core::parse_call(args)?;
        let token = secrets::get("bridge_token").map_err(|_| "Missing bridge_token secret")?;
        if token.len() < 32 || token.contains(['\r', '\n']) {
            return Err("Invalid bridge_token secret".into());
        }
        let client = waki::Client::new();
        let base = format!("http://127.0.0.1:{}", call.port);
        let builder = match call.request {
            Request::Generate { prompt, transparent_background } => {
                let body = serde_json::json!({"prompt": prompt, "transparent_background": transparent_background}).to_string();
                client
                    .post(&format!("{base}/v1/jobs"))
                    .header("Content-Type", "application/json")
                    .header("Content-Length", body.len().to_string())
                    .body(body)
            }
            Request::Status { job_id } => client.get(&format!("{base}/v1/jobs/{job_id}")),
        };
        let response = builder
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .map_err(|_| "Cannot reach Codex bridge; check service and plugin egress grants")?;
        let status = response.status_code();
        let body = response.body().map_err(|_| "Cannot read bridge response")?;
        if body.len() > 65536 {
            return Err("Bridge response too large".into());
        }
        core::response_output(status, &body)
    }

    impl Tool for CodexImageGen {
        fn name() -> String {
            "codex_image_gen".into()
        }
        fn description() -> String {
            "Generate images using the local Codex subscription. Call action=generate once with a prompt. \
             Then call action=status with the returned job_id until completed or failed. Status waits briefly; \
             generation can take several minutes. Never resubmit a pending job. Return the completed \
             [IMAGE:...] marker to the user. No FAL key or OpenAI API key is needed.".into()
        }
        fn parameters_schema() -> String {
            core::parameters_schema().to_string()
        }
        fn execute(args: String) -> Result<ToolResult, String> {
            let result = execute_request(&args);
            let success = result.is_ok();
            logging::log_record(
                logging::LogLevel::Info,
                &logging::PluginEvent {
                    function_name: "codex_image_gen::execute".into(),
                    action: if success {
                        logging::PluginAction::Complete
                    } else {
                        logging::PluginAction::Fail
                    },
                    outcome: Some(if success {
                        logging::PluginOutcome::Success
                    } else {
                        logging::PluginOutcome::Failure
                    }),
                    duration_ms: None,
                    attrs: None,
                    message: if success {
                        "Codex bridge request completed"
                    } else {
                        "Codex bridge request failed"
                    }
                    .into(),
                },
            );
            Ok(match result {
                Ok(output) => ToolResult {
                    success: true,
                    output,
                    error: None,
                },
                Err(error) => ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(error),
                },
            })
        }
    }
    export!(CodexImageGen);
}
