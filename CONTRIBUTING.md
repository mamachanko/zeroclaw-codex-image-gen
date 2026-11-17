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
`main`, merge queues, and manual requests. Workflows default to read-only
permissions. Release publication and scheduled Renovate jobs have scoped write
permissions; PR CI does not use account secrets.

## Dependency updates

[Renovate](.github/workflows/renovate.yml) runs its pinned
`ghcr.io/renovatebot/renovate:44.247.2` Docker image directly, nightly at
**03:17 UTC** or through **Actions → Renovate → Run workflow**. This follows
[Pink’s working setup](https://github.com/mamachanko/pink/blob/f29920a53e08de127b16a4919791038e1b6b72fe/.github/workflows/renovate.yml).

It uses GitHub’s built-in `GITHUB_TOKEN`. No personal access token, Renovate
account, hosted app, or repository secret is needed. `RENOVATE_TOKEN` inside
the container is simply Renovate’s environment variable for that built-in token.
The job grants Contents, Issues and Pull requests write permissions, targets
only this repository on `main`, and skips onboarding. Its identity is
`github-actions[bot]`. Runs are serialized, and the timeout is 60 minutes.

### One-time setup

1. In **Settings → Actions → General → Workflow permissions**, enable
   **Allow GitHub Actions to create and approve pull requests**. GitHub Actions
   and repository Issues must also be enabled; the recommended Renovate preset
   uses a Dependency Dashboard issue.
2. If the hosted Renovate app previously served this repository, remove this
   repository from that app’s access to avoid competing bots.
3. Merge the workflow to `main`, then run it manually once and inspect its logs
   and Dependency Dashboard. GitHub schedules use the default branch and may
   be delayed. No update PR is expected when dependencies are current.

### What gets updated

Renovate manages Cargo dependencies in both the root crate and
`prototypes/direct-wasm`. Updates, including major updates, are grouped into
`rust-dependencies` PRs. Automerging is disabled.

The built-in token cannot update workflow files, so the GitHub Actions manager
is disabled. Review action commit pins and the Renovate Docker image version
manually. CI runner and language-version pins also remain manually maintained.
The dependency policy has no additional weekly schedule; every nightly run can
create Cargo updates.

### CI for bot-created PRs

GitHub may require maintainer approval before CI runs on a PR created with
`GITHUB_TOKEN`. Approve the pending workflow when GitHub offers that option.
If no run is created, use **Actions → CI → Run workflow**, select the PR’s
current head branch, and enter its number in the `pull_request` field.

The same fallback is available from the CLI. Replace this example’s branch and
PR number with the actual PR values:

```bash
gh workflow run ci.yml --repo mamachanko/zeroclaw-codex-image-gen \
  --ref renovate/rust-dependencies -f pull_request=123
```

The `Verify CI target` job checks that the PR is open, belongs to this
repository, and has the exact branch and commit selected for the run. A stale
or unrelated run fails the gate. The normal `Required checks` job still
requires every validation and build job to pass. General manual CI runs can
omit the PR number; selecting a PR number activates the stricter check.
Review CodeQL status as well, and do not bypass required checks.

Execution settings are in `.github/workflows/renovate.yml`; policy is in
`renovate.json`. The write-enabled bot never runs on PR events. To validate
configuration changes with Node.js 24.11 or newer in the 24.x line:

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
