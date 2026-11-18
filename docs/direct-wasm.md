# Direct WASM integration

The plugin uses the stock ZeroClaw tool world. It owns a separate Codex
authorization and keeps credentials in the host's encrypted plugin state.
The Codex CLI can continue to own its own session.

The account module follows the Codex device authorization flow. Each poll
advances the flow once. Before a credential mutation, it reserves the account
record through compare-and-swap. If a process loses a response that could
have rotated a credential, start a new authorization using `reconnect`.

The generation module reserves a request ID before it sends the image POST.
It binds that ID to a digest of the image arguments. A repeat with the same
ID reads the stored outcome, and different arguments under the same ID are
rejected. A process timeout can leave the outcome unknown. The plugin retains
request records and refuses new work when the host state quota is full.

After decoding a bounded response, the plugin sends the image bytes to the
existing authenticated gateway route `POST /api/upload?agent=<alias>`.
Operator configuration fixes the loopback port and agent. ZeroClaw chooses
the file name and saves it in that agent's workspace. The plugin checks the
returned path and marker. The tool result carries the marker as text so the
agent can include it in its final response. If the upload response is lost,
check the workspace for the file before starting a new generation.

Related source:

- [Codex device authorization](https://github.com/openai/codex/blob/ce5a565ba0f2e459e953bb18f1cedd5440f46d6b/codex-rs/login/src/device_code_auth.rs)
- [Codex image request](https://github.com/openai/codex/blob/ce5a565ba0f2e459e953bb18f1cedd5440f46d6b/codex-rs/codex-api/src/endpoint/images.rs)
- [ZeroClaw 0.9.0 gateway upload](https://github.com/zeroclaw-labs/zeroclaw/blob/6c78ba59bce6a3d5079cf0c0310488770e264ad4/crates/zeroclaw-gateway/src/api_upload.rs)

A fixture test on stock ZeroClaw 0.9.0 exercised device authorization,
token exchange, one provider image request, and upload through the native
gateway. It verified the exact saved bytes and that a repeat with the same
ID returned the saved marker without a second image request. The packaged
plugin also received a live device sign-in code. Completing authorization
and live generation requires an eligible subscriber.

The HTTP adapter bounds connection setup. ZeroClaw's configured call
deadline controls the full operation. If that deadline interrupts a call
before a response or state write, the durable claim still prevents replay.
An image might be saved before its marker can be recorded. Check the
workspace if the outcome is uncertain.

Chat delivery needs verification for each configured channel. Development
versions remain gated from public release.
