# Opus Launcher

Opus Launcher is the native desktop launcher for OPUS. It owns account and
profile management, installation and repair, platform integration, runtime
artifact verification, isolated game instances, and Minecraft process
lifecycle. Java code that runs inside Minecraft belongs to the separate
`project-opusmc/runtime` repository.

> **Product-direction notice — September 15, 2026:** in the Opus superproject,
> [Decision 0008](../docs/decisions/0008-opus-client-launcher-ui-first-mainline.md)
> makes Opus Client + Opus Launcher, UI first, the active mainline. This
> Launcher remains the product's desktop surface, but the artifact, staging,
> and launch behavior documented below are the current implementation baseline;
> no UI-first delivery or runtime-contract change is claimed here.

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
artifacts/opus-lwjgl-macos-compat-2.9.2-nightly-20140822.jar
artifacts/opus-native-ui-1.8.9-0.1.0.jar
artifacts/opus-oneconfig-1.8.9-forge-0.2.2-alpha228-full.jar
artifacts/opus-elementa-762.jar
artifacts/opus-universalcraft-1.8.9-forge-516.jar
```

Launcher stages the Core Mod, sealed LWJGL macOS compatibility artifact, native
Elementa shell bridge, branded OpusConfig, Elementa, and UniversalCraft beside
locally imported OptiFine. No CEF/webview helper or retired monolithic client
JAR is part of the unified runtime contract.
Before Forge scans an isolated instance, Launcher removes a known stale
`opus-client-legacy-1.8.9.jar`.

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

Build the one unified macOS bundle with:

```bash
OPUS_RUNTIME_ARTIFACT_DIR=/path/to/runtime/build/runtime \
  npm --prefix desktop run tauri:build
```

## Accounts And Instances

The unified catalog supports multiple Microsoft accounts and multiple offline
profiles. Microsoft identities display their verified Minecraft profile name
with `[OFFICIAL]` or `[PREMIUM]`; offline profiles display `[UNOFFICIAL]`.
Different identities can launch concurrently, while duplicate launches for the
same identity are rejected.

Microsoft passwords and authorization codes never enter Launcher. Browser
sign-in uses PKCE and a bounded localhost callback; refresh credentials are
stored in Opus's private local credential vault, so routine tests do not invoke
the macOS Keychain prompt.

The sole data root is `~/.opus-launcher` on macOS/Linux, with
`%LOCALAPPDATA%/OpusLauncher` on Windows. `OPUS_HOME` selects an isolated
root when needed. On first use, the launcher imports a valid offline username
from the retired QA data directory without deleting that historical file.
Historical UI Preview source is quarantined and has no supported production
launcher build or launch path.

## License

Copyright (c) 2026 Polydevs. All rights reserved. See [LICENSE](LICENSE).
