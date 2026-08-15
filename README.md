# RBW Client

RBW Client is a desktop launcher and controlled **Forge + OptiFine** Minecraft
Java Edition 1.8.9 runtime for Ranked Bedwars. It keeps launcher UI, account,
and managed game files separate from the normal `.minecraft` installation.

Working on this repository? Start at [docs/README.md](docs/README.md) for the
architecture, module map, IPC contract, and the invariants that must not
regress.

## Desktop launcher (macOS)

The normal entry point is the packaged native launcher, not a raw Java process:

```bash
npm --prefix desktop install
npm --prefix desktop run tauri -- build --debug
open "target/debug/bundle/macos/RBW Client.app"
```

The build prepares the Forge bridge and RBW coremod. On macOS, RBW launches a small native
game app through LaunchServices; it immediately `exec`s Mojang's managed Java
8 runtime. This gives the game a foreground app identity without changing AWT,
Carbon, JNI, or the JVM's main thread.

## Desktop artifact flavors

Build the normal Premium launcher for real Microsoft sign-in, Java Edition
ownership verification, verified installation, and Minecraft launch:

```bash
npm --prefix desktop run tauri:build:premium
```

On macOS, the application bundle is written to
`target/release/bundle/macos/RBW Client.app`.

Build the separate QA offline-demo launcher:

```bash
npm --prefix desktop run tauri:build:qa
```

On macOS, the QA application bundle is written to
`target/debug/bundle/macos/RBW Client Demo.app`. It uses the separate bundle ID
`dev.rbw.client.qa`, QA icon, QA-only data root, and bundles the verified Forge
bridge/coremod. The QA build remains a debug artifact for controlled demos; it is
not a Premium release build.

QA asks the player to choose a Minecraft-compatible offline username instead
of signing in with Microsoft. It never performs ownership verification or
stores a Microsoft credential. It can install the managed Forge 1.8.9 runtime.
Before launch, the player imports their own local `OptiFine_1.8.9_HD_U_M5.jar`;
RBW verifies and copies it into its isolated runtime but never downloads or
redistributes OptiFine. Its offline session can join only servers configured with
`online-mode=false`; an `online-mode=true` server rejects the session during
normal Minecraft authentication. See [the QA offline-demo notes](docs/qa-offline-demo.md)
for operational limits and server-safety requirements.

RBW includes its first-party public Microsoft application Client ID. Open
**Account** and choose **Sign in with Microsoft**; RBW opens Microsoft's
official sign-in in the system browser. After account selection, password and
consent stay on Microsoft's page; its PKCE-protected localhost callback returns
the result to RBW automatically. Do not enter a client secret, password or
token anywhere in RBW. The launcher shows only a redacted profile and stores
the refresh credential only in the operating-system keychain.

Minecraft Services separately reviews and allow-lists new Java game-service
integrations. Before a new RBW Client ID can complete sign-in, the application
owner must submit that exact ID through Mojang's official
[AppID Review form](https://aka.ms/mce-reviewappid) and wait for approval. An
Entra registration alone is not sufficient; end users must never substitute a
Client ID from another launcher.

### QA development

For local QA/demo work while the AppID review is pending, run the separate QA
application in development mode:

```bash
npm --prefix desktop run tauri:dev:qa
```

It prepares the same Forge bridge/coremod used by the launcher, then starts the
locked Forge + locally imported OptiFine profile with the saved offline username.
QA data is isolated from
Premium data: by default it lives at `~/.rbw-client-qa` on macOS/Linux or
`AppData/Local/RBWClientQA` on Windows, and it can be moved for a local test
with `RBW_QA_HOME`. QA does not use the Microsoft account/keychain flow.

The QA Cargo feature is non-default and the QA command intentionally produces
a debug bundle. Do not distribute it as a Premium build or use it on an
uncontrolled offline-mode server.

## CLI diagnostics

For launcher-created, per-session diagnostic files, their privacy boundary,
and guidance on interpreting frame/tick/render percentiles, see
[per-session diagnostics](docs/diagnostics.md).

```bash
./game/gradlew -p game test prepareBootstrap
cargo run -p rbw-launcher -- doctor
cargo run -p rbw-launcher -- install
cargo run -p rbw-launcher -- import-optifine /path/to/OptiFine_1.8.9_HD_U_M5.jar
cargo run -p rbw-launcher -- launch --offline --dry-run
```

`launch` verifies the complete cached installation before starting. On macOS,
an actual CLI game launch is deliberately blocked: use `RBW Client.app`, whose
LaunchServices game stub preserves a foreground game identity. Once an install
has succeeded, a valid cache can launch without network access; missing or
corrupt artifacts trigger an official Mojang download and repair. Game data is
isolated under the RBW data directory rather than the user's `.minecraft`.

`--offline` is a development profile for local singleplayer and UI testing. It
cannot authenticate to online-mode multiplayer servers.

The packaged desktop launcher is the normal sign-in path and uses its built-in
first-party public Client ID. Its **Account** view opens the official Microsoft
page in the system browser and receives the result automatically through a
PKCE-protected localhost callback. No password, authorization code or token is
copied into the launcher UI.

The following environment-variable flow is retained only for non-desktop CLI
development and diagnostics. It is not required for RBW Client.app users. In
both flows, only the refresh credential is stored in the operating-system
keychain.

```bash
export RBW_MICROSOFT_CLIENT_ID="<public-desktop-app-client-id>"
cargo run -p rbw-launcher -- account login
cargo run -p rbw-launcher -- launch
```

Only the pinned Forge + OptiFine Minecraft 1.8.9 profile is supported. Mojang
and Forge artifacts are verified against locked contracts; OptiFine is
user-provided locally and is never committed, downloaded, bundled, or
redistributed by this repository.
