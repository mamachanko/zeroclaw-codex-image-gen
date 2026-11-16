# ZeroClaw Codex Image Gen

**Generate images in ZeroClaw using your Codex subscription.** Connect your
existing Codex login, then ask your agent for an image in ordinary conversation.
No FAL account or OpenAI API key needed.

## Get started

You’ll need **ZeroClaw 0.8.6+ with WASM plugin support**, a **Codex CLI signed in
with ChatGPT and able to generate images**, and **Python 3.10+** on Linux or macOS.
Run everything as the same user on the same machine as ZeroClaw.

### 1. Download the plugin

Open [plugin downloads](https://github.com/mamachanko/zeroclaw-codex-image-gen/actions/workflows/ci.yml)
and choose the latest green run, then download **codex-image-gen-package** under
**Artifacts**. GitHub requires you to sign in to download it.
The package includes the plugin and its small
local connection service, called the bridge. You don’t need Rust to use it.

Assuming the download is in `~/Downloads`, unpack it into a permanent location:

```bash
mkdir -p "$HOME/.local/share/zeroclaw-codex-image-gen"
unzip "$HOME/Downloads/codex-image-gen-package.zip" \
  -d "$HOME/.local/share/zeroclaw-codex-image-gen"
cd "$HOME/.local/share/zeroclaw-codex-image-gen"
unzip codex-image-gen-0.1.0.zip
cd codex-image-gen
```

There are two ZIPs: GitHub wraps the plugin ZIP in an artifact ZIP. The commands
above unpack both. If you prefer to build it yourself, see [Build from source](#build-from-source).

### 2. Connect Codex

If you already use the Codex CLI with ChatGPT, the bridge uses that login.
Otherwise, run `codex login` first.

In the plugin directory, start the bridge:

```bash
python3 bridge/codex_image_bridge.py \
  --output-dir "$HOME/.zeroclaw/workspace/images"
```

Change the output directory if your ZeroClaw workspace is elsewhere. Generated
images need to be inside that workspace so ZeroClaw can send them back to you.

You should see `Codex image bridge: http://127.0.0.1:8787` followed by a token
file path. **Leave this terminal running** and open another for the next step.
The bridge handles the connection to Codex; you don’t need to copy Codex
credentials anywhere.

### 3. Enable it in ZeroClaw

In the second terminal, install the plugin:

```bash
zeroclaw plugin install "$HOME/.local/share/zeroclaw-codex-image-gen/codex-image-gen"
zeroclaw plugin info codex-image-gen
```

ZeroClaw prints a **config entry key** beginning with `zpi1_`. Copy that key
into the first line below, then run the block. A missing-configuration message
at this point is expected; these commands supply it.

```bash
key='PASTE_YOUR_zpi1_KEY_HERE'

zeroclaw config set "plugins.entries.$key.config.bridge_token" \
  "$(cat "$HOME/.local/state/zeroclaw-codex-image/bridge.token")"
zeroclaw config set "plugins.entries.$key.egress_hosts" '127.0.0.1'
zeroclaw config set "plugins.entries.$key.egress_allow_private" '127.0.0.1'
zeroclaw config set plugins.enabled true
zeroclaw config set plugins.auto_discover true
zeroclaw config set image_gen.enabled false
zeroclaw plugin list --verify
```

This connects the plugin to the local bridge and turns off ZeroClaw’s built-in
FAL image tool. The bridge token is created for you during step 2; it is separate
from your Codex login.

### 4. Tell your agent where to send image requests

Add this to your agent’s existing workspace instructions:

```text
For image generation, use codex_image_gen.
Submit action=generate once with the image prompt, then call action=status
with the returned job_id until completed or failed. Never resubmit a pending
job. Include the returned [IMAGE:...] marker in your response.
Do not use FAL or API-key image-generation fallbacks.
```

Give your agent enough time to wait for an image. In the **runtime profile your
agent already uses**, set `max_tool_iterations` to `40`. For example, if its
`runtime_profile` is `default`, update the existing table in your ZeroClaw config:

```toml
[runtime_profiles.default]
max_tool_iterations = 40
```

If you restrict tool access, allow `codex_image_gen`. Your channel’s message
timeout should also allow several minutes for generation. Restart your
ZeroClaw daemon after making these changes.

### 5. Ask for your first image

Send your ZeroClaw agent a message like:

> Create a picture of a tiny orange robot tending a rooftop garden,
> in a warm picture-book style.

Your agent should submit the request, wait for Codex, and return the image.
Generation can take a few minutes. The image is also saved in the output
directory you chose in step 2.

If anything gets stuck, [Troubleshooting](#troubleshooting) below will help you
find the next step.

## Make it yours

| Want to… | What to change |
|---|---|
| Save images somewhere else | Pass `--output-dir /path/inside/your/zeroclaw/workspace` when starting the bridge. |
| Use a transparent background | Ask your agent for a transparent background; the tool accepts `transparent_background: true`. |
| Keep the bridge running after closing the terminal | Run the bridge command under your usual service manager, as the same user who is signed in to Codex. |
| Use a Codex executable outside your `PATH` | Add `--codex /absolute/path/to/codex` to the bridge command. |
| Use a different Codex home | Set `CODEX_HOME` when starting the bridge. It inherits that home’s login and configuration. |
| Change the bridge port | Start it with `--port 9000` and set `plugins.entries.$key.config.bridge_port` to `9000`. |
| Allow more time per image | Set `--timeout` on the bridge, in seconds. The default is `600`; also allow enough agent iterations and message time. |

## Troubleshooting

| What you see | What to do |
|---|---|
| The tool is missing | Run `zeroclaw plugin list --verify`. Check that plugin discovery is enabled, the token is configured, and your ZeroClaw build supports WASM plugins. |
| “Cannot reach Codex bridge” | Check the first terminal is still running. The bridge port and both `127.0.0.1` egress settings must match your setup. |
| “Bridge authentication failed” | Repeat the `bridge_token` configuration command using the token file belonging to the running bridge. |
| Login expired or revoked | Run `codex login` as the bridge user, with the same `CODEX_HOME`. |
| Codex produced no image-generation result | Update Codex and check that its built-in `image_gen` is available for your account and selected model. |
| The agent stops waiting too soon | Check its selected runtime profile’s iteration limit and the channel’s message timeout. Keep the job ID; check its status instead of submitting again. |
| A file is saved, but no image appears in chat | Keep the output inside the ZeroClaw workspace and make sure the agent returns the `[IMAGE:...]` marker to a channel that supports attachments. |
| You use containers or separate machines | This version expects a shared filesystem and binds only to `127.0.0.1`. The quick start assumes ZeroClaw and the bridge run on the same machine. |

Codex controls image availability and subscription quotas. The bridge accepts
ChatGPT subscription login; API-key authentication and accounts reporting the
free plan are rejected. It never falls back to a separately billed image API.

**Verification status:** automated tests and the WASM build pass. A live Codex
handshake and account read succeeded, but generation was blocked by the test
environment’s expired login. Actual subscription image generation and loading
the plugin in a running ZeroClaw daemon remain unverified.

## How it works

```text
Your request in ZeroClaw
  → codex_image_gen plugin
  → local bridge
  → Codex CLI using your existing login
  → generated image in your ZeroClaw workspace
```

The bridge is needed because ZeroClaw’s WASM plugins cannot launch Codex
directly. Codex manages OAuth, credential storage, token refresh, and image
generation. The plugin never reads `auth.json` or copies refresh tokens.

The agent submits one background job and checks its status. Each check waits
up to 20 seconds, keeping individual calls within ZeroClaw’s default 30-second
plugin deadline. The bridge verifies a completed Codex image-generation event
and image bytes before reporting success. It preserves Codex’s original image
and saves a copy in your chosen directory.

<details>
<summary>Tool calls and job limits</summary>

The tool is named `codex_image_gen`. To start a generation:

```json
{"action":"generate","prompt":"A cozy cabin beneath an aurora","transparent_background":false}
```

To check it:

```json
{"action":"status","job_id":"COPY_THE_32_CHARACTER_JOB_ID"}
```

Jobs move through `queued`, `running`, `completed`, or `failed`. The bridge runs
one job at a time and accepts up to four unfinished jobs. Job metadata stays in
memory for one hour; restarting the bridge loses that status but preserves
completed image files. After a timeout or disconnect, check your artifacts
before submitting again: the remote generation may already have used quota.

Prompts are limited to 32 KiB and images to 32 MiB. This version generates one
image per job. Editing, reference images, batches, and API-level size, quality,
or model overrides are not available. You can describe the desired composition
and dimensions in your prompt, subject to Codex’s image tool capabilities.
Generated files and Codex’s original artifacts require manual cleanup.

</details>

<details>
<summary>Permissions, credentials, and compatibility</summary>

The plugin requests `http_client` and `config_read`, with no filesystem
permissions. ZeroClaw stores the local bridge token as an encrypted secret,
excluded from model-visible configuration. The bridge authenticates requests,
rejects browser origins and non-loopback Host headers, and does not log prompts,
credentials, protocol payloads, or raw Codex errors.

Each job uses a fresh ephemeral, read-only Codex thread. The bridge disables
the shell feature, refuses interactive approvals, removes API credential
environment variables, and selects the official provider and ChatGPT login.
It inherits Codex’s user configuration; use a dedicated Codex home if you want
to isolate other configured integrations.

The app-server protocol is version-sensitive. Compatibility was checked against
ZeroClaw registry source `eb0d38b`, host source `6e83982`, and Codex source
`662a634`, with a live handshake using Codex CLI `0.167.0-alpha.4`. The vendored
WIT contract is pinned in [wit/UPSTREAM_REF](wit/UPSTREAM_REF).

This plugin is distributed here as a complete local package. It is not yet
available through ZeroClaw’s plugin registry: that registry’s packager does not
include the Python bridge. Keep your existing signature policy; sign your local
build if your host requires signed components. ZeroClaw needs a WASM compiler
backend, for example `plugins-wasm,plugins-wasm-cranelift`.

Upstream references:
[ZeroClaw tool contract](https://github.com/zeroclaw-labs/zeroclaw-plugins/tree/main/wit/v0),
[Codex image tool](https://github.com/openai/codex/tree/main/codex-rs/ext/image-generation),
[Codex app-server protocol](https://github.com/openai/codex/tree/main/codex-rs/app-server-protocol).

</details>

## Build from source

To build the plugin yourself, you’ll need Rust with the
`wasm32-wasip2` target and Python 3.11+ for packaging.

```bash
git clone https://github.com/mamachanko/zeroclaw-codex-image-gen.git
cd zeroclaw-codex-image-gen
rustup target add wasm32-wasip2
cargo build --locked --target wasm32-wasip2 --release
python3 tools/package.py
unzip dist/codex-image-gen-0.1.0.zip -d dist
```

Your installable plugin is now in `dist/codex-image-gen/`. Continue with
[Connect Codex](#2-connect-codex), running the bridge from that directory, then
use its absolute path in the `zeroclaw plugin install` command.

<details>
<summary>Development checks</summary>

```bash
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --target wasm32-wasip2 -- -D warnings
cargo build --locked --target wasm32-wasip2 --release
python3 -m unittest discover -s bridge -p 'test_*.py' -v
python3 tools/package.py
```

Rust tests cover argument validation and tool output. Python tests cover
authentication, job execution and polling, image events, quota failures, file
confinement, and a fixture Codex subprocess exercising the stdio protocol,
approval refusal, environment filtering, disconnects, and timeouts.

[CI](https://github.com/mamachanko/zeroclaw-codex-image-gen/actions/workflows/ci.yml)
runs these checks without credentials or image usage and uploads an installable
ZIP with a SHA-256 checksum.

</details>

## Remove the plugin

Stop the bridge, then run:

```bash
zeroclaw plugin remove codex-image-gen
```

Remove the image-generation instruction from your agent. If you want to return
to ZeroClaw’s built-in FAL tool, run `zeroclaw config set image_gen.enabled true`.
Your generated images stay in place; delete them and the bridge token when you
no longer need them.

## License

Project code is [MIT licensed](LICENSE). The unmodified ZeroClaw WIT contract
is vendored under MIT OR Apache-2.0; see [NOTICE](NOTICE),
[wit/LICENSE-MIT](wit/LICENSE-MIT), and [wit/LICENSE-APACHE](wit/LICENSE-APACHE).
