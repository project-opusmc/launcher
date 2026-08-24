# OPUS desktop launcher plan

## User-facing flow

1. Open **Opus Launcher**.
2. Dashboard shows the account catalog, active instances, locked Forge 1.8.9
   runtime, OptiFine import, and Java runtime health.
3. Open **Account** to add any number of Microsoft accounts or unofficial
   offline profiles. Microsoft sign-in opens the normal Microsoft page in the
   system browser; password and consent remain there. A PKCE-protected localhost
   callback returns the verified Minecraft profile to OPUS.
4. Import the player's locally obtained exact OptiFine 1.8.9 HD U M5 JAR in
   Settings. OPUS verifies and isolates a copy; it never downloads or
   redistributes OptiFine.
5. Set memory and launcher preferences in Settings.
6. Select an identity and press **Launch**. The launcher verifies the Forge +
   OptiFine profile, creates an identity-isolated instance, and starts Forge
   through its managed Java runtime. Different identities may run concurrently;
   a duplicate launch for the same identity is rejected.
7. Minecraft opens as the normal Forge 1.8.9 client: Singleplayer, Multiplayer,
   Options and Quit remain vanilla UI.

## Unified account catalog

One Opus Launcher bundle owns both Microsoft and offline identities. The
offline path stores only a Minecraft-compatible username, derives its
deterministic offline UUID, and never reads a Microsoft credential or opens the
credential vault for that identity. It uses the same verified Forge bridge,
Core Mod, and locally imported OptiFine contract as an official identity.

Offline mode is not an identity or access-control system. Offline sessions
must use operator-controlled `online-mode=false` servers with an appropriate
whitelist and moderation policy, because a player can choose another player's
offline username. See [offline profile operations](qa-offline.md).

## Components

| Component | Responsibility |
| --- | --- |
| `desktop/` | React/TypeScript launcher views and local presentation state. |
| `desktop/src-tauri/` | Tauri commands, non-secret settings and background jobs. |
| `crates/engine/` | Verified Mojang + locked Forge install/cache, local OptiFine import, native extraction and launch plan. |
| `crates/auth/` | Microsoft browser OAuth with PKCE, Xbox/Minecraft exchange and Opus-local refresh credential vault. |
| OPUS Runtime artifacts | Versioned bootstrap, Forge coremod, telemetry, transformers, and in-game UI supplied by the Runtime repository. |
| macOS game stub | LaunchServices app that immediately `exec`s the managed Java 8 process, preserving its foreground app identity. |

## External release prerequisite

OPUS's embedded public Client ID must be reviewed and manually allow-listed by
Mojang before Minecraft Services will issue a Java session. Submit the exact ID
once through the official [AppID Review form](https://aka.ms/mce-reviewappid),
retain that ID in the shipped application, and wait for approval. End users do
not configure this and OPUS must not borrow another launcher's Client ID.

## Security invariants

- No browser-visible token, refresh credential or game access token.
- No token in an OS command line, launcher log or vanilla game log.
- Only checksum-verified managed artifacts enter the game classpath; OptiFine
  is checksum-verified after a local user import and is never downloaded or
  bundled by OPUS.
- The unified launcher permits only the verified OPUS coremod and exact
  imported OptiFine JAR in every managed Forge mod set.
- On macOS, Minecraft 1.8.9 launches through a compiled app stub. JVM options intended for later `FirstThreadOnMacOS` snapshots are not applied to 1.8.9.
- The launcher ships with its first-party public Microsoft application Client
  ID. It opens the official Microsoft account page in the system browser and
  accepts only a matching PKCE/state-protected localhost callback; refresh
  credentials remain in Opus's private local vault.
- An authenticated game session requires an Opus-vault-backed account, not a
  user-supplied Client ID or client secret.
- Settings cannot change the locked Minecraft version or escape OPUS's data
  directory.
