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
3. `cargo test --workspace`
4. `npm --prefix desktop run check`
5. `npm --prefix desktop run build`

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
```

Stage and verify it with:

```bash
OPUS_RUNTIME_ARTIFACT_DIR=/path/to/runtime/build/runtime \
  ./scripts/prepare-desktop-assets.sh
```

`stage-runtime-artifacts.mjs` validates the schema, artifact roles, filenames,
sizes, and SHA-256 values before copying anything into Tauri resources.
The production staging gate also verifies that launcher windows remain
non-persistent (`incognito: true`) so the file-owned vault is the only
credential store. It rejects any unexpected Runtime artifact rather than
allowing an old UI/client JAR back into a bundle.

## Desktop Bundles

Unified launcher:

```bash
OPUS_RUNTIME_ARTIFACT_DIR=/path/to/runtime/build/runtime \
  npm --prefix desktop run tauri:build
```

Output on macOS:
`target/release/bundle/macos/Opus Launcher.app`.

The bundle command runs `assert-file-only-bundle.sh` after Tauri creates the
macOS app. A successful build therefore verifies the file-only credential
contract and the Core-Mod-first artifact boundary (two runtime JARs only, no
retired CEF/client payload, and the stock `Minecraft` host/Dock label).

Install locally after building:

```bash
npm --prefix desktop run tauri:install
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
`Minecraft.app` LaunchServices host stub is required to preserve the foreground
application identity. Use the packaged desktop Launcher for a real launch.

## Environment Variables

| Variable | Effect |
| --- | --- |
| `OPUS_RUNTIME_ARTIFACT_DIR` | Verified Runtime output used for desktop packaging. |
| `OPUS_HOME` | Overrides the unified launcher data root. |
| `OPUS_MICROSOFT_CLIENT_ID` | CLI-only OAuth client ID override. |
| `OPUS_CHECK_OFFICIAL=1` | Adds the live Mojang metadata test. |
| `OPUS_GAME_WORKDIR` | Internal variable set for the macOS game stub. |

## Test Ownership

| Area | Coverage |
| --- | --- |
| `crates/platform` | platform detection, Java parsing, and data-root isolation |
| `crates/engine` | pinned metadata, downloads, install state, Forge staging, launch plans, native extraction, and secret-free process arguments |
| `crates/auth` | PKCE, callback validation, cancellation, token redaction, and Minecraft service error handling |
| `desktop/src-tauri` | settings, unified official/offline account catalog, multi-instance coordination, and IPC payloads |
| `desktop/src` | TypeScript contracts and production frontend build |

Session diagnostics and launch triage are documented in
[diagnostics.md](diagnostics.md).
