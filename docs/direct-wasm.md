# Direct WASM integration

Status: the root implementation is bridge-free. **Stock ZeroClaw cannot run
it yet.** The auth and artifact imports remain proposed host APIs.

The intended installation is a plugin package with `manifest.toml` and its
WASM component, followed by an explicit grant to use an existing Codex login.
The agent should generate an image with a single tool call. Users should not
need a Python service, another token, subprocess management, or polling.

This preserves automatic login reuse. Manually copying an access token would
make installation smaller but require maintenance every time it expires.

## Existing host boundaries

The host was rechecked at ZeroClaw master commit `cda933e6` on 2026-11-17.
The separately pinned stable plugin WIT in `wit/v0` remains unchanged; its
source revision is recorded in `NOTICE`.

| Need | What the current host provides | What is missing |
|---|---|---|
| Send a generation request | `wasi:http` with instance egress grants | Nothing for the HTTP request itself. |
| Use and refresh an existing subscription login | Native `AuthService` manages `openai-codex` profiles | A scoped plugin import delegating to that service. `secrets.get` only reads the plugin's own stored configuration. |
| Save generated image bytes | Native tools write images and return attachment markers | An artifact-writing service for tool plugins. Declaring `file_write` currently does not preopen a filesystem directory. |
| Wait for image generation | A configurable plugin call deadline | A suitable deadline for the admitted image tool; the default 30 seconds is too short for many generations. |

Source references:

- [`PluginState::new`](https://github.com/zeroclaw-labs/zeroclaw/blob/cda933e6b9937deacbcc0d4d2cd7f1ae5bb4500e/crates/zeroclaw-plugins/src/component.rs) builds `WasiCtx::builder().build()` without filesystem preopens. The same file implements instance-scoped secret reads.
- [`PluginHostServices`](https://github.com/zeroclaw-labs/zeroclaw/blob/cda933e6b9937deacbcc0d4d2cd7f1ae5bb4500e/crates/zeroclaw-plugins/src/services.rs) carries config and state services, with no provider-auth or image-artifact backend.
- [`AuthService`](https://github.com/zeroclaw-labs/zeroclaw/blob/cda933e6b9937deacbcc0d4d2cd7f1ae5bb4500e/crates/zeroclaw-providers/src/auth/mod.rs) already owns profile selection, encrypted token persistence, and OpenAI token refresh.
- [Codex's image client](https://github.com/openai/codex/blob/662a634/codex-rs/codex-api/src/endpoint/images.rs) posts a typed image-generation request to the provider's `images/generations` endpoint.

## Proposed host contract

The [active WIT](../wit/direct/services.wit) is deliberately
in a new `zeroclaw:proposed-image-services` package. It does not masquerade as an
existing ZeroClaw API and does not modify the vendored stable contract.

### Host-managed Codex credentials

`codex-auth.get()` returns a transient access token and account ID. It accepts
no guest-selected profile or credentials. The host derives the explicitly
bound profile from the admitted instance's live configuration and delegates
refresh to its existing `AuthService`.

The host must check a dedicated auth grant and the current execution frame
before resolving credentials. Metadata probes receive no credentials. The
operator authorizes the account/profile binding; a guest cannot switch to
another profile. Refresh tokens and persistent token storage remain host-owned.
Failures use detail-free `access-denied`, `login-required`, and `unavailable`
variants. A revoked grant must take effect on the next call.

The component receives an access token in guest memory, like an explicitly
granted API credential. Before accepting the contract, host reviewers should
decide whether that disclosure is appropriate or whether the host should inject
authentication into a destination-bound HTTP request instead. The latter can
keep all credentials outside guest memory but needs a broader HTTP contract.

The native provider uses ZeroClaw's auth profiles. Its existing
`zeroclaw auth login --model-provider openai-codex --import <auth.json>` flow
can import a Codex CLI auth file. A keyring-backed Codex CLI login is a separate
case: this proposal does not claim the stores are interchangeable or silently
create a second refresh-token owner. Profile selection, import, and concurrent
use by the CLI must be settled explicitly in the host change.

### Host-owned image artifacts

`artifacts.save-image(bytes)` validates the image and saves it in the admitted
instance's authorized workspace, returning a path and byte count. The guest
supplies no directory or filename.

The host must independently enforce its artifact grant, image validation,
byte/storage quotas, filesystem confinement, symlink handling, and exclusive
file creation. The returned artifact must be usable by ZeroClaw's existing
attachment delivery path. Raw base64 must not be put into the model's context
or persisted in session history to work around this missing capability.

### Generation stays in WASM

The plugin validates the prompt, obtains current credentials, and issues one
HTTP request to the fixed Codex images endpoint. It validates and decodes the
image response, calls the artifact service, and returns an attachment marker.
It does not manage login, spawn Codex, select arbitrary endpoints, or retry a
generation automatically. Authentication, endpoint access and billing remain
subject to the account's Codex entitlements.

The component fixes the model to Codex's observed `gpt-image-2`, requests one
image, supports transparency, and bounds images to 8 MiB. Backend protocol and
entitlement behavior remain unverified against a live subscription. The HTTP
shim reads successful responses in bounded chunks, aborting after the base64
image limit plus 64 KiB of JSON overhead. Error response bodies are discarded.

## Implementation and release gates

1. Agree the host auth and artifact contract, grants, profile binding, and call
   deadline behavior. These are host capabilities, not features a manifest can
   invent. Settle whether auth reuses a ZeroClaw profile or imports the existing
   Codex CLI login through a supported host flow.
2. Implement and test the host services outside `zeroclaw-runtime`'s holding
   crate, delegating to the canonical auth and artifact owners. Coordinate any
   necessary wiring/extraction with the upstream architecture owners.
3. Run a real guest against that host. Prove admission, denied/revoked grants,
   token refresh, artifact confinement, generation timeout/cancellation, and
   channel image delivery. A guest-only build does not prove these behaviors.
4. Verify one image with a real eligible subscription. Mark the minimum
   compatible host version accurately.
5. Document and test installation of the complete archive on that supported
   host, then replace the proposed API contract with its agreed version and
   enable public releases.

The Python bridge, loopback configuration, polling API, and separate prototype
crate have been removed. The root crate, manifest, CI, and package all use the
direct component. Python remains only in build and CI tooling and is excluded
from the runtime archive.

Until the host gates are met, `registry = false` keeps the plugin out of registry
discovery. CI produces a development archive for review; the release workflow
runs `tools/check_release_ready.py` and refuses to publish the proposed contract.

## Validation boundary

The [root implementation](../src/lib.rs) contains a thin WIT shim, a direct
HTTP generation core, and host-fixture tests. Tests cover single-call success,
fresh credential resolution, auth denial, header injection, fixed request
parameters, provider/transport failures without automatic retry or error-body
leakage, bounded response streaming, image validation/limits, and artifact
errors. Packaging tests assert the complete archive contents and reproducible
checksums, including exclusion of legacy bridge files. CI also builds the guest
against the proposed interfaces and tests the publication guard.

The host services themselves are not implemented in this repository. They need
an upstream ZeroClaw change and review; no stock-host compatibility or live
generation is claimed.
