# Working in this repository

- Make changes on a topic branch and open a pull request. Never push directly to
  `main`, bypass required checks, or merge a PR without an explicit user request.
- Keep CI/workflow changes separate from product changes when they can be
  reviewed independently. Declare dependencies between stacked PRs.
- Preserve existing edits. Do not commit credentials, auth files, generated
  images, build outputs, or release archives.
- Keep the plugin's runtime dependencies small. Use the pinned Rust toolchain
  and `--locked` for Cargo checks and builds.
- Run checks appropriate to the changed behavior. Cover authentication and HTTP
  behavior at the protocol boundary, using fixtures instead of real accounts.
- Keep `manifest.toml`, `Cargo.toml`, and the crate's lockfile version aligned.
  Package the complete installable artifact and verify its checksum.
- Describe observed validation accurately. A passing mocked test does not verify
  a live subscription, and a WASM build does not verify admission by ZeroClaw.
- Lead user documentation with installation and use. Put implementation details
  in the reference/development sections.
