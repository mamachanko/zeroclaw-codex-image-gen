# ZeroClaw Codex Image Gen

[![CI](https://github.com/mamachanko/zeroclaw-codex-image-gen/actions/workflows/ci.yml/badge.svg)](https://github.com/mamachanko/zeroclaw-codex-image-gen/actions/workflows/ci.yml)

A ZeroClaw WASM tool, `codex_image_gen`, that generates images through the
installed Codex CLI using its existing ChatGPT subscription login. Codex owns
OAuth, credential storage, token refresh, model selection, and image generation.
The plugin does not read `auth.json`, copy refresh tokens, or require FAL/OpenAI
API keys. It never falls back to a separately billed image API.

## How it works

```text
ZeroClaw → codex_image_gen.wasm → authenticated loopback bridge
         → codex app-server → built-in image_gen.imagegen → image file
```

The bridge is necessary: ZeroClaw's `tool-plugin` WASM contract has no process
execution or Codex-auth import. It uses Codex's JSONL app-server protocol over
private subprocess pipes. A job succeeds only after an actual `imageGeneration`
item completes and valid image bytes are available, not when the assistant says
it created an image. A copy is stored in your ZeroClaw workspace; Codex's
original artifact is preserved.

Generation runs in the background. `generate` returns a job ID immediately;
`status` waits up to 20 seconds and returns the current state. This stays within
ZeroClaw's default 30-second WASM call deadline. The agent repeats `status`, then
returns the `[IMAGE:/absolute/path]` marker for channels that support images.

## Requirements

- ZeroClaw v0.8.6 or later with WASM plugins and a compiler backend, e.g.
  `plugins-wasm,plugins-wasm-cranelift`.
- A recent Codex CLI exposing `app-server` and built-in `image_gen.imagegen`.
  Your signed-in subscription, selected model, and account rollout must allow
  image generation. A subscription alone does not guarantee availability.
- Python 3.10+ for the bridge on Linux or macOS; Python 3.11+ for the package
  script. No Python packages required.
- Codex and the bridge run as the same OS user on the same machine as ZeroClaw.
  They share its filesystem. Containers require additional network/filesystem
  arrangements; this version intentionally binds only to `127.0.0.1`.

This is a local, source-only plugin (`registry = false`): the registry's current
packager does not ship its required Python companion. Installing by registry
name is therefore unavailable. Use the complete package/directory install
below. Keep your existing plugin signature policy; sign your local build if your
host requires signed components.

## Install

1. Clone this repository and build the plugin:

   ```bash
   git clone https://github.com/mamachanko/zeroclaw-codex-image-gen.git
   cd zeroclaw-codex-image-gen
   rustup target add wasm32-wasip2
   cargo build --locked --target wasm32-wasip2 --release
   python3 tools/package.py
   unzip dist/codex-image-gen-0.1.0.zip -d dist
   ```

   This creates a complete installable package in `dist/codex-image-gen/`.
   Alternatively, download the package artifact from a successful
   [CI run](https://github.com/mamachanko/zeroclaw-codex-image-gen/actions), then
   extract it. Keep `bridge/codex_image_bridge.py` in a permanent location.

2. Sign in to Codex as the bridge user and start the bridge. Choose your actual
   ZeroClaw workspace for `--output-dir`:

   ```bash
   codex login
   python3 bridge/codex_image_bridge.py \
     --output-dir "$HOME/.zeroclaw/workspace/images"
   ```

   Leave it running, or run this command under your service manager. If Codex is
   not on the service's `PATH`, supply `--codex /absolute/path/to/codex`.
   `CODEX_HOME` is inherited, so it uses the same Codex login/keyring as that user.
   The bridge creates an owner-only random token file at
   `~/.local/state/zeroclaw-codex-image/bridge.token`. This local service token is
   separate from your Codex credentials. Override with `--token-file` if needed.

3. In another terminal, install this directory and obtain its instance key:

   ```bash
   zeroclaw plugin install /absolute/path/to/zeroclaw-codex-image-gen/dist/codex-image-gen
   zeroclaw plugin info codex-image-gen
   ```

   The info command prints a `zpi1_...` config entry key. It can report missing
   configuration until the following step is complete.

4. Copy that key into `key`, then configure the plugin:

   ```bash
   key='PASTE_THE_zpi1_INSTANCE_KEY_HERE'
   zeroclaw config set "plugins.entries.$key.config.bridge_token" \
     "$(cat "$HOME/.local/state/zeroclaw-codex-image/bridge.token")"
   zeroclaw config set "plugins.entries.$key.egress_hosts" '127.0.0.1'
   zeroclaw config set "plugins.entries.$key.egress_allow_private" '127.0.0.1'
   zeroclaw config set plugins.enabled true
   zeroclaw config set plugins.auto_discover true
   zeroclaw config set image_gen.enabled false
   zeroclaw plugin list --verify
   ```

   The private-address grant is explicitly limited to the loopback destination.
   ZeroClaw stores `bridge_token` as an encrypted, schema-designated secret;
   it is excluded from model-visible `__config`. The WASM plugin requests only
   `http_client` and `config_read`, with no filesystem permissions.

   For a different port, start the bridge with `--port 9000` and set
   `plugins.entries.$key.config.bridge_port` to `9000`. Port defaults to `8787`.

5. Give your agent this instruction in its existing workspace instructions:

   > For image generation, use `codex_image_gen`. Submit `action=generate`
   > once, then poll `action=status` with its job ID until completed or failed.
   > Never resubmit a pending job. Include the returned `[IMAGE:...]` marker
   > in the response. Do not use FAL or API-key image-generation fallbacks.

   Allow enough tool iterations for polling: 40 is suitable for one generation
   with the default 600-second bridge timeout. On current ZeroClaw, this is
   `max_tool_iterations = 40` in the agent's selected
   `[runtime_profiles.<profile>]`, referenced by
   `[agents.<alias>].runtime_profile`. Also allow `codex_image_gen` if your agent
   uses a restricted tool allowlist, and ensure its channel/message timeout
   permits a several-minute operation. Restart the ZeroClaw daemon after setup.

The tool uses its own name so it can coexist with the built-in `image_gen`.
Installing it does not rewrite ZeroClaw's built-in provider. Disabling the FAL
tool and adding the instruction above directs normal image requests to Codex.

## Calls

```json
{"action":"generate","prompt":"A cozy cabin beneath an aurora","transparent_background":false}
```

```json
{"action":"status","job_id":"COPY_THE_32_CHARACTER_JOB_ID"}
```

States are `queued`, `running`, `completed`, and `failed`. Do not submit a second
generation to check the first. The bridge runs one job at a time, accepts at most
four unfinished jobs, and retains job metadata for one hour in memory. A restart
loses job status; completed image files persist. On a timeout or disconnect, check
your artifacts before manually resubmitting: a remote generation may already have
consumed quota. Image files and Codex's original artifacts require manual cleanup.

Prompts are limited to 32 KiB; images to 32 MiB. Transparency is passed to Codex's
image tool. This initial version generates one image per job; editing, reference
images, batch generation, and API-level size/quality/model overrides are excluded.
Describe desired composition and dimensions in the prompt, subject to the image
tool's own capabilities.

## Troubleshooting and boundaries

- **Login expired/revoked:** run `codex login` as the bridge user with the same
  `CODEX_HOME`. The bridge never handles refresh tokens itself.
- **No image-generation result:** update Codex and confirm its built-in
  `image_gen` is available to your account/model. API-key auth and the free plan
  are rejected before a generation turn. Codex enforces subscription quotas.
- **Cannot reach bridge:** check that it is running, the port/token matches, and
  both loopback egress settings are present. `/healthz` requires the same bearer
  token; it checks the bridge process, not account eligibility.
- **Tool missing:** enable plugin discovery, supply the required secret, and
  run `zeroclaw plugin list --verify`. Match the host's WIT and signature policy.
- **Image file exists but no attachment:** ensure the output directory is in
  the ZeroClaw workspace and the agent returns the image marker to a channel
  supporting attachments.

The bridge authenticates all requests, rejects browser origins and non-loopback
Host headers, and does not log prompts, credentials, protocol payloads, or raw
Codex errors. It starts a fresh ephemeral, read-only Codex thread per job, disables
the shell feature, and refuses interactive approval requests. It inherits Codex's
normal user configuration; use a dedicated Codex profile/home if you need to
isolate unrelated configured integrations. API credential environment variables
are removed from the subprocess, and the official provider and ChatGPT login
method are selected explicitly. The app-server protocol remains version-sensitive.

## Development and verification

```bash
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --target wasm32-wasip2 -- -D warnings
cargo build --locked --target wasm32-wasip2 --release
python3 -m unittest discover -s bridge -p 'test_*.py' -v
python3 tools/package.py
```

The Rust tests exercise validation and tool output. Python tests cover HTTP
authentication, job execution/polling, canonical image events, quota failures,
file confinement, and a real subprocess with a fixture app-server covering JSONL
framing, buffered notifications, approval refusal, environment filtering,
disconnects, and timeouts. CI runs both suites without credentials or image usage,
and uploads the complete plugin package and its SHA-256 checksum as a workflow
artifact.

Compatibility was checked against registry commit `eb0d38b` (vendored `wit/v0`),
ZeroClaw host source `6e83982`, and Codex source `662a634`. A live protocol handshake
and `account/read` succeeded with Codex CLI `0.167.0-alpha.4`; a real generation
attempt was blocked by the environment's revoked/expired Codex login. Actual
subscription image generation and loading this component inside a running
ZeroClaw daemon have not been verified in this workspace.

Upstream references:
[ZeroClaw tool contract](https://github.com/zeroclaw-labs/zeroclaw-plugins/tree/main/wit/v0),
[Codex image tool](https://github.com/openai/codex/tree/main/codex-rs/ext/image-generation),
[Codex app-server protocol](https://github.com/openai/codex/tree/main/codex-rs/app-server-protocol).

To remove: stop the bridge, run `zeroclaw plugin remove codex-image-gen`, remove
the workspace instruction, and optionally re-enable `image_gen.enabled`. Delete
the bridge token and generated files only if you no longer need them.

## License

Project code is [MIT licensed](LICENSE). The unmodified ZeroClaw WIT contract
is vendored under MIT OR Apache-2.0; see [NOTICE](NOTICE),
[wit/LICENSE-MIT](wit/LICENSE-MIT), and [wit/LICENSE-APACHE](wit/LICENSE-APACHE).
