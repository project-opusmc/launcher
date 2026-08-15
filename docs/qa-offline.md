# QA Offline Launcher

`Opus Launcher QA` is an internal, offline-only build used to verify the same
managed Forge + OptiFine runtime path as Premium without Microsoft credentials.
It is not an alternative authentication method and must not be published as a
production account launcher.

## Build And Isolation

```bash
OPUS_RUNTIME_ARTIFACT_DIR=/path/to/runtime/build/runtime \
  npm --prefix desktop run tauri:build:qa
```

The bundle identifier is `org.polydevs.opusmc.launcher.qa`. Its data is isolated
from Premium:

| Platform | Default QA root |
| --- | --- |
| macOS/Linux | `~/.opus-launcher-qa` |
| Windows | `%LOCALAPPDATA%/OpusLauncherQA` |

`OPUS_QA_HOME` may select a dedicated absolute root. QA ignores `OPUS_HOME` and
rejects any root that overlaps Premium storage.

## Identity

QA supports saved offline profiles using Minecraft-compatible names: 3-16
ASCII letters, digits, or underscores. Launcher derives the standard
`OfflinePlayer:<username>` UUID and never calls Microsoft, checks Java Edition
ownership, opens the keychain, or stores an access or refresh token for that
profile. Offline profiles appear in the account catalog as `[UNOFFICIAL]`.

## Runtime Boundary

QA uses the same checksum-verified OPUS Runtime artifacts as Premium. OptiFine
remains a local user import and must match the exact locked JAR. Each identity
gets its own game directory under `instances/<uuid>/game`.

The managed Forge mod directory accepts only:

- `opus-runtime-legacy-1.8.9.jar`
- `opus-client-legacy-1.8.9.jar`
- `OptiFine_1.8.9_HD_U_M5.jar`

An unexpected visible entry blocks launch. Supporting another artifact requires
an explicit immutable contract and Runtime/Launcher integration review.

## Multiplayer Boundary

Offline identities cannot authenticate to servers using `online-mode=true`.
An `online-mode=false` server also cannot prove ownership of a username, so QA
must be limited to operator-controlled test servers with appropriate access
controls. Offline names must never receive production privileges solely from
their displayed username.
