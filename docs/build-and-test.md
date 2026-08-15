# Build, Run, And Test

This repository builds Opus Launcher independently. It does not compile OPUS
Runtime source; desktop packages consume a verified Runtime artifact directory.

## Toolchain

| Tool | Version / requirement |
| --- | --- |
| Rust | `1.92.0`, with `rustfmt` and `clippy` |
| Node.js | 24 |
| npm | compatible with the checked-in lockfile |
| Tauri prerequisites | native requirements for Tauri 2 on the host platform |

Minecraft uses the managed Java 8 runtime downloaded and verified by Launcher.
A developer JDK is not required for building this repository.

## Source Gate

Install frontend dependencies once, then run the complete local source gate:

```bash
npm ci --prefix desktop
./scripts/check.sh
```

The script runs:

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. QA/UI Preview feature clippy with warnings denied
4. `cargo test --workspace`
5. QA-edition Rust tests
6. `npm --prefix desktop run check`
7. `npm --prefix desktop run build`

With live access to Mojang's official endpoints, add the pinned metadata test:

```bash
OPUS_CHECK_OFFICIAL=1 ./scripts/check.sh
```

CI runs the same gate on macOS 14. Runtime has its own Java/Gradle gate in the
`project-opusmc/runtime` repository; the OPUS superproject runs both component
gates for an integration release.

## Runtime Artifact Staging

Build Runtime first or point to a verified Runtime output containing:

```text
runtime-manifest.json
runtime-checksums.json
artifacts/opus-bootstrap-<version>.jar
artifacts/opus-runtime-legacy-1.8.9-<version>.jar
artifacts/opus-client-legacy-1.8.9-<version>.jar
```

Stage and verify it with:

```bash
OPUS_RUNTIME_ARTIFACT_DIR=/path/to/runtime/build/runtime \
  ./scripts/prepare-desktop-assets.sh
```

`stage-runtime-artifacts.mjs` validates the schema, artifact roles, filenames,
sizes, and SHA-256 values before copying anything into Tauri resources.

## Desktop Bundles

Premium:

```bash
OPUS_RUNTIME_ARTIFACT_DIR=/path/to/runtime/build/runtime \
  npm --prefix desktop run tauri:build:premium
```

Output on macOS:
`target/release/bundle/macos/Opus Launcher.app`.

Internal QA:

```bash
OPUS_RUNTIME_ARTIFACT_DIR=/path/to/runtime/build/runtime \
  npm --prefix desktop run tauri:build:qa
```

Output on macOS:
`target/debug/bundle/macos/Opus Launcher QA.app`. QA is intentionally a debug,
offline-only bundle with an isolated identifier and data root.

Install locally after building:

```bash
npm --prefix desktop run tauri:install:premium
npm --prefix desktop run tauri:install:qa
```

The installers refuse to replace a running OPUS app, stage and validate the
new bundle, ad-hoc sign it for local use, and move an existing same-name bundle
to Trash before replacement.

## CLI Diagnostics

```bash
cargo run -p opus-cli -- doctor
cargo run -p opus-cli -- install
cargo run -p opus-cli -- import-optifine /path/to/OptiFine_1.8.9_HD_U_M5.jar
cargo run -p opus-cli -- launch --offline --dry-run
```

On macOS, a real CLI game launch is blocked because the packaged
`Opus Client.app` LaunchServices stub is required to preserve the foreground
application identity. Use the packaged desktop Launcher for a real launch.

## Environment Variables

| Variable | Effect |
| --- | --- |
| `OPUS_RUNTIME_ARTIFACT_DIR` | Verified Runtime output used for desktop packaging. |
| `OPUS_HOME` | Overrides the Premium data root. |
| `OPUS_QA_HOME` | Overrides the QA data root; must be absolute and isolated. |
| `OPUS_UI_PREVIEW_HOME` | Overrides the UI Preview data root; must be absolute and isolated. |
| `OPUS_MICROSOFT_CLIENT_ID` | CLI-only OAuth client ID override. |
| `OPUS_CHECK_OFFICIAL=1` | Adds the live Mojang metadata test. |
| `OPUS_GAME_WORKDIR` | Internal variable set for the macOS game stub. |

For a disposable QA run:

```bash
OPUS_QA_HOME="$HOME/tmp/opus-launcher-qa" \
OPUS_RUNTIME_ARTIFACT_DIR=/path/to/runtime/build/runtime \
  npm --prefix desktop run tauri:dev:qa
```

## Test Ownership

| Area | Coverage |
| --- | --- |
| `crates/platform` | platform detection, Java parsing, and data-root isolation |
| `crates/engine` | pinned metadata, downloads, install state, Forge staging, launch plans, native extraction, and secret-free process arguments |
| `crates/auth` | PKCE, callback validation, cancellation, token redaction, and Minecraft service error handling |
| `desktop/src-tauri` | settings, multi-account catalog, multi-instance coordination, IPC payloads, and QA profile validation |
| `desktop/src` | TypeScript contracts and production frontend build |

Session diagnostics and launch triage are documented in
[diagnostics.md](diagnostics.md).
