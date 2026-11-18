# ZeroClaw Codex Image Gen

Generate an image through a Codex subscription from a Rust WASM tool on
ZeroClaw 0.9.0. The plugin uses its own Codex device sign-in. It saves the
finished image through ZeroClaw's existing gateway.

This development package requires an active loopback gateway and a ChatGPT
account eligible for Codex image generation. See [validation status](docs/direct-wasm.md).

## Install the development package

Build with the pinned Rust toolchain and Python 3.11 or later.

```bash
cargo build --locked --target wasm32-wasip2 --release
python3 tools/package.py
cd dist
sha256sum -c codex-image-gen-0.2.0-dev.1.zip.sha256
unzip codex-image-gen-0.2.0-dev.1.zip
zeroclaw plugin install ./codex-image-gen
zeroclaw plugin info codex-image-gen
```

Use the config entry key printed by ZeroClaw. Set the local gateway port,
the alias of the agent that receives images, and its gateway pairing token.

```bash
entry='<config entry key>'
zeroclaw config set "plugins.entries.$entry.config.gateway_port" 42617
zeroclaw config set "plugins.entries.$entry.config.gateway_agent" assistant
zeroclaw config set "plugins.entries.$entry.config.gateway_token"
zeroclaw config set "plugins.entries.$entry.egress_allow_private" 127.0.0.1
zeroclaw config set plugins.enabled true
zeroclaw config set plugins.auto_discover true
zeroclaw config set plugins.limits.call_timeout_ms 240000
zeroclaw config set image_gen.enabled false
```

Enter the token at the masked prompt. Keep gateway pairing enabled and bind
the gateway to loopback. A pairing token grants access beyond image uploads.
Grant it only to a plugin you trust. Make sure the route that runs your agent
can also wait for a generation. If you invoke the agent through `/webhook`,
configure `gateway.request_timeout_secs` above the plugin call deadline.
Set `multimodal.max_image_size_mb` to at least 8 for the full plugin image limit.

For upgrades, replace the old loopback-only egress list with the declared
destinations `auth.openai.com`, `chatgpt.com`, and `127.0.0.1`. Remove any
old bridge configuration from the plugin instance. Apply setup to the gateway's
configuration, then start the gateway.

## Connect and generate

Use a private operator session for sign-in. Ask the agent to call
`codex_image_gen` with `{"action":"login_start"}`. Open the verification page
it returns and enter the one-time code. Then ask the agent to call
`{"action":"login_poll"}`. If authorization is pending, wait the returned
interval and check again. To replace a previous authorization, set
`reconnect` to `true` on `login_start`.

For a new image, use one fresh request ID per image.

```json
{
  "action": "generate",
  "request_id": "rooftop-garden-01",
  "prompt": "A tiny orange robot tending a rooftop garden",
  "transparent_background": false
}
```

Tell the agent to include the exact returned `[IMAGE:...]` marker in its
reply. The gateway saves the image under the configured agent's
`webui_uploads` directory. Attachment delivery depends on the channel.

After a timeout, call `{"action":"status","request_id":"rooftop-garden-01"}`.
Reuse that ID to check the original operation. A new ID may spend quota again.

For development checks and release policy, see [CONTRIBUTING.md](CONTRIBUTING.md).
