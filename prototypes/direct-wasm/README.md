# Direct WASM prototype

**This is a host-contract prototype, not an installable stock ZeroClaw plugin.**
Its proposed imports do not exist in the released ZeroClaw plugin API.
The [proposed manifest](manifest.proposed.toml) names the corresponding grants;
it deliberately is not named `manifest.toml` to prevent accidental installation.

It demonstrates the intended single-call flow: obtain current host-managed
Codex credentials, call the image endpoint directly over WASI HTTP, and ask the
host to save the returned image. The agent supplies only a prompt and an
optional transparency setting. There is no Python service, service token,
Codex subprocess, job submission, or status polling.

The design and the host changes needed to make it usable are described in
[the proposal](../../docs/direct-wasm.md). Nothing in this prototype reads an
auth file, refreshes tokens independently, or accepts a manually copied token.

```bash
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --target wasm32-wasip2 -- -D warnings
cargo build --locked --target wasm32-wasip2 --release
```

The output is `target/wasm32-wasip2/release/codex_image_gen_prototype.wasm`.
Building it proves the proposed WIT is consistent with the guest, not that
ZeroClaw implements the new services. CI builds this separately and does not
include it in the published plugin package.
