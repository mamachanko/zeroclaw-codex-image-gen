# Contributing

All changes go through pull requests. Create a topic branch, make a focused
change, and open a PR against `main`. Keep the title and description about the
resulting behavior, with the validation performed. Merge after review and a
successful **Required checks** status.

## Checks

The Rust version and WASM target are defined in `rust-toolchain.toml`.

```bash
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --target wasm32-wasip2 -- -D warnings
cargo build --locked --target wasm32-wasip2 --release
python3 -m unittest discover -s bridge -p 'test_*.py' -v
python3 tools/package.py
```

PR CI validates workflows, runs Python tests, Rust tests and lint checks, builds the WASM component,
and uploads the installable ZIP and checksum. The final `Required checks` job
fails if any dependency fails, is skipped, or is cancelled. CI also runs on
`main`, merge queues, and manual requests. Workflows use read-only permissions
except for the release publication job, and PR runs do not use account secrets.

## Dependency updates

[Renovate](https://github.com/apps/renovate) manages Cargo dependencies in both
the root crate and `prototypes/direct-wasm`, plus GitHub Actions references.
Updates are grouped into `rust-dependencies` and `actions` PRs, including major
updates, and scheduled for Mondays from 00:00 to 05:59 UTC. Action references
remain pinned to commit digests. CI runner and language-version settings stay
manually maintained. Dependency PRs use the normal CI and review process;
automerging is disabled.

The hosted Renovate GitHub App must have access to this repository. Enable it
for `mamachanko/zeroclaw-codex-image-gen` when adopting this configuration; the
JSON file alone does not run the bot. No Renovate workflow or repository token
is needed when using the hosted app. The configuration is in `renovate.json`.

To validate configuration changes with Node.js 24.11 or newer in the 24.x line:

```bash
npx --yes --package=renovate@44.247.2 renovate-config-validator --strict renovate.json
```

## Repository protection

Configure a ruleset for `main` in **Settings → Rules → Rulesets**:

- Require a pull request before merging.
- Require the `Required checks` status and an up-to-date branch.
- Block force pushes and deletion.

The ruleset should be active and apply to administrators as well. A minimum
number of approving reviewers can be chosen separately for the team's size.
Adding workflow files alone does not enforce this repository setting.

A repository administrator can apply the supplied ruleset with:

```bash
gh api repos/mamachanko/zeroclaw-codex-image-gen/rulesets \
  --method POST --input .github/main-ruleset.json
```

The supplied ruleset requires PRs and successful CI without a separate reviewer
approval, so a single-maintainer repository can still merge its own PRs. It has
no bypass actors and accepts the required check only from GitHub Actions.

## Direct WASM prototype

The proposed replacement is reviewed separately in
[`docs/direct-wasm.md`](docs/direct-wasm.md). Its standalone crate is in
`prototypes/direct-wasm`; run the same Rust checks there when changing it.
CI validates that crate as an additional required job. It is not packaged or
released because stock ZeroClaw does not implement its proposed host services.

## Releases

After the changes have merged, a maintainer can publish a release by creating
and pushing a version tag, for example `v0.1.0`. The tag must match
`manifest.toml` and point to a commit reachable from `main`.

The release workflow reruns CI, downloads its package artifact, verifies the
SHA-256 checksum, and creates a GitHub release with the ZIP and checksum. Tags
are a deliberate maintainer action; opening or merging a PR does not publish a
release. Do not move an existing release tag.
