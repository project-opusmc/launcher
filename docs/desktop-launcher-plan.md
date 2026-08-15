# RBW desktop launcher plan

## User-facing flow

1. Open **RBW Client**.
2. Dashboard shows account, locked Forge 1.8.9 runtime, OptiFine import, and
   Java runtime health.
3. Open **Account** and select **Sign in with Microsoft**. RBW opens
   Microsoft's normal account sign-in in the system browser. Account selection,
   password and consent stay on Microsoft's page; a PKCE-protected localhost
   callback returns the result to RBW automatically. Only a redacted profile is
   shown in the UI.
4. Import the player's locally obtained exact OptiFine 1.8.9 HD U M5 JAR in
   Settings. RBW verifies and isolates a copy; it never downloads or
   redistributes OptiFine.
5. Set memory and launcher preferences in Settings.
6. Press **Launch**. The launcher verifies the Forge + OptiFine profile,
   creates an isolated session, and starts Forge through its managed Java
   runtime.
7. Minecraft opens as the normal Forge 1.8.9 client: Singleplayer, Multiplayer,
   Options and Quit remain vanilla UI.

## QA offline-demo flavor

The QA artifact is a separate debug launcher used before Premium sign-in is
released. It has its own Tauri identifier and icon, its own runtime/settings
root, and packages the same verified Forge bridge/coremod as the Premium
application. It can install and launch the managed Forge profile rather than
simulating a game lifecycle, but it still requires a locally imported verified
OptiFine JAR.

QA replaces Microsoft sign-in, ownership verification, and keychain access
with a saved Minecraft-compatible username. The backend builds a deterministic
offline Minecraft identity for that name, so it never receives or stores a
Premium credential. The resulting session is intentionally accepted only by
servers with `online-mode=false`; a correctly configured `online-mode=true`
server rejects it through normal Minecraft session authentication.

Offline-mode is not an identity or access-control system. QA demos must use
operator-controlled servers with an appropriate whitelist and moderation
policy, because a player can choose another player's offline username. The QA
bundle is not a fallback for the Premium launcher and must not be published as
a production artifact. See [QA offline-demo operations](qa-offline-demo.md).

## Components

| Component | Responsibility |
| --- | --- |
| `desktop/` | React/TypeScript launcher views and local presentation state. |
| `desktop/src-tauri/` | Tauri commands, non-secret settings and background jobs. |
| `launcher/rbw-runtime` | Verified Mojang + locked Forge install/cache, local OptiFine import, native extraction and launch plan. |
| `launcher/rbw-auth` | Microsoft browser OAuth with PKCE, Xbox/Minecraft exchange and keychain refresh credential. |
| `game/` | Java 8 Forge bridge, RBW coremod, Forge transformer adapter, telemetry, and in-game UI. |
| macOS game stub | LaunchServices app that immediately `exec`s the managed Java 8 process, preserving its foreground app identity. |

## External release prerequisite

RBW's embedded public Client ID must be reviewed and manually allow-listed by
Mojang before Minecraft Services will issue a Java session. Submit the exact ID
once through the official [AppID Review form](https://aka.ms/mce-reviewappid),
retain that ID in the shipped application, and wait for approval. End users do
not configure this and RBW must not borrow another launcher's Client ID.

## Security invariants

- No browser-visible token, refresh credential or game access token.
- No token in an OS command line, launcher log or vanilla game log.
- Only checksum-verified managed artifacts enter the game classpath; OptiFine
  is checksum-verified after a local user import and is never downloaded or
  bundled by RBW.
- QA permits only the verified RBW coremod and exact imported OptiFine JAR in
  its managed Forge mod set.
- On macOS, Minecraft 1.8.9 launches through a compiled app stub. JVM options intended for later `FirstThreadOnMacOS` snapshots are not applied to 1.8.9.
- The launcher ships with its first-party public Microsoft application Client
  ID. It opens the official Microsoft account page in the system browser and
  accepts only a matching PKCE/state-protected localhost callback; refresh
  credentials remain in the operating-system keychain.
- An authenticated game session requires a keychain-backed account, not a
  user-supplied Client ID or client secret.
- Settings cannot change the locked Minecraft version or escape RBW's data
  directory.
