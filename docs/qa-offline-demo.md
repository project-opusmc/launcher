# QA offline-demo launcher

`Ranked Bedwars Client Demo` is an internal/demo flavor of the launcher for
showing the locked Forge + OptiFine Minecraft 1.8.9 runtime before the Premium
Microsoft integration is released. It is deliberately not an alternative
sign-in method for the Premium client.

## Build and local data

Build the QA app with:

```bash
npm --prefix desktop run tauri:build:qa
```

The command intentionally creates a debug bundle. Before bundling, it builds
the Forge bridge, coremod, and normal client-mod artifacts plus a small native macOS game app stub. The stub
is opened through LaunchServices and immediately `exec`s the managed Java 8
runtime, so Minecraft receives a foreground app identity without a window
manipulation hack.

QA has a distinct application identifier (`dev.rbw.client.qa`) and icon. Its
launcher settings and managed Minecraft cache are isolated from Premium data:

| Platform | Default QA data root |
| --- | --- |
| macOS/Linux | `~/.rbw-client-qa` |
| Windows | `AppData/Local/RBWClientQA` |

For a disposable local test, set `RBW_QA_HOME` to a dedicated absolute path
before starting the QA app. `RBW_HOME` is reserved for the Premium/CLI path and
does not redirect QA data.

## Session behavior

The QA UI saves one username that must meet Minecraft's offline-name rules:
3–16 ASCII letters, digits, or underscores. At launch the backend derives the
standard deterministic `OfflinePlayer:<username>` UUID and uses the offline
launch identity. It does not call Microsoft, request a Java Edition
entitlement, access the OS keychain, or retain an access/refresh token.

The first setup may download checksum-verified Mojang and locked Forge runtime
artifacts; subsequent launches can use a complete verified cache. OptiFine is
never downloaded by RBW: before every launch is allowed, import the exact local
`OptiFine_1.8.9_HD_U_M5.jar` obtained by the player. RBW verifies it and copies
it only into the isolated QA runtime; it never alters or removes the original
file. QA does not use the user's ordinary `.minecraft` directory.

## Forge mod boundary

QA is a controlled three-artifact Forge mod set: the checksum-verified RBW
coremod, checksum-verified normal RBW client mod, and exact locally imported
OptiFine JAR. Do not add user mods, modpacks, or ad-hoc test JARs to its managed `game/mods/` directory. An
unexpected visible top-level entry makes launch fail. Supporting another mod
requires an explicit pinned artifact, Forge ordering review, validation for all
Forge-discoverable directories, and a clean-root QA smoke test.

## Multiplayer boundary

An offline identity cannot authenticate to a server configured with
`online-mode=true`. Minecraft's normal server authentication rejects that
session; this is the enforcement boundary, not a launcher-maintained server
list. QA can only be used with servers deliberately configured as
`online-mode=false`.

An offline-mode server does **not** prove who a player is. Someone can select a
known username, including the name of a staff member. Keep QA demos limited to
servers operated by the team, use whitelisting or a suitable server-side
authentication policy, and do not grant offline names production privileges.

## Release boundary

Do not distribute `Ranked Bedwars Client Demo` as a Premium build, present it as a licensed
account launcher, or rely on it for public multiplayer access. The Premium
artifact remains the only flavor that performs Microsoft sign-in and Java
Edition ownership/session verification.
