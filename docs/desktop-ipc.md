# Desktop Application And IPC Contract

Opus Launcher is a Tauri 2 application. The React/TypeScript TUI in `desktop/src`
communicates with the Rust backend in `desktop/src-tauri/src` through explicitly
registered commands and two application events.

## Build Flavors

| Build | Feature | Behavior |
| --- | --- | --- |
| Premium | none | Microsoft accounts and unofficial offline profiles |
| QA | `qa-edition` | offline-only, isolated debug bundle |
| UI Preview | `ui-preview` | QA behavior with a third isolated data root |
| Developer test | `developer-test-profile` | debug-only in-memory simulation |

QA, UI Preview, and developer-test features are rejected in release builds.
Each flavor has its own `generate_handler!` list, so unavailable commands are
not registered at runtime.

## Commands

Registered in every real-runtime build:

| Command | Purpose |
| --- | --- |
| `launcher_snapshot` | runtime, account, active-instance, and platform status |
| `install_minecraft` | verified install or repair with progress events |
| `import_optifine` | verify and import the exact local OptiFine JAR |
| `get_settings` / `save_settings` | launcher settings |
| `get_utility_settings` / `save_utility_settings` | in-game utility preferences |
| `list_accounts` | complete account/profile catalog |
| `select_account` | set the selected identity |
| `save_offline_profile` | add or update an `[UNOFFICIAL]` profile |
| `remove_account` | remove one catalog entry and its credential when applicable |
| `launch_game` | launch the selected identity in its isolated instance |

Premium also registers `login_with_microsoft` and
`cancel_microsoft_login`. QA registers `get_qa_profile` and `save_qa_profile`.
The developer-test build adds its simulation commands.

## Events

| Event | Payload | Meaning |
| --- | --- | --- |
| `opus://install-progress` | aggregate file counters and phase | install/repair progress |
| `opus://game-finished` | session, account, outcome, message | one game instance ended or failed |

Install events contain exactly five aggregate fields and no URL, path, artifact
name, username, or token.

## Snapshot And Account Shapes

`LauncherSnapshot` includes:

- build edition, platform, and data directory;
- Minecraft, OptiFine, Java, and macOS game-app readiness;
- the full `accounts` array and `selectedAccountId`;
- `activeLaunches` and `activeAccountIds` for concurrent instances;
- a safe developer-test status object.

Each account summary contains `id`, `username`, optional UUID, `kind`, `badge`,
`ready`, and `selected`. Microsoft account IDs are
`microsoft:<normalized-uuid>`; offline IDs are `offline:<derived-uuid>`.
Badges are normalized to `official`, `premium`, or `unofficial`.

The frontend never receives a refresh token, Minecraft access token, Microsoft
authorization code, password, or client secret.

## Settings

The data root contains three independent versioned files:

| File | Content |
| --- | --- |
| `launcher-settings-v1.json` | memory and close-on-launch preference |
| `utility-settings-v1.json` | enabled state, anchor, offset, scale, and opacity for each utility |
| `accounts-v1.json` | non-secret account/profile metadata and selected ID |

Writes use a same-directory partial file, `fsync`, and atomic rename. Account
credentials are not stored in `accounts-v1.json`; each Microsoft refresh token
is stored in the operating-system keychain under its Minecraft profile UUID.

## Multi-Instance Launch

`launch_game` requires an explicit account ID. The coordinator reserves that
identity before token refresh or artifact staging, rejects a duplicate launch
for the same account, and permits different identities to run concurrently.

Each identity uses:

```text
<data-root>/instances/<minecraft-uuid>/game/
```

The shared verified Minecraft libraries, assets, and Java runtime remain under
the common data root. Each game session gets an isolated game directory and
session log directory.

The launch command returns immediately after process startup. A background
thread waits for completion, releases the account slot, and emits
`opus://game-finished`.

## Runtime Resources

Tauri packages:

- `bootstrap/` with the Runtime manifest, checksums, and three OPUS Runtime JARs;
- `brand/opus-wordmark-transparent.png`;
- `Opus Client.app`, the macOS LaunchServices stub.

`scripts/stage-runtime-artifacts.mjs` validates the Runtime manifest and SHA-256
contract before resources are staged. `desktop/src-tauri/build.rs` compiles the
macOS stub from `resources/macos-game-stub.c`.

## Capability And CSP

The `main` window receives only `core:default` plus
`core:window:allow-close`. The explicit close permission is required for the
Tauri window close command; without it Tauri reports
`Command plugin:window|close not allowed by ACL`.

No filesystem, shell, or HTTP plugin is exposed to the frontend. Premium uses
a restrictive CSP allowing local Tauri IPC, local development WebSocket access,
packaged assets, inline styles, and self-hosted scripts.

## Security Rules For New IPC

1. Register the command only in the build flavors that need it.
2. Move blocking filesystem, network, and process work off the IPC thread.
3. Return aggregate, non-secret values and add a serialization test when the
   payload could grow a sensitive field.
4. Keep account credentials in Rust and the OS keychain.
5. Add the narrowest Tauri capability only when a frontend API requires it.
