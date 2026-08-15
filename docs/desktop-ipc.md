# Desktop application and IPC contract

The launcher is a Tauri 2 app: a React/TypeScript webview
(`desktop/src/App.tsx`) talking to a Rust backend
(`desktop/src-tauri/src/lib.rs`) over Tauri commands and events.

## Feature gating

Three mutually shaped builds, decided at compile time:

| Build | Features | Notes |
| --- | --- | --- |
| Premium | *(none)* | Full account flow. |
| QA offline | `qa-edition` | No account code compiled. |
| Developer test | `developer-test-profile` (debug only) | In-memory simulation on top of Premium. |

`compile_error!` rejects either non-default feature in a release build, and the
developer profile is additionally guarded by `debug_assertions` at every use
site. `run()` contains three separate `generate_handler!` blocks, so the set of
registered commands differs per build — a command that does not exist cannot be
invoked from the webview.

## Commands

Registered in every build:

| Command | Signature | Notes |
| --- | --- | --- |
| `launcher_snapshot` | `() -> LauncherSnapshot` | Runs blocking work off the UI thread. |
| `install_minecraft` | `() -> InstallResult` | Emits `rbw://install-progress` while running. |
| `import_optifine` | `(sourcePath) -> OptiFineImportResult` | Verifies and copies the exact user-provided local OptiFine JAR; it never downloads or removes the source file. |
| `get_settings` / `save_settings` | `LauncherSettings` | Atomic write. |
| `get_utility_settings` / `save_utility_settings` | `UtilitySettings` | Atomic write; validated on read too. |
| `launch_game` | `(settings) -> GameLaunchStarted` | Returns as soon as the process starts; completion arrives as an event. |

Premium only: `login_with_microsoft`, `cancel_microsoft_login`, `logout`.
QA only: `get_qa_profile`, `save_qa_profile`.
Developer test only: `set_developer_test_profile`, `simulate_developer_game`.

## Events

| Event | Payload | When |
| --- | --- | --- |
| `rbw://install-progress` | `InstallProgressEvent` | Throttled during install/repair. |
| `rbw://game-finished` | `GameLaunchFinished` | The game process ended, or the launch failed. |

## Payload shapes

Rust serializes with `rename_all = "camelCase"`; the TypeScript types in
`App.tsx` mirror these one-for-one.

```ts
type LauncherSnapshot = {
  buildEdition: "premium" | "qaOffline";
  platform: string;             // "macos x86_64" or a simulation label
  dataDirectory: string;
  minecraftReady: boolean;
  minecraftStatus: string;      // Forge runtime status, human-readable
  optifineReady: boolean;
  optifineStatus: string;       // import/verification status, never a source path
  javaStatus: string;
  accountStored: boolean;       // never a token
  offlineProfile: { username: string; valid: boolean } | null;  // QA only
  gameLaunchReady: boolean;     // macOS: game stub present
  developerTestProfile: { available: boolean; active: boolean; simulationActive: boolean };
};

type InstallProgressEvent = {
  phase: string;                // InstallPhase::label(), e.g. "Installing Forge 1.8.9 runtime"
  completedFiles: number; totalFiles: number;
  downloadedFiles: number; cachedFiles: number;
};

type InstallResult = {
  minecraftVersion: string;
  javaVersion: string;
  optifineReady: boolean;
  downloadedFiles: number; cachedFiles: number;
};

type OptiFineImportResult = { fileName: string };

type GameLaunchStarted  = { sessionId: string; logDirectory: string | null; simulated: boolean };
type GameLaunchFinished = { sessionId: string; logDirectory: string | null;
                            outcome: "exited" | "failed"; message: string; simulated: boolean };
type AccountResult      = { profile: string };   // "Name (uuid)" — redacted
```

`InstallProgressEvent` carries exactly five fields: no URL, no path, no artifact
name. There is a test asserting the serialized object has five keys and no
`url` / `path` / `token`. Keep it that way when adding progress detail.

`import_optifine` is intentionally separate from `install_minecraft`: the
installer may fetch only locked Mojang/Forge artifacts, while OptiFine must be
selected locally by the player. The UI must not present an OptiFine download
button or retain the source path after the import completes.

Errors are returned as `Result<T, String>` — plain human-readable strings built
by `display_error`. They surface directly in the UI, so they must never contain
a token or an unexpected internal path.

## Settings

Two independent files under the data root, both written atomically through
`write_json_atomic` (write `.part-<pid>`, fsync, rename; the partial file is
removed on failure):

**`launcher-settings-v1.json`** — `maxMemoryMib` (512–16384, validated on save
*and* before launch) and `closeLauncherOnGameStart`. Deserialization tolerates
unknown legacy keys: an old file containing `microsoftClientId` still loads, and
the field is never written back. There are tests for both directions.

**`utility-settings-v1.json`** — a `BTreeMap<String, UtilityPreference>` under
`utilities`. A map, not a struct, so new client utilities do not change the
top-level JSON contract. Each preference has `enabled`, `anchor`
(`top-left` | `top-right` | `bottom-left` | `bottom-right`), `offset` (free text,
≤64 chars), `scale` (50–150), `opacity` (25–100). Validation runs on save and
again on load; an invalid file is reported as invalid rather than silently
repaired.

Six utilities ship in the v1 defaults: `performance-overlay`, `status-timers`,
`armor-status`, `crosshair`, `world-time`, `chat-readability`.

> The Rust defaults and the TypeScript `defaultUtilitySettings` in `App.tsx` are
> maintained separately and currently differ in a few anchor/offset/opacity
> values. The persisted file is authoritative once written; the Rust defaults
> apply when no file exists. If you change one side, check the other.

The file path is passed to the game as `-Drbw.utility.settings.file=`, which is
how the in-game HUD reads the same preferences. See
[game-bootstrap.md](game-bootstrap.md).

## Launch orchestration

`launch_game` does the preparation on a blocking task
(`prepare_game_launch`) and then hands the plan to a plain
`std::thread::spawn` that waits for the game and emits `rbw://game-finished`.
The command itself returns immediately with the session id.

`prepare_game_launch`:

1. Validates settings.
2. Requires `translation_available()` — on an ARM Mac without Rosetta it fails
   with a clear message before touching anything else.
3. `installer.load_cached()` — **launch never installs**. A missing or corrupt
   installation directs the user to Install / Repair.
4. Builds the identity: Premium refreshes through the keychain and re-saves the
   rotated token; QA loads and re-validates the offline profile.
5. Resolves the bundled `bootstrap-*.jar` and `rbw-forge-coremod-*.jar`, and on
   macOS the game app.
6. Builds a `LaunchMode::ForgeBootstrap` plan with the utility settings file
   and the brand wordmark attached. This requires the locked Forge runtime and
   verified locally imported OptiFine JAR; legacy standalone bootstrap modes
   are not selected.

If `closeLauncherOnGameStart` is set, the main window is hidden (not closed)
after the process starts.

## Resource resolution

`launcher_resource_path` first tries Tauri's bundled resource directory, then
falls back to a `CARGO_MANIFEST_DIR`-relative development path. Three resources
are resolved this way: `bootstrap/` (the bridge and verified Forge coremod
JARs), `brand/rbw-wordmark-transparent.png`, and `Ranked Bedwars Client.app`.
The macOS game app is additionally checked for its inner executable before
being accepted.

## The macOS game stub

`desktop/src-tauri/build.rs` compiles `resources/macos-game-stub.c` with
`xcrun clang` as a universal `arm64 + x86_64` binary
(`-Werror`, `-mmacosx-version-min=11.0`) into
`resources/game-app/Contents/MacOS/Ranked Bedwars Client`. It stages the output
under `OUT_DIR` and only replaces the watched resource when the C source is
newer, because compiling directly into a Tauri-watched directory triggers a
rebuild loop.

The stub itself is about 35 lines: skip LaunchServices `-psn_*` arguments,
`chdir` to `$RBW_GAME_WORKDIR`, `execv` the Java executable. No AWT, no Carbon
process transform, no JNI, no `-XstartOnFirstThread`. `execv` preserves both the
stdin protocol and the foreground app identity.

## Developer test profile

`developer_test.rs` is a mutex-guarded, in-memory coordinator. When active,
`snapshot_for_profile` short-circuits before any real service call — there is a
test asserting the real snapshot closure is never invoked — and
`require_runtime_mode()` blocks install, launch, login, and logout. The
simulation issues ids like `developer-test-0000`, refuses to start twice,
ignores a stale finish, and cannot be disabled mid-session. Simulated events
carry `simulated: true` and `logDirectory: null`.

Note the asymmetry: QA bypasses the *developer simulation* guard because it is a
real runtime path. It never bypasses an authentication guard — it has no
authentication code to bypass.

## Security configuration

Premium CSP (`tauri.conf.json`):

```
default-src 'self';
connect-src 'self' ipc: http://ipc.localhost ws://localhost:1420;
img-src 'self' asset: http://asset.localhost;
style-src 'self' 'unsafe-inline';
script-src 'self'
```

Capabilities are `core:default` for the `main` window only — no filesystem,
shell, or HTTP plugin is exposed to the webview.

## Frontend notes

`App.tsx` is a single component tree with Home, Utilities, Account, and
Settings pages. It loads the snapshot, settings, utility settings, and — in QA
— the offline profile on mount, subscribes to both events, and debounces
utility-setting saves. Settings exposes the local OptiFine import state; it
does not expose a Forge/modpack picker or an arbitrary-mod installation flow.
It never stores credentials; account state is the single `accountStored`
boolean.

## When you add a command

1. Decide which builds should register it, and add it to the right
   `generate_handler!` block(s) — remember there are three.
2. Do blocking work in `spawn_blocking`, not on the IPC thread.
3. Return only aggregate, non-secret data; add a serialization test if the
   payload could plausibly grow a sensitive field.
4. Mirror the type in `App.tsx`.
5. If it can start a process or an installer, gate it behind
   `state.require_runtime_mode()?`.
