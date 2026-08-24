# Offline Profiles And Legacy QA Migration

There is one production `Opus Launcher.app`. Its account catalog supports both
official Microsoft identities and explicitly labeled `[UNOFFICIAL]` offline
profiles; they use one managed Runtime and one data root.

The former `Opus Launcher QA` package is retired. On first use, a valid username
from its historic `offline-profile-v1.json` is imported into the unified
catalog. The old file remains untouched as a fallback and is never used as a
live runtime or credential store.

## Identity

Offline profiles use Minecraft-compatible names: 3-16 ASCII letters, digits,
or underscores. Launcher derives the standard `OfflinePlayer:<username>` UUID
and never calls Microsoft, checks Java Edition ownership, opens the credential
vault, or stores an access or refresh token for that profile.

## Runtime Boundary

Offline profiles use the same checksum-verified OPUS Runtime as official
profiles. OptiFine remains a local user import and must match the exact locked
JAR. Each identity gets its own game directory under `instances/<uuid>/game`.

The managed Forge mod directory accepts only:

- `opus-runtime-legacy-1.8.9.jar`
- `OptiFine_1.8.9_HD_U_M5.jar`

An unexpected visible entry blocks launch. Supporting another artifact requires
an explicit immutable contract and Runtime/Launcher integration review.

## Multiplayer Boundary

Offline identities cannot authenticate to servers using `online-mode=true`.
An `online-mode=false` server also cannot prove ownership of a username, so
offline play must be limited to operator-controlled test servers with
appropriate access controls. Offline names must never receive production
privileges solely from their displayed username.
