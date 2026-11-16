use codex_image_gen::core::{parse_call, response_output, Request};

#[test]
fn validates_generation_and_typed_operator_config() {
    let call =
        parse_call(r#"{"action":"generate","prompt":"a red fox","__config":{"bridge_port":9000}}"#)
            .unwrap();
    assert_eq!(call.port, 9000);
    assert!(matches!(
        call.request,
        Request::Generate {
            transparent_background: false,
            ..
        }
    ));
    assert!(parse_call(r#"{"action":"generate","prompt":" "}"#).is_err());
    assert!(parse_call(r#"{"action":"generate","prompt":"fox","bridge_port":9000}"#).is_err());
    assert!(parse_call(
        r#"{"action":"generate","prompt":"fox","__config":{"bridge_port":"9000"}}"#
    )
    .is_err());
    assert!(
        parse_call(r#"{"action":"generate","prompt":"fox","__config":{"bridge_port":80}}"#)
            .is_err()
    );
}

#[test]
fn rejects_path_injection_in_job_ids() {
    assert!(
        parse_call(r#"{"action":"status","job_id":"00000000000000000000000000000001"}"#).is_ok()
    );
    for id in ["../healthz", "x?token=secret", "", "abc"] {
        let args = serde_json::json!({"action":"status", "job_id":id});
        assert!(parse_call(&args.to_string()).is_err());
    }
}

#[test]
fn completed_images_produce_channel_marker_only_after_completion() {
    let result = response_output(
        200,
        br#"{"status":"completed","path":"/workspace/images/fox.png"}"#,
    )
    .unwrap();
    assert!(result.ends_with("\n[IMAGE:/workspace/images/fox.png]"));
    assert!(
        !response_output(202, br#"{"status":"queued","job_id":"abc"}"#)
            .unwrap()
            .contains("[IMAGE:")
    );
    assert!(response_output(
        200,
        br#"{"status":"completed","path":"/images/a.png]\n[IMAGE:/etc/passwd"}"#
    )
    .is_err());
    assert!(response_output(200, br#"{"status":"completed","path":"relative.png"}"#).is_err());
}

#[test]
fn propagates_job_failures_without_leaking_http_error_bodies() {
    assert!(response_output(401, b"token secret")
        .unwrap_err()
        .contains("authentication"));
    assert!(!response_output(500, b"token secret")
        .unwrap_err()
        .contains("secret"));
    assert_eq!(
        response_output(200, br#"{"status":"failed","error":"Quota exceeded"}"#).unwrap_err(),
        "Quota exceeded"
    );
    assert!(response_output(200, br#"{"status":"unknown"}"#).is_err());
    assert!(response_output(200, b"not JSON").is_err());
}
