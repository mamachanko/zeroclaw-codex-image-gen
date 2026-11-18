use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::core::{Credentials, HttpResponse};

const STATE_KEY: &str = "account.v1";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const VERIFY_URL: &str = "https://auth.openai.com/codex/device";
const REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
const LOGIN_SECONDS: u64 = 15 * 60;
const REFRESH_EARLY_SECONDS: u64 = 60;
const INVALID_RESPONSE: &str =
    "The authentication service returned an invalid response. Start a new login with reconnect=true.";
const UNCONFIRMED: &str = "An authentication operation is in progress or its result is unknown. Start a new login with reconnect=true if the previous call has ended.";
const STATE_ERROR: &str =
    "Cannot update the account state. Another call may have changed it; check the login status.";

pub(crate) trait AccountHost {
    fn get(&mut self, key: &str) -> Result<Option<(Vec<u8>, u64)>, String>;
    fn put(&mut self, key: &str, value: &[u8], expected: Option<u64>) -> Result<u64, String>;
    fn now(&self) -> u64;
    fn auth_post(
        &mut self,
        path: &str,
        content_type: &str,
        body: Vec<u8>,
    ) -> Result<HttpResponse, String>;
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Account {
    Pending(DeviceGrant),
    Ready(Tokens),
    Claimed { operation: Mutation },
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Mutation {
    Start,
    Poll,
    Exchange,
    Refresh,
}

#[derive(Deserialize, Serialize)]
struct DeviceGrant {
    device_auth_id: String,
    user_code: String,
    interval: u64,
    next_poll_at: u64,
    expires_at: u64,
}

#[derive(Deserialize, Serialize)]
struct Tokens {
    access_token: String,
    refresh_token: String,
    account_id: String,
    expires_at: u64,
}

#[derive(Deserialize)]
struct DeviceResponse {
    device_auth_id: String,
    #[serde(alias = "usercode")]
    user_code: String,
    interval: Option<Value>,
}

#[derive(Deserialize)]
struct CodeResponse {
    authorization_code: String,
    code_verifier: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    id_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

pub(crate) fn start(host: &mut impl AccountHost, reconnect: bool) -> Result<String, String> {
    let stored = host
        .get(STATE_KEY)
        .map_err(|_| "Cannot read the encrypted account state.")?;
    if !reconnect {
        if let Some((bytes, _)) = &stored {
            let account: Account = serde_json::from_slice(bytes).map_err(|_| {
                "The saved account state is invalid. Start a new login with reconnect=true."
            })?;
            return match account {
                Account::Pending(grant) => instructions(&grant, host.now()),
                Account::Ready(_) => Ok("Connected to ChatGPT.".into()),
                Account::Claimed { .. } => Err(UNCONFIRMED.into()),
            };
        }
    }
    let revision = claim(host, stored.map(|(_, revision)| revision), Mutation::Start)?;
    let response = post_json(
        host,
        "/api/accounts/deviceauth/usercode",
        json!({"client_id": CLIENT_ID}),
    )?;
    if !(200..300).contains(&response.status) {
        return Err(match response.status {
            404 => "Device login is unavailable for this authentication service.",
            _ => "Cannot start device login. Start a new login with reconnect=true to try again.",
        }
        .into());
    }
    let response: DeviceResponse = decode(&response.body)?;
    let interval = match response.interval {
        None | Some(Value::Null) => 5,
        Some(Value::String(value)) => value.trim().parse::<u64>().map_err(|_| INVALID_RESPONSE)?,
        Some(Value::Number(value)) => value.as_u64().ok_or(INVALID_RESPONSE)?,
        Some(_) => return Err(INVALID_RESPONSE.into()),
    }
    .max(1);
    if !header_value(&response.device_auth_id, 4096)
        || response.user_code.is_empty()
        || response.user_code.len() > 64
        || !response
            .user_code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || interval > LOGIN_SECONDS
    {
        return Err(INVALID_RESPONSE.into());
    }
    let now = host.now();
    let grant = DeviceGrant {
        device_auth_id: response.device_auth_id,
        user_code: response.user_code,
        interval,
        next_poll_at: now.saturating_add(interval),
        expires_at: now.saturating_add(LOGIN_SECONDS),
    };
    let message = instructions(&grant, now)?;
    save(host, &Account::Pending(grant), Some(revision))?;
    Ok(message)
}

pub(crate) fn poll(host: &mut impl AccountHost) -> Result<String, String> {
    let (account, revision) = load(host)?.ok_or("Start a login with action=login_start first.")?;
    let grant = match account {
        Account::Ready(_) => return Ok("Connected to ChatGPT.".into()),
        Account::Claimed { .. } => return Err(UNCONFIRMED.into()),
        Account::Pending(grant) => grant,
    };
    let now = host.now();
    check_deadline(&grant, now)?;
    if now < grant.next_poll_at {
        return Ok(pending_message(grant.next_poll_at - now));
    }
    let revision = claim(host, Some(revision), Mutation::Poll)?;
    let response = post_json(
        host,
        "/api/accounts/deviceauth/token",
        json!({"device_auth_id": grant.device_auth_id, "user_code": grant.user_code}),
    )?;
    if matches!(response.status, 403 | 404) {
        let next_poll_at = host.now().saturating_add(grant.interval);
        let message = pending_message(grant.interval);
        save(
            host,
            &Account::Pending(DeviceGrant {
                next_poll_at,
                ..grant
            }),
            Some(revision),
        )?;
        return Ok(message);
    }
    if !(200..300).contains(&response.status) {
        return Err("Device login failed. Start a new login with reconnect=true.".into());
    }
    let code: CodeResponse = decode(&response.body)?;
    if !header_value(&code.authorization_code, 4096) || !header_value(&code.code_verifier, 4096) {
        return Err(INVALID_RESPONSE.into());
    }
    let body = form(&[
        ("grant_type", "authorization_code"),
        ("client_id", CLIENT_ID),
        ("code", &code.authorization_code),
        ("redirect_uri", REDIRECT_URI),
        ("code_verifier", &code.code_verifier),
    ]);
    let revision = claim(host, Some(revision), Mutation::Exchange)?;
    let response = host
        .auth_post("/oauth/token", "application/x-www-form-urlencoded", body)
        .map_err(|_| UNCONFIRMED)?;
    let tokens = tokens_from_response(response, host.now(), None)?;
    save(host, &Account::Ready(tokens), Some(revision))?;
    Ok("Connected to ChatGPT.".into())
}

pub(crate) fn access(host: &mut impl AccountHost) -> Result<Credentials, String> {
    let (account, revision) = load(host)?.ok_or("Sign in with action=login_start first.")?;
    let tokens = match account {
        Account::Ready(tokens) => tokens,
        Account::Pending(_) => {
            return Err(
                "Complete the browser authorization and call action=login_poll first.".into(),
            )
        }
        Account::Claimed { .. } => return Err(UNCONFIRMED.into()),
    };
    if tokens.expires_at > host.now().saturating_add(REFRESH_EARLY_SECONDS) {
        return Ok(credentials(&tokens));
    }
    let revision = claim(host, Some(revision), Mutation::Refresh)?;
    let response = post_json(
        host,
        "/oauth/token",
        json!({
            "grant_type": "refresh_token",
            "client_id": CLIENT_ID,
            "refresh_token": tokens.refresh_token,
            "refresh_reason": "proactive_expiry",
        }),
    )?;
    let tokens = tokens_from_response(response, host.now(), Some(tokens))?;
    let access = credentials(&tokens);
    save(host, &Account::Ready(tokens), Some(revision))?;
    Ok(access)
}

fn load(host: &mut impl AccountHost) -> Result<Option<(Account, u64)>, String> {
    host.get(STATE_KEY)
        .map_err(|_| "Cannot read the encrypted account state.")?
        .map(|(bytes, revision)| {
            serde_json::from_slice(&bytes)
                .map(|account| (account, revision))
                .map_err(|_| "The saved account state is invalid.".into())
        })
        .transpose()
}

fn save(
    host: &mut impl AccountHost,
    account: &Account,
    expected: Option<u64>,
) -> Result<u64, String> {
    let bytes = serde_json::to_vec(account).map_err(|_| "Cannot encode account state.")?;
    host.put(STATE_KEY, &bytes, expected)
        .map_err(|_| STATE_ERROR.into())
}

fn claim(
    host: &mut impl AccountHost,
    expected: Option<u64>,
    operation: Mutation,
) -> Result<u64, String> {
    // A claim has no replayable secrets. Only this call holds the mutation input.
    save(host, &Account::Claimed { operation }, expected)
}

fn post_json(host: &mut impl AccountHost, path: &str, body: Value) -> Result<HttpResponse, String> {
    host.auth_post(path, "application/json", body.to_string().into_bytes())
        .map_err(|_| UNCONFIRMED.into())
}

fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    serde_json::from_slice(bytes).map_err(|_| INVALID_RESPONSE.into())
}

fn instructions(grant: &DeviceGrant, now: u64) -> Result<String, String> {
    check_deadline(grant, now)?;
    Ok(format!(
        "Open {VERIFY_URL} and enter code {}. Then call action=login_poll in {} seconds. This login expires in {} seconds. Continue only if you started this login.",
        grant.user_code,
        grant.next_poll_at.saturating_sub(now),
        grant.expires_at - now,
    ))
}

fn check_deadline(grant: &DeviceGrant, now: u64) -> Result<(), String> {
    if now >= grant.expires_at {
        return Err("Device login expired. Start a new login with reconnect=true.".into());
    }
    Ok(())
}

fn pending_message(seconds: u64) -> String {
    format!("Authorization is pending. Call action=login_poll again in {seconds} seconds.")
}

fn credentials(tokens: &Tokens) -> Credentials {
    Credentials {
        access_token: tokens.access_token.clone(),
        account_id: tokens.account_id.clone(),
    }
}

fn tokens_from_response(
    response: HttpResponse,
    now: u64,
    previous: Option<Tokens>,
) -> Result<Tokens, String> {
    if !(200..300).contains(&response.status) {
        return Err("Authentication failed. Start a new login with reconnect=true.".into());
    }
    let response: TokenResponse = decode(&response.body)?;
    if !header_value(&response.access_token, 32768) {
        return Err(INVALID_RESPONSE.into());
    }
    let access_claims = jwt_claims(&response.access_token);
    let id_claims = response
        .id_token
        .as_deref()
        .map(|token| jwt_claims(token).ok_or_else(|| INVALID_RESPONSE.to_string()))
        .transpose()?;
    let access_account = account_id(access_claims.as_ref())?;
    let id_account = account_id(id_claims.as_ref())?;
    if matches!((&access_account, &id_account), (Some(access), Some(id)) if access != id) {
        return Err(INVALID_RESPONSE.into());
    }
    let account_id = id_account
        .or(access_account)
        .or_else(|| previous.as_ref().map(|tokens| tokens.account_id.clone()))
        .ok_or(INVALID_RESPONSE)?;
    if previous
        .as_ref()
        .is_some_and(|tokens| tokens.account_id != account_id)
    {
        return Err("The refreshed account changed. Start a new login with reconnect=true.".into());
    }
    let jwt_expiry = access_claims
        .as_ref()
        .and_then(|claims| claims.get("exp"))
        .map(|expiry| expiry.as_u64().ok_or(INVALID_RESPONSE))
        .transpose()?;
    let response_expiry = response
        .expires_in
        .map(|seconds| now.saturating_add(seconds));
    let expires_at = match (jwt_expiry, response_expiry) {
        (Some(jwt), Some(response)) => jwt.min(response),
        (Some(expiry), None) | (None, Some(expiry)) => expiry,
        (None, None) => return Err(INVALID_RESPONSE.into()),
    };
    let refresh_token = response
        .refresh_token
        .or_else(|| previous.map(|tokens| tokens.refresh_token))
        .ok_or(INVALID_RESPONSE)?;
    if expires_at <= now || !header_value(&refresh_token, 32768) {
        return Err(INVALID_RESPONSE.into());
    }
    Ok(Tokens {
        access_token: response.access_token,
        refresh_token,
        account_id,
        expires_at,
    })
}

fn jwt_claims(token: &str) -> Option<Value> {
    let mut segments = token.split('.');
    let header = segments.next()?;
    let payload = segments.next()?;
    let signature = segments.next()?;
    if header.is_empty() || payload.is_empty() || signature.is_empty() || segments.next().is_some()
    {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value.is_object().then_some(value)
}

fn account_id(claims: Option<&Value>) -> Result<Option<String>, String> {
    let Some(value) = claims
        .and_then(|claims| claims.get("https://api.openai.com/auth"))
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .filter(|value| !value.is_null())
    else {
        return Ok(None);
    };
    let id = value.as_str().ok_or(INVALID_RESPONSE)?;
    if !header_value(id, 256) {
        return Err(INVALID_RESPONSE.into());
    }
    Ok(Some(id.to_owned()))
}

fn header_value(value: &str, limit: usize) -> bool {
    !value.is_empty()
        && value.len() <= limit
        && value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

fn form(parameters: &[(&str, &str)]) -> Vec<u8> {
    let mut encoded = String::new();
    for (index, (key, value)) in parameters.iter().enumerate() {
        if index != 0 {
            encoded.push('&');
        }
        encoded.push_str(key);
        encoded.push('=');
        for byte in value.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                    encoded.push(byte as char)
                }
                b' ' => encoded.push('+'),
                _ => {
                    const HEX: &[u8; 16] = b"0123456789ABCDEF";
                    encoded.push('%');
                    encoded.push(HEX[(byte >> 4) as usize] as char);
                    encoded.push(HEX[(byte & 15) as usize] as char);
                }
            }
        }
    }
    encoded.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct Request {
        path: String,
        content_type: String,
        body: Vec<u8>,
    }

    struct Fixture {
        state: Option<(Vec<u8>, u64)>,
        now: u64,
        responses: VecDeque<Result<HttpResponse, String>>,
        requests: Vec<Request>,
        writes: usize,
        fail_write: Option<usize>,
        reconnect_during_request: Option<usize>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                state: None,
                now: 1000,
                responses: VecDeque::new(),
                requests: Vec::new(),
                writes: 0,
                fail_write: None,
                reconnect_during_request: None,
            }
        }

        fn respond(&mut self, status: u16, body: Value) {
            self.responses.push_back(Ok(HttpResponse {
                status,
                body: body.to_string().into_bytes(),
            }));
        }

        fn device(&mut self) {
            self.respond(
                200,
                json!({"device_auth_id":"private-device", "user_code":"ABCD-EFGH", "interval":"5"}),
            );
        }

        fn code(&mut self) {
            self.respond(
                200,
                json!({
                    "authorization_code":"private+code&with=punctuation/",
                    "code_verifier":"verifier~with+punctuation",
                    "code_challenge":"unused-challenge"
                }),
            );
        }

        fn connected() -> Self {
            let mut host = Self::new();
            host.device();
            start(&mut host, false).unwrap();
            host.now += 5;
            host.code();
            host.respond(200, token_reply(2000, "original-refresh"));
            poll(&mut host).unwrap();
            host
        }
    }

    impl AccountHost for Fixture {
        fn get(&mut self, key: &str) -> Result<Option<(Vec<u8>, u64)>, String> {
            assert_eq!(key, STATE_KEY);
            Ok(self.state.clone())
        }

        fn put(&mut self, key: &str, value: &[u8], expected: Option<u64>) -> Result<u64, String> {
            assert_eq!(key, STATE_KEY);
            self.writes += 1;
            if self.fail_write == Some(self.writes)
                || self.state.as_ref().map(|(_, revision)| *revision) != expected
            {
                return Err("storage failure with secret detail".into());
            }
            let revision = expected.unwrap_or(0) + 1;
            self.state = Some((value.to_vec(), revision));
            Ok(revision)
        }

        fn now(&self) -> u64 {
            self.now
        }

        fn auth_post(
            &mut self,
            path: &str,
            content_type: &str,
            body: Vec<u8>,
        ) -> Result<HttpResponse, String> {
            let stored: Account = serde_json::from_slice(&self.state.as_ref().unwrap().0).unwrap();
            assert!(matches!(stored, Account::Claimed { .. }));
            self.requests.push(Request {
                path: path.into(),
                content_type: content_type.into(),
                body,
            });
            let response = self.responses.pop_front().expect("unexpected network call");
            if self.reconnect_during_request == Some(self.requests.len()) {
                self.reconnect_during_request = None;
                start(self, true).unwrap();
            }
            response
        }
    }

    fn jwt(claims: Value) -> String {
        format!(
            "e30.{}.fixture-signature",
            URL_SAFE_NO_PAD.encode(claims.to_string())
        )
    }

    fn token_reply(expiry: u64, refresh_token: &str) -> Value {
        json!({
            "access_token":jwt(json!({
                "exp":expiry,
                "https://api.openai.com/auth":{"chatgpt_account_id":"fixture-account"}
            })),
            "id_token":jwt(json!({
                "https://api.openai.com/auth":{"chatgpt_account_id":"fixture-account"}
            })),
            "refresh_token":refresh_token
        })
    }

    #[test]
    fn device_authorization_matches_codex_wire_and_returns_only_public_login_details() {
        let mut host = Fixture::new();
        host.device();
        let instructions = start(&mut host, false).unwrap();
        assert!(instructions.contains("https://auth.openai.com/codex/device"));
        assert!(instructions.contains("ABCD-EFGH"));
        assert!(!instructions.contains("private-device"));
        assert_eq!(host.requests[0].path, "/api/accounts/deviceauth/usercode");
        assert_eq!(host.requests[0].content_type, "application/json");
        assert_eq!(
            serde_json::from_slice::<Value>(&host.requests[0].body).unwrap(),
            json!({"client_id":"app_EMoamEEZ73f0CkXaXp7hrann"})
        );
        assert_eq!(start(&mut host, false).unwrap(), instructions);
        assert!(poll(&mut host).unwrap().contains("5 seconds"));
        assert_eq!(host.requests.len(), 1);

        host.now += 5;
        host.code();
        host.respond(200, token_reply(2000, "private-refresh"));
        let connected = poll(&mut host).unwrap();
        assert_eq!(connected, "Connected to ChatGPT.");
        assert_eq!(host.requests[1].path, "/api/accounts/deviceauth/token");
        assert_eq!(
            serde_json::from_slice::<Value>(&host.requests[1].body).unwrap(),
            json!({"device_auth_id":"private-device", "user_code":"ABCD-EFGH"})
        );
        let exchange = &host.requests[2];
        assert_eq!(exchange.path, "/oauth/token");
        assert_eq!(exchange.content_type, "application/x-www-form-urlencoded");
        assert_eq!(
            String::from_utf8(exchange.body.clone()).unwrap(),
            "grant_type=authorization_code&client_id=app_EMoamEEZ73f0CkXaXp7hrann&code=private%2Bcode%26with%3Dpunctuation%2F&redirect_uri=https%3A%2F%2Fauth.openai.com%2Fdeviceauth%2Fcallback&code_verifier=verifier%7Ewith%2Bpunctuation"
        );
        let credentials = access(&mut host).unwrap();
        assert_eq!(credentials.account_id, "fixture-account");
        assert!(credentials.access_token.contains('.'));
        assert_eq!(start(&mut host, false).unwrap(), connected);
        assert_eq!(poll(&mut host).unwrap(), connected);
        assert_eq!(host.requests.len(), 3);
    }

    #[test]
    fn pending_polls_are_bounded_and_preserve_the_original_login() {
        let mut host = Fixture::new();
        host.device();
        start(&mut host, false).unwrap();
        for status in [403, 404] {
            host.now += 5;
            host.respond(status, json!({"error":"arbitrary private service detail"}));
            assert!(poll(&mut host).unwrap().contains("pending"));
            let count = host.requests.len();
            assert!(poll(&mut host).unwrap().contains("5 seconds"));
            assert!(start(&mut host, false).unwrap().contains("ABCD-EFGH"));
            assert_eq!(host.requests.len(), count);
        }
        host.now = 1900;
        assert!(poll(&mut host).unwrap_err().contains("expired"));
        assert!(start(&mut host, false).unwrap_err().contains("expired"));
        assert_eq!(host.requests.len(), 3);
        host.device();
        assert!(start(&mut host, true).unwrap().contains("ABCD-EFGH"));
        assert_eq!(host.requests.len(), 4);
    }

    #[test]
    fn refresh_rotates_tokens_once_and_retains_optional_refresh_token() {
        let mut host = Fixture::connected();
        host.now = 1940;
        host.respond(200, token_reply(3000, "rotated-refresh"));
        let refreshed = access(&mut host).unwrap();
        assert_eq!(refreshed.account_id, "fixture-account");
        assert_eq!(host.requests[3].content_type, "application/json");
        assert_eq!(
            serde_json::from_slice::<Value>(&host.requests[3].body).unwrap(),
            json!({
                "grant_type":"refresh_token", "client_id":"app_EMoamEEZ73f0CkXaXp7hrann",
                "refresh_token":"original-refresh", "refresh_reason":"proactive_expiry"
            })
        );
        assert_eq!(
            access(&mut host).unwrap().access_token,
            refreshed.access_token
        );
        assert_eq!(host.requests.len(), 4);

        host.now = 2940;
        host.respond(
            200,
            json!({"access_token":"opaque-access", "expires_in":600}),
        );
        assert_eq!(access(&mut host).unwrap().access_token, "opaque-access");
        host.now = 3480;
        host.respond(200, token_reply(4500, "next-refresh"));
        access(&mut host).unwrap();
        for request in &host.requests[4..] {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(body["refresh_token"], "rotated-refresh");
        }
    }

    #[test]
    fn failed_claim_does_not_send_a_request_or_leak_storage_errors() {
        let mut host = Fixture::new();
        host.fail_write = Some(1);
        let error = start(&mut host, false).unwrap_err();
        assert!(!error.contains("secret detail"));
        assert!(host.requests.is_empty());

        let mut host = Fixture::connected();
        host.now = 1940;
        host.fail_write = Some(host.writes + 1);
        assert!(access(&mut host).is_err());
        assert_eq!(host.requests.len(), 3);
    }

    #[test]
    fn ambiguous_refresh_cannot_replay_even_after_time_passes() {
        let mut host = Fixture::connected();
        host.now = 1940;
        host.responses
            .push_back(Err("transport error echoes original-refresh".into()));
        let error = access(&mut host).err().unwrap();
        assert!(!error.contains("original-refresh"));
        assert!(error.contains("unknown"));
        host.now = 1_000_000;
        assert!(access(&mut host).is_err());
        assert!(start(&mut host, false).is_err());
        assert!(poll(&mut host).is_err());
        assert_eq!(host.requests.len(), 4);
        host.device();
        assert!(start(&mut host, true).is_ok());
    }

    #[test]
    fn lost_poll_or_exchange_reply_requires_explicit_reconnect() {
        for lose_exchange in [false, true] {
            let mut host = Fixture::new();
            host.device();
            start(&mut host, false).unwrap();
            host.now += 5;
            if lose_exchange {
                host.code();
            }
            host.responses
                .push_back(Err("transport error with private-device".into()));
            let error = poll(&mut host).unwrap_err();
            assert!(!error.contains("private-device"));
            assert!(poll(&mut host).is_err());
            assert!(start(&mut host, false).is_err());
            assert_eq!(host.requests.len(), if lose_exchange { 3 } else { 2 });
            host.device();
            assert!(start(&mut host, true).is_ok());
        }
    }

    #[test]
    fn explicit_reconnect_can_replace_unreadable_saved_state() {
        let mut host = Fixture::new();
        host.state = Some((b"outdated state format".to_vec(), 42));
        assert!(start(&mut host, false).is_err());
        assert!(host.requests.is_empty());
        host.device();
        assert!(start(&mut host, true).unwrap().contains("ABCD-EFGH"));
        assert_eq!(host.state.as_ref().unwrap().1, 44);
    }

    #[test]
    fn failed_final_commit_never_replays_the_consumed_code() {
        let mut host = Fixture::new();
        host.device();
        start(&mut host, false).unwrap();
        host.now += 5;
        host.code();
        host.respond(200, token_reply(2000, "private-refresh"));
        host.fail_write = Some(5);
        assert!(poll(&mut host).is_err());
        assert!(poll(&mut host).is_err());
        assert!(access(&mut host).is_err());
        assert_eq!(host.requests.len(), 3);
    }

    #[test]
    fn reconnect_fences_the_old_poll_before_code_exchange() {
        let mut host = Fixture::new();
        host.device();
        start(&mut host, false).unwrap();
        host.now += 5;
        host.code();
        host.device();
        host.reconnect_during_request = Some(2);
        assert!(poll(&mut host).is_err());
        assert!(start(&mut host, false).unwrap().contains("ABCD-EFGH"));
        assert_eq!(host.requests.len(), 3);
        assert!(host
            .requests
            .iter()
            .all(|request| request.path != "/oauth/token"));
    }

    #[test]
    fn reconnect_fences_old_refresh_credentials_and_preserves_new_login() {
        let mut host = Fixture::connected();
        host.now = 1940;
        host.respond(200, token_reply(3000, "obsolete-refresh"));
        host.device();
        host.reconnect_during_request = Some(4);
        assert!(access(&mut host).is_err());
        assert!(start(&mut host, false).unwrap().contains("ABCD-EFGH"));
        assert!(access(&mut host).is_err());
        assert_eq!(host.requests.len(), 5);
    }

    #[test]
    fn token_response_requires_safe_identity_and_known_unexpired_access() {
        let valid = token_reply(2000, "refresh");
        let mut cases = Vec::new();
        let mut no_expiry = valid.clone();
        no_expiry["access_token"] = json!("opaque-access");
        cases.push(no_expiry);
        let mut expired = valid.clone();
        expired["access_token"] = json!(jwt(json!({"exp":1000})));
        expired["expires_in"] = json!(3600);
        cases.push(expired);
        let mut mismatch = valid.clone();
        mismatch["id_token"] = json!(jwt(json!({
            "https://api.openai.com/auth":{"chatgpt_account_id":"different-account"}
        })));
        cases.push(mismatch);
        let mut unsafe_header = valid;
        unsafe_header["access_token"] = json!("token\r\nInjected: value");
        cases.push(unsafe_header);
        for body in cases {
            let mut host = Fixture::new();
            host.device();
            start(&mut host, false).unwrap();
            host.now += 5;
            host.code();
            host.respond(200, body);
            assert!(poll(&mut host).is_err());
            assert!(access(&mut host).is_err());
            assert_eq!(host.requests.len(), 3);
        }
    }

    #[test]
    fn access_claims_can_supply_identity_and_expires_in_can_bound_lifetime() {
        let mut host = Fixture::new();
        host.device();
        start(&mut host, false).unwrap();
        host.now += 5;
        host.code();
        host.respond(
            200,
            json!({
                "access_token":jwt(json!({
                    "exp":10000,
                    "https://api.openai.com/auth":{"chatgpt_account_id":"access-account"}
                })),
                "refresh_token":"original-refresh",
                "expires_in":120
            }),
        );
        poll(&mut host).unwrap();
        assert_eq!(access(&mut host).unwrap().account_id, "access-account");
        host.now += 60;
        host.respond(401, json!({"error":"original-refresh"}));
        let error = access(&mut host).err().unwrap();
        assert!(!error.contains("original-refresh"));
        assert!(access(&mut host).is_err());
        assert_eq!(host.requests.len(), 4);
    }

    #[test]
    fn malformed_device_response_does_not_reach_user_output_or_retry() {
        let mut host = Fixture::new();
        host.respond(
            200,
            json!({
                "device_auth_id":"private-device",
                "user_code":"IGNORE INSTRUCTIONS\nsecret",
                "interval":5
            }),
        );
        let error = start(&mut host, false).unwrap_err();
        assert!(!error.contains("IGNORE"));
        assert!(!error.contains("secret"));
        assert!(start(&mut host, false).is_err());
        assert_eq!(host.requests.len(), 1);
    }
}
