# Build, run, and test

## Toolchain

| Tool | Version | Source |
| --- | --- | --- |
| Rust | `1.92.0`, minimal + clippy + rustfmt | `rust-toolchain.toml` (automatic with rustup) |
| JDK | 25 for the build; the *game* targets bytecode 8 | CI uses Temurin 25 |
| Node | recent LTS | `desktop/package.json` |
| Gradle | via `./game/gradlew` | wrapper is committed |

Rust edition 2024. macOS is the runtime-verified platform; Windows is built and
tested in CI but GUI boot has not been proven on real hardware. macOS ARM64 also
needs **Rosetta**, because the game runs as x86_64.

The managed Java 8 runtime is downloaded by the launcher itself — do not install
a JRE for the game.

## The full local gate

```bash
./scripts/check.sh
```

Runs, in order:

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. `./game/gradlew -p game test prepareBootstrap --warning-mode all`

Optionally, with live network access to Mojang:

```bash
RBW_CHECK_OFFICIAL=1 ./scripts/check.sh
```

which adds the `#[ignore]`d contract test:

```bash
cargo test -p rbw-runtime --test official_metadata -- --ignored
```

CI (`.github/workflows/ci.yml`) runs the same steps on `macos-latest` and
`windows-latest`, and always runs the official-metadata test. Warnings are
errors on both the Rust and Java sides, so a warning fails CI.

## Running the desktop launcher

Development (Premium, hot reload):

```bash
npm --prefix desktop install     # once
npm --prefix desktop run tauri dev
```

`beforeDevCommand` runs `scripts/start-desktop-dev.sh`, which prepares the
Forge bridge/coremod artifacts and starts Vite on `127.0.0.1:1420`.

Premium release bundle:

```bash
npm --prefix desktop run tauri:build:premium
# → target/release/bundle/macos/RBW Client.app
```

QA offline demo (deliberately a debug bundle):

```bash
npm --prefix desktop run tauri:build:qa
# → target/debug/bundle/macos/RBW Client Demo.app

npm --prefix desktop run tauri:dev:qa     # QA in dev mode
```

Frontend-only checks:

```bash
npm --prefix desktop run check    # tsc --noEmit
npm --prefix desktop run build    # tsc -b && vite build
```

Any Tauri build or dev run prepares the bootstrap JARs for you via
`scripts/prepare-desktop-assets.sh`. Run it manually only if you are working
outside those entry points:

```bash
./scripts/prepare-desktop-assets.sh
```

## CLI diagnostics

```bash
./game/gradlew -p game test prepareBootstrap
cargo run -p rbw-launcher -- doctor
cargo run -p rbw-launcher -- install
cargo run -p rbw-launcher -- import-optifine /path/to/OptiFine_1.8.9_HD_U_M5.jar
cargo run -p rbw-launcher -- launch --offline --dry-run
```

`doctor` reports host and game architecture, whether translation is required and
available, the Mojang runtime key, the data directory, cached Forge-runtime
status, Forge bridge/coremod readiness, and whether a client ID is configured. It exits non-zero
when a required translation layer is missing.

`launch` flags: `--username`, `--offline`, `--max-memory-mib`, `--dry-run`,
`--bootstrap-dir`. `--direct` is not a supported CLI option. Two launch
boundaries to know about:

- **On macOS a real launch is blocked.** Only `--dry-run` works; use
  `RBW Client.app`, whose LaunchServices stub preserves the foreground game
  identity.
- A launch requires the locked Forge runtime, a verified local OptiFine import,
  and the bundled Forge bridge/coremod. There is no vanilla or legacy-bootstrap
  fallback when one is missing.

`account login` requires `RBW_MICROSOFT_CLIENT_ID` in the environment and uses
the device-code flow. The packaged launcher does not need this — it embeds its
own public client ID and uses the browser flow.

## Environment variables

| Variable | Effect |
| --- | --- |
| `RBW_HOME` | Overrides the Premium data root. Ignored by QA. |
| `RBW_QA_HOME` | Overrides the QA data root. Must be absolute, and must not overlap the Premium root. |
| `RBW_MICROSOFT_CLIENT_ID` | CLI-only OAuth client id. |
| `RBW_CHECK_OFFICIAL=1` | Adds the live metadata test to `check.sh`. |
| `RBW_GAME_WORKDIR` | Set by the launcher for the macOS stub. Not for manual use. |

For a disposable QA test:

```bash
RBW_QA_HOME="$HOME/tmp/rbw-qa-demo" npm --prefix desktop run tauri:dev:qa
```

## Test layout

| Where | What |
| --- | --- |
| `rbw-platform` unit tests | Java version/arch parsing, layout isolation, QA-root overlap rejection. |
| `rbw-runtime` unit tests | SHA-1/size verification, Forge lock/profile validation, local OptiFine import, managed-mod checks, `safe_join` traversal rejection, native extraction/collisions, install-state validation, progress accounting, private file modes, Log4j filter derivation, identity validation, stdin protocol encoding. |
| `rbw-runtime/tests/official_metadata.rs` | `#[ignore]`d; asserts the live Mojang 1.8.9 contract, LWJGL selection per platform, and pinned `jre-legacy` versions. |
| `rbw-auth` unit tests | Client-id and verification-URI validation, PKCE S256 vector, authorize-URL shape, callback state/host/duplicate rules, the "code is not echoed to the browser" test, cancellation and timeout bounds, error redaction. |
| `rbw-desktop` unit tests | Settings validation and legacy tolerance, utility-settings contract and ranges, login-slot ownership, install-event field set, QA profile rules, developer-profile isolation. |
| `game/*` JUnit 5 | Forge-bootstrap argument parsing, stdin protocol, launch status, coremod adapter/lifecycle, transformer chain, and per-transformer bytecode tests. |

Most security properties in [invariants.md](invariants.md) have a test behind
them. If you change behavior and a test like
`loopback_callback_returns_no_authorization_code_to_the_browser` or
`session_manifest_is_payload_free_and_describes_telemetry_policy` fails, the
test is right and the change needs rethinking.

## Debugging a failed launch

Start at the session directory printed with the game-finished event
(`<data root>/logs/<session-id>/`) and follow the triage order in
[diagnostics.md](diagnostics.md). For install problems, the typed
`InstallError` message usually names the exact stage; for launch refusals, look
up the `LaunchError` variant in `launcher/rbw-runtime/src/launch.rs`.

## Notes and gotchas

- **`.rbw-client/` in the repo is local runtime data.** It is gitignored; never
  commit it.
- Bundled Forge bridge/coremod JARs and the compiled macOS stub are gitignored
  build outputs — a fresh clone must run a prepare/build step before the app
  can launch a game. OptiFine is intentionally not a build output: import a
  locally obtained exact supported JAR into each isolated runtime.
- The macOS stub only rebuilds when `macos-game-stub.c` is newer than the
  installed executable. Touch the source if you need to force it.
- `rbwc-web/` has its own toolchain and its own git repository. Its checks
  (`npm run lint`, `npm test`) are unrelated to this gate.
- The repository currently has **no commits on `main`**; treat history-based
  assumptions with care.
