# Opus Launcher

Opus Launcher is the native desktop launcher for OPUS. It owns account and
profile management, installation and repair, platform integration, runtime
artifact verification, isolated game instances, and Minecraft process
lifecycle. Java code that runs inside Minecraft belongs to the separate
`project-opusmc/runtime` repository.

This repository is independently buildable. In product releases it is pinned
as the `launcher/` submodule of the `project-opusmc/opus` superproject.

## Workspace

```text
launcher/
|- crates/auth/       Microsoft and Minecraft authentication
|- crates/platform/   OS paths, Java discovery, and process integration
|- crates/engine/     install, verify, repair, and launch planning
|- crates/cli/        diagnostics and command-line operations
|- desktop/           React TUI and Tauri desktop application
|- docs/
`- scripts/
```

The graphical TUI supports keyboard and mouse input. `Shift + Arrow` changes
the focused pane; plain arrow keys navigate within the current pane.

## Runtime Artifacts

Launcher consumes the immutable OPUS Runtime contract:

```text
runtime-manifest.json
runtime-checksums.json
artifacts/opus-bootstrap-<version>.jar
artifacts/opus-runtime-legacy-1.8.9-<version>.jar
artifacts/opus-client-legacy-1.8.9-<version>.jar
```

Set `OPUS_RUNTIME_ARTIFACT_DIR` to a verified Runtime output directory before a
desktop build. In the superproject this staging is handled by the root build
scripts.

```bash
OPUS_RUNTIME_ARTIFACT_DIR=/path/to/runtime/build/runtime \
  ./scripts/prepare-desktop-assets.sh
```

OptiFine is never downloaded, bundled, or redistributed by OPUS. The user must
import the exact supported local JAR, which Launcher verifies before copying it
into an isolated instance.

## Build And Check

Requirements: Rust 1.92, Node.js 24, npm, and the native prerequisites required
by Tauri 2.

```bash
npm ci --prefix desktop
./scripts/check.sh
```

Build the Premium or internal QA macOS bundle with:

```bash
OPUS_RUNTIME_ARTIFACT_DIR=/path/to/runtime/build/runtime \
  npm --prefix desktop run tauri:build:premium

OPUS_RUNTIME_ARTIFACT_DIR=/path/to/runtime/build/runtime \
  npm --prefix desktop run tauri:build:qa
```

## Accounts And Instances

The unified catalog supports multiple Microsoft accounts and multiple offline
profiles. Microsoft identities display their verified Minecraft profile name
with `[OFFICIAL]` or `[PREMIUM]`; offline profiles display `[UNOFFICIAL]`.
Different identities can launch concurrently, while duplicate launches for the
same identity are rejected.

Microsoft passwords and authorization codes never enter Launcher. Browser
sign-in uses PKCE and a bounded localhost callback; refresh credentials are
stored in the operating-system keychain.

Default data roots are `~/.opus-launcher`, `~/.opus-launcher-qa`, and
`~/.opus-launcher-ui-preview` on macOS/Linux, with corresponding
`OpusLauncher*` directories below Local App Data on Windows. `OPUS_HOME`,
`OPUS_QA_HOME`, and `OPUS_UI_PREVIEW_HOME` override their own isolated lanes.
Launcher does not import, delete, or rewrite data belonging to earlier products.

## License

Copyright (c) 2026 Polydevs. All rights reserved. See [LICENSE](LICENSE).
