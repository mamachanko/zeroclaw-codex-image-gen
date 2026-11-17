# ZeroClaw Codex Image Gen

Generate images in ZeroClaw using your Codex subscription, with a manifest
and a WASM component.

## Can I use it in ZeroClaw yet?

**Not on stock ZeroClaw yet.** This branch removes the Python bridge and makes
direct WASM generation the main implementation. ZeroClaw still needs to expose
Codex authentication and image storage to plugins before it can run this version.
Public releases are blocked until that integration is ready.

The intended experience is simple: install the plugin, grant access to your
Codex login, and ask your agent for an image. The package contains no companion
service. See the [host integration work](https://github.com/mamachanko/zeroclaw-codex-image-gen/blob/main/docs/direct-wasm.md) for the remaining
requirements and validation gates.

## Using the tool on an integrated host

Once the host services are implemented and granted, the agent calls
`codex_image_gen` once:

```json
{
  "prompt": "A tiny orange robot tending a rooftop garden, in a warm picture-book style",
  "transparent_background": false
}
```

The call waits for generation, saves the image through the host, and returns
an `[IMAGE:...]` marker for ZeroClaw to deliver. The host must allow enough time
for image generation, which can take several minutes. Ask for transparency by
setting `transparent_background` to `true`.

The host owns login, token refresh, account selection, and the image directory.
The plugin requests one image per call and makes no automatic retry, so a
failed request does not silently trigger another generation. Availability and
quotas depend on your Codex subscription.

## Moving from the bridge version

The `0.2.0-dev.0` implementation replaces the `0.1.0` bridge design. It is a
development version and cannot replace a working installation on stock ZeroClaw.

When moving to a compatible host:

- Stop the old bridge process and remove any service-manager entry for it.
- Remove this plugin's `bridge_token` and `bridge_port` configuration and its
  loopback egress grants. The new component connects directly to `chatgpt.com`.
- Replace the old generate/status polling instructions with a single
  `codex_image_gen` call using `prompt` and optional `transparent_background`.
- Configure Codex authentication through the host's supported flow. This
  component does not read Codex CLI files or keyrings itself.

## Build for development

Install [Rust](https://rustup.rs/) and Python 3.11+. The repository pins the Rust
toolchain and WASM target. Python is used for packaging and CI tooling only;
it is not a runtime dependency and no Python files enter the package.

```bash
git clone https://github.com/mamachanko/zeroclaw-codex-image-gen.git
cd zeroclaw-codex-image-gen
cargo build --locked --target wasm32-wasip2 --release
python3 tools/package.py
```

This creates `dist/codex-image-gen-0.2.0-dev.0.zip` and its SHA-256 checksum.
The archive contains the WASM component, manifest, README, and license notices.
CI also uploads these files as **codex-image-gen-package** under
[successful CI runs](https://github.com/mamachanko/zeroclaw-codex-image-gen/actions/workflows/ci.yml).
These are development artifacts requiring the proposed host APIs.

For checks, pull requests, and dependency maintenance, see
[CONTRIBUTING.md](https://github.com/mamachanko/zeroclaw-codex-image-gen/blob/main/CONTRIBUTING.md).
For the authentication contract, storage boundary, and release gates, see
[Direct WASM integration](https://github.com/mamachanko/zeroclaw-codex-image-gen/blob/main/docs/direct-wasm.md).
