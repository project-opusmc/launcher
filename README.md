# Opus

Opus consists of **Opus Launcher**, a native desktop launcher, and **Opus Client**,
the controlled Forge + OptiFine Minecraft Java Edition 1.8.9 runtime it starts.
Launcher state, accounts, isolated instances, and managed game files remain
separate from the normal `.minecraft` installation.

Working on this repository? Start at [docs/README.md](docs/README.md) for the
architecture, module map, IPC contract, and invariants.

## Desktop launcher

Build the production launcher with Microsoft authentication, ownership
verification, installation/repair, and real Minecraft launch support:

```bash
npm --prefix desktop install
npm --prefix desktop run tauri:build:premium
open "target/release/bundle/macos/Opus Launcher.app"
```

On macOS, Opus Launcher starts the managed game through the bundled
`Opus Client.app` LaunchServices stub. The stub immediately executes the pinned
Java runtime while preserving a foreground application identity.

The launcher uses a pane-based graphical TUI with keyboard and mouse support.
`Shift + Arrow` moves between panes; plain arrow keys navigate within the
focused pane.

## Accounts and profiles

The unified account catalog supports:

- multiple Microsoft accounts, each displayed with its verified Minecraft
  profile name and an `[OFFICIAL]` or `[PREMIUM]` tag;
- multiple offline profiles displayed as `[UNOFFICIAL]`;
- explicit identity selection before launch;
- concurrent Minecraft instances for different identities;
- one isolated game directory per Minecraft UUID.

Microsoft passwords and authorization codes never enter Opus Launcher. Sign-in
opens Microsoft's official page in the system browser, uses PKCE with a bounded
localhost callback, and stores only the refresh credential in the operating
system keychain.

The default data root is `~/.opus-launcher` on macOS/Linux or
`AppData/Local/OpusLauncher` on Windows. Existing `.rbw-client` data and legacy
Keychain entries are migrated automatically. `OPUS_HOME` overrides the normal
root; the legacy `RBW_HOME` alias remains accepted during migration.

## QA lane

The separate QA bundle remains an internal debug artifact, not a second user
product:

```bash
npm --prefix desktop run tauri:build:qa
```

It is written to `target/debug/bundle/macos/Opus Launcher QA.app` and uses an
isolated data root. User-facing offline profiles belong in the unified Opus
Launcher account list and carry the `[UNOFFICIAL]` tag.

## Microsoft application registration

Opus Launcher includes its first-party public Microsoft application Client ID.
Minecraft Services must also approve that exact ID through Mojang's official
[AppID Review form](https://aka.ms/mce-reviewappid). An Entra registration alone
is not sufficient, and end users must never substitute another launcher's ID.

For CLI-only development, `OPUS_MICROSOFT_CLIENT_ID` is the preferred override;
`RBW_MICROSOFT_CLIENT_ID` remains a compatibility alias.

## CLI diagnostics

```bash
./game/gradlew -p game test prepareBootstrap
cargo run -p rbw-launcher -- doctor
cargo run -p rbw-launcher -- install
cargo run -p rbw-launcher -- import-optifine /path/to/OptiFine_1.8.9_HD_U_M5.jar
cargo run -p rbw-launcher -- launch --offline --dry-run
```

An actual macOS CLI game launch is deliberately blocked; use the packaged
`Opus Launcher.app` so LaunchServices can preserve the game app identity.
Mojang and Forge artifacts are verified against locked contracts. OptiFine is
provided locally by the user and is never downloaded, bundled, or redistributed
by this repository.
