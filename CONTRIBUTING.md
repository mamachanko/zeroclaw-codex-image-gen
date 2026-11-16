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

## Releases

After the changes have merged, a maintainer can publish a release by creating
and pushing a version tag, for example `v0.1.0`. The tag must match
`manifest.toml` and point to a commit reachable from `main`.

The release workflow reruns CI, downloads its package artifact, verifies the
SHA-256 checksum, and creates a GitHub release with the ZIP and checksum. Tags
are a deliberate maintainer action; opening or merging a PR does not publish a
release. Do not move an existing release tag.
