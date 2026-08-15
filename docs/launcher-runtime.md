# Launcher runtime: install, verify, launch

Everything in this page lives in `crates/engine`, with path and platform
support from `crates/platform`. The supported launch path is the pinned
Forge + locally imported OptiFine 1.8.9 profile; vanilla is only the verified
base dependency of that profile, not a selectable product runtime.

## Pinned constants

`crates/engine/src/lib.rs` holds the values that lock the runtime:

| Constant | Value / role |
| --- | --- |
| `MINECRAFT_VERSION` | `1.8.9` |
| `MINECRAFT_VERSION_JSON_URL` + `_SHA1` | Exact 1.8.9 metadata document and its digest |
| `VERSION_MANIFEST_URL` | Official `version_manifest_v2.json` |
| `JAVA_RUNTIME_INDEX_URL` + `_SHA1` + `_SIZE` | Pinned Mojang Java runtime index (13 385 bytes) |
| `ForgeRuntimeLock` | Checked-in Forge `1.8.9-11.15.1.2318-1.8.9` profile, resolved library contracts, and the OptiFine local-import contract |

Installation reads the public manifest first, finds the `1.8.9` entry, and
**compares it against the pinned URL and SHA-1**. A mismatch is
`InstallError::VersionContractChanged` — the installer stops rather than
following a changed pointer.

## Data layout

`OpusPaths::from_root` derives everything from one root:

```
<root>/
├── install-state-v1.json      versioned installation state
├── .install.lock              exclusive lock file (fs2)
├── cache/
├── minecraft/
│   ├── versions/1.8.9/{1.8.9.json, 1.8.9.jar}
│   ├── libraries/<maven path>/…       Mojang + pinned Forge libraries
│   └── assets/{indexes,objects,log_configs}/…
├── runtime/
│   ├── indexes/java-runtime-all-v1.json
│   ├── manifests/<platform>/<component>/<version>.json
│   └── java/<platform>/<component>/<version>/…
├── game/mods/
│   └── OptiFine_1.8.9_HD_U_M5.jar           verified local import cache
├── instances/<minecraft-uuid>/game/          identity-isolated game directory
│   └── mods/
│       ├── OptiFine_1.8.9_HD_U_M5.jar
│       ├── opus-runtime-legacy-1.8.9.jar
│       └── opus-client-legacy-1.8.9.jar
├── logs/<session-id>/         per-session diagnostics
└── sessions/<session-id>/     natives, classpath file, filtered log4j
```

Roots: `~/.opus-launcher` (macOS/Linux) or `%LOCALAPPDATA%/OpusLauncher`
(Windows) for Premium; `~/.opus-launcher-qa` /
`%LOCALAPPDATA%/OpusLauncherQA` for QA; and `~/.opus-launcher-ui-preview` /
`%LOCALAPPDATA%/OpusLauncherUiPreview` for the
temporary UI Preview. `OPUS_HOME` overrides Premium only; `OPUS_QA_HOME` and
`OPUS_UI_PREVIEW_HOME` override their respective isolated roots and must be
absolute.

Every path built from remote metadata goes through `safe_join`, which rejects
empty segments, `.`, `..`, absolute paths, leading separators, and Windows
drive prefixes (`C:`). Asset object hashes must additionally be 40 hex
characters. That is the defense against a hostile metadata document writing
outside the data root.

## Downloading and verification

`Downloader` (`src/download.rs`):

- HTTPS only — `validate_url` rejects any other scheme, including for JSON.
- 15 s connect timeout, 60 s request timeout, at most 5 redirects.
- User agent `Opus-Launcher/<crate version>`.
- If the spec declares a size and the response advertises `Content-Length`, a
  mismatch aborts before writing (`RemoteSizeMismatch`).
- The body is read through `.take(size + 1)`, or 512 MiB when no size is known;
  JSON reads are capped at 16 MiB.
- Bytes go to a sibling `.<name>.part-<pid>` file, are verified with streaming
  SHA-1 (64 KiB buffer), then atomically renamed. A failed verification deletes
  the partial file and returns `IntegrityMismatch`.
- `ensure` short-circuits to `DownloadOutcome::AlreadyPresent` when the
  destination already verifies, which is what makes offline launches possible.

`verify_file` is also the cache check: size first (cheap), then SHA-1.

## The installer

`Installer` (`src/install.rs`) exposes three entry points:

| Method | Behavior |
| --- | --- |
| `install()` / `install_with_progress(cb)` | Full install or repair, reaching the network as needed. |
| `load_cached()` | Verifies an existing installation with **zero** network access. Any missing or corrupt artifact is an error. |
| `prepare()` | Cache-first: try `load_cached`, fall back to `install`. On double failure returns `PrepareFailed` carrying both causes. |

All three run under an exclusive `fs2` lock on `<root>/.install.lock`, so two
launcher processes cannot install concurrently.

### Order of work

1. Fetch the version manifest and check the pinned Mojang base-client contract.
2. Download and verify `1.8.9.json`, then `validate_version`: id must be
   `1.8.9`, its base main class must be `net.minecraft.client.main.Main`, and
   the Java contract must be `jre-legacy` major 8.
3. Download the base client JAR, every rule-allowed base library artifact and
   native classifier, logging config (Log4j XML), and asset index.
4. Download the pinned Forge library set from `ForgeRuntimeLock`, verifying
   each resolved URL, size, and SHA-1. The lock then replaces the active
   runtime id, profile id, main class, and argument template with Forge's
   LaunchWrapper contract.
5. All asset objects are fetched in parallel via Rayon from
   `https://resources.download.minecraft.net/<first2>/<hash>`.
6. Java runtime: pinned index → platform/component entry (last one wins) →
   runtime manifest → all `raw` file downloads in parallel; `executable` entries
   get mode `0755` on Unix. Symlink entries are **unsupported and fail**
   (`UnsupportedRuntimeLink`).
7. Probe the installed Java: must be major version 8 and match the game
   architecture, else `IncompatibleManagedJava`.
8. Write `install-state-v1.json` atomically with the locked Forge runtime and
   profile IDs.

OptiFine is not a network-install step. `import_optifine` accepts only a local
file that matches the lock's exact size and SHA-1, copies it atomically into the
shared local import cache, and never changes or removes the source file. At
launch it is verified again and staged into the selected identity's isolated
`instances/<uuid>/game/mods/` directory. The QA policy is documented in
[qa-offline.md](qa-offline.md).

`java_home` is `<runtime root>/jre.bundle/Contents/Home` on macOS, the runtime
root itself elsewhere.

### Install state validation

`load_cached` does not trust its own state file. `validate_install_state`
requires: schema version 1, Minecraft `1.8.9`, the expected Forge runtime and
profile IDs, a runtime platform matching the current host, component
`jre-legacy`, a version name that survives `safe_join`, and a manifest URL that
is **HTTPS on `piston-meta.mojang.com`** with a 40-hex SHA-1 and a size in
`1..=16 MiB`. It then cross-checks the recorded Java selection against the
pinned index, so a tampered state file cannot redirect the runtime.

### Progress reporting

`InstallPhase` is a fixed set of user-facing labels (waiting for lock, Mojang
metadata/libraries/assets, Forge runtime, Java metadata/runtime, finalizing,
complete).
`ProgressReporter` serializes callbacks from Rayon workers behind a mutex and
emits on the first artifact, every fourth, and the last — enough to feel live
without flooding the webview. `completed_files` is stage-local; `downloaded` and
`cached` counters are cumulative. The callback may run on worker threads, so it
must be cheap and thread-safe.

## Library rules and natives

`library_is_allowed` walks the rule list and lets the **last matching rule
win**; an empty rule list means allowed. `native_classifier` looks up the OS
rule name (`osx`, `windows`, `linux`) and substitutes `${arch}` with the game
architecture's bit width. Both use `platform.game_arch`, never the host
architecture — this is why a macOS ARM64 host correctly selects x86_64 LWJGL 2
natives.

`extract_natives` unpacks the selected archives into a fresh per-session
`natives/` directory. It refuses a non-empty destination, skips directories and
`META-INF/`, honors each library's `extract.exclude` prefixes, rejects path
traversal, and treats a duplicate output path across two archives as an error
(`ArchiveCollision`). Zero extracted files is an error too.

## Launch planning

`LaunchPlan::build` (`src/launch.rs`) is the single place a runnable command is
assembled.

**Identity.** `GameIdentity::offline` validates the username (3–16 ASCII
alphanumerics or underscore) and derives the standard MD5-based
`OfflinePlayer:<name>` UUID with the version/variant bits set.
`GameIdentity::authenticated` requires a 32-hex UUID, a whitespace-free token of
8–4096 characters, and `user_type == "msa"`.

**Session setup.** A session id of `<unix-millis>-<pid>`; session, natives, and
log directories created with mode `0700`; `diagnostics.jsonl` and `gc.log`
pre-created at `0600` so a permissive umask cannot widen them; a payload-free
`session-manifest.json` written up front.

**JVM arguments.** macOS adds `-Xdock:name=Opus Client`. Then heap
bounds, three native library path properties (`java.library.path`,
`org.lwjgl.librarypath`, `net.java.games.input.librarypath`), the diagnostics
file property, GC logging with details and date stamps, and an
`-XX:ErrorFile=…/jvm_crash_%p.log`. Optional `-Dopus.utility.settings.file=` and
`-Dopus.brand.wordmark.file=` connect the launcher's settings and branding to the
in-game UI.

**Logging config.** `prepare_logging_config` reads Mojang's verified Log4j XML
and requires exactly one `<filters>` element **and** exactly one occurrence of
Mojang's `${...}` lookup-deny `RegexFilter`. If that anchor is missing the
launch fails closed (`UnexpectedLoggingConfiguration`) rather than running
without the Log4Shell mitigation. OPUS then injects a second filter that denies
`Session ID is token:` messages and writes the derived config into the session
directory.

**Game arguments.** The verified base arguments are rendered first, then the
locked Forge profile supplies the active `minecraftArguments` template. It is
split with `shlex`; every `${…}` placeholder must resolve, including Forge's
`--tweakClass net.minecraftforge.fml.common.launcher.FMLTweaker`, or launch
fails with `UnresolvedPlaceholder`.

**Supported launch mode: `ForgeBootstrap`.** The runtime id must equal the
locked Forge profile, the managed OptiFine JAR must verify, and both bundled OPUS
artifacts must be present. The launcher:

- canonicalizes the single `bootstrap-*.jar` and all managed Forge/Minecraft
  classpath entries;
- starts `org.polydevs.opusmc.bootstrap.ForgeBootstrapMain` rather than the vanilla main
  class;
- sends the ordinary game arguments through stdin; and
- verifies the OPUS Runtime coremod checksum and required `FMLCorePlugin`
  manifest, then stages it as `game/mods/opus-runtime-legacy-1.8.9.jar`;
- verifies and stages the typed Forge client mod into
  `game/mods/opus-client-legacy-1.8.9.jar`; and
- rejects an unexpected visible top-level entry in the managed `game/mods/`
  directory.

The typed Forge client mod is the product UI route; Launcher treats both OPUS
Runtime JARs as immutable artifacts and does not compile their source.

The product desktop and CLI paths do not select `Direct` or standalone
`Bootstrap` mode. Those internal variants exist only for test boundaries; they
are not a fallback if the Forge profile or OptiFine import is missing. The
direct branch also defensively refuses authenticated identities.

**Stdin protocol.** `encode_game_arguments` emits big-endian `u32` count, then
for each argument a big-endian `u32` byte length and its UTF-8 bytes. Limits:
256 arguments, 1 MiB each. This is why an access token never appears in `ps`.

**Native preflight.** `validate_native_architectures` confirms both an LWJGL and
a JInput native family were extracted, catching a rule/classifier mistake before
the game reaches its main loop.

## Starting and supervising the process

Two entry points:

`launch_game` — spawn the JVM directly with piped stdio. If a stdin payload
exists it is written and the pipe closed. Two threads tee stdout and stderr into
the session directory, replacing any known access token with `<redacted>` line
by line.

`launch_game_via_macos_app` — the macOS path. It validates that
`Opus Client.app/Contents/MacOS/Opus Client` exists, injects
`-Dopus.game.statusFile=…` *before* `-cp`, writes the stdin payload to a file,
and runs:

```
/usr/bin/open -n --stdin <payload> --stdout <raw> --stderr <raw>
              --env OPUS_GAME_WORKDIR=<game dir>
              "Opus Client.app" --args <java> <jvm args> <main> <game args>
```

LaunchServices cannot reliably wait for a stub that immediately replaces itself
with Java, so OPUS does not use `open -W`. It waits on the Runtime status file
instead. Because `open` does not stream, raw output files are redacted into the
final `game.stdout.log` / `game.stderr.log` afterwards and the raw files (and
the stdin payload) are deleted.

Since `open`'s exit status only reports LaunchServices, the real outcome comes
from the status file the Forge bootstrap writes:

| Status | Result |
| --- | --- |
| `exited`, `terminated` | success (`game-lifecycle-complete`) |
| `failed` | `MacosGameFailed` |
| `starting`, `running`, empty | `MacosGameDidNotExit` |
| anything else | `MacosGameStatusInvalid` |

Every path calls `finalize_session`, which writes `launcher-summary.json` and
best-effort snapshots the game's own `logs/latest.log` — but only if its mtime
is at or after the launch start, so a stale log from a previous session is never
attributed to this one. Diagnostics failures never turn a successful game into a
reported failure.

## Error catalogue

`InstallError`, `LaunchError`, `DownloadError`, `NativeError`, and
`PlatformError` are exhaustive `thiserror` enums at the bottom of their modules.
Read them before adding a new failure mode — most conditions already have a
typed variant, and their messages are deliberately free of tokens and URLs.

## When you change this crate

- Changing Forge? Update the checked-in runtime lock and test every new or
  changed resolved artifact. Do not execute an installer or follow mutable
  Forge profile metadata at runtime.
- Changing OptiFine support? Keep it a local, user-provided import. OPUS must
  not add a download, bundled copy, or redistribution path.
- Adding a managed mod? Extend the QA allowlist and directory validation for
  every Forge-discoverable location, lock its artifact and ordering, and add a
  clean-root smoke test.
- Adding another network fetch? It must go through `Downloader` with a
  `DownloadSpec`. There is no unverified download path, and adding one breaks
  [invariants.md](invariants.md).
- Adding a path derived from metadata? It must go through `safe_join`.
- Changing progress reporting? Remember the callback runs on Rayon threads.
- Changing launch arguments? Anything secret belongs in the stdin payload, never
  in `jvm_arguments` or `game_arguments`.
