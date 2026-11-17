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

[Renovate Bot GitHub Action](https://github.com/renovatebot/github-action) runs
nightly at **03:17 UTC**, or manually through **Actions → Renovate → Run workflow**.
The workflow runs from `main` and targets only this repository. The hosted
Renovate GitHub App is not required.

Renovate manages Cargo dependencies in both the root crate and
`prototypes/direct-wasm`, plus GitHub Actions references. Updates are grouped
into `rust-dependencies` and `actions` PRs, including major updates. Action
references remain pinned to commit digests. CI runner and language-version
settings stay manually maintained. The Renovate action and its pinned bot
version are maintained in the `actions` update group. Dependency PRs use the
normal CI and review process; automerging is disabled.

### One-time authentication setup

Create a [classic personal access token](https://github.com/settings/tokens)
with **public_repo** and **workflow** scopes, following the action's
[authentication instructions](https://github.com/renovatebot/github-action#token).
Store it as the repository Actions secret **`RENOVATE_TOKEN`** in
**Settings → Secrets and variables → Actions**. Use an appropriate expiry and
replace the secret when the token expires. A private repository needs `repo`
instead of `public_repo`.

The PAT lets Renovate update workflow files and trigger the normal PR CI.
Using the default `GITHUB_TOKEN` would require manual approval for bot-created
PR workflows and does not provide the documented `workflow` token scope.
The job's default token remains read-only; only the bot action receives the PAT.
A maintainer must add the secret before the first run. A missing secret fails
with a clear setup message.

If the hosted Renovate app was enabled previously, disable it for this
repository so only the action runs. Action-created branches use
`renovate-action/`. The workflow runs only on its nightly schedule or a manual
request; it never runs on pull requests.

Bot execution settings are in `.github/workflows/renovate.yml`; dependency
policy is in `renovate.json`. The repository policy has no additional weekly
schedule, so every nightly run can create updates. PR CI validates workflow
syntax without invoking the bot.

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
