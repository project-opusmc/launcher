# Invariants

Properties the codebase enforces deliberately. Most have a test behind them. If
your change breaks one, the change is wrong — not the invariant. When an
invariant genuinely needs to change, record the reasoning in a new ADR.

## Integrity

1. **No unverified bytes reach the game.** Every managed artifact — version
   JSON, client JAR, Mojang and Forge libraries, natives, assets, Java runtime
   files, Log4j config, and the OPUS Core Mod — must match its declared size
   and SHA-1 before use. All managed downloads go through `Downloader::ensure`;
   there is no bypass path.
2. **Downloads are atomic.** Bytes land in a sibling `.part-<pid>` file, are
   verified, then renamed. A failed verification deletes the partial file.
3. **HTTPS only.** `validate_url` rejects every other scheme, including for JSON
   metadata.
4. **The Forge + OptiFine 1.8.9 contract is pinned.** The base version JSON URL
   and SHA-1, Java runtime index URL/SHA-1/size, and checked-in Forge profile
   lock are immutable inputs. A changed official pointer is
   `VersionContractChanged`, not a silent follow; a changed Forge library or
   profile is rejected rather than adopted from mutable metadata.
5. **Metadata cannot escape the data root.** Every metadata-derived path goes
   through `safe_join`, which rejects `..`, absolute paths, leading separators,
   and Windows drive prefixes. Asset hashes must be 40 hex characters.
6. **Cached install state is re-validated, not trusted.** Schema version,
   Minecraft version, locked Forge runtime/profile IDs, host platform,
   `jre-legacy` component, and an HTTPS `piston-meta.mojang.com` manifest URL
   with a plausible size — then cross-checked against the pinned index.
7. **Rule evaluation uses the game architecture.** Never the host architecture.
   That is what makes macOS ARM64 select x86_64 natives correctly.
8. **Native extraction is strict.** No traversal, no cross-archive collisions,
   no empty result, and a preflight requires both LWJGL and JInput families.
9. **Launch never installs.** The desktop path uses `load_cached`; a corrupt
   installation surfaces as Install / Repair rather than an implicit download.
10. **OptiFine is local-import only.** OPUS accepts only the exact locked local
   JAR after size/SHA-1 verification, copies it into its isolated runtime, and
   never downloads, bundles, redistributes, modifies, uploads, or deletes the
   user's original file.

## Credentials

11. **No token in the webview.** The frontend receives non-secret account
    summaries and a redacted `"Name (uuid)"` sign-in result. Never a token,
    refresh credential, authorization code, password, or client secret.
12. **No token on a command line.** Authenticated game arguments cross into the
    Forge bridge through the length-prefixed stdin protocol. `ps` shows no token.
13. **Only the refresh token is persisted, and only in Opus's local vault.** It
    is keyed by Minecraft profile UUID under the platform data directory (or
    `OPUS_AUTH_DIR` for an isolated test), with a `0700` directory and `0600`
    token files on Unix. The launcher never calls the OS keychain. The unified
    Tauri window is `incognito`, so WebKit cannot create a persistent
    website-data store and reintroduce a macOS Keychain prompt.
14. **The authorization code is never echoed to the browser.** The three
    callback pages are static, `no-store`, `default-src 'none'`.
15. **Callback validation is strict.** Loopback peers only, `GET` on exactly
    `/`, single `Host` matching the bound port, matching `state`, no duplicated
    `code`/`state`/`error`, 8 KiB header cap, 4096-char code cap.
16. **The listener binds before the browser opens.** No callback can arrive
    before OPUS is listening.
17. **Cancellation cannot leak a credential.** Checked before accept, after
    read, before exchange, and before persisting.
18. **Identity-service responses are never surfaced.** Only a `code`/`error`
    field is extracted; bodies are never interpolated into an error. The single
    exception is the exact `Invalid app registration` mapping.
19. **Every supported product launch uses the Forge stdin bridge.** The desktop
    and CLI compile only the Forge bootstrap mode; no direct or standalone
    bootstrap launch variant remains selectable.
20. **One login at a time.** The `LoginAttempt` guard is owned by the blocking
    task, so a dropped IPC future cannot free the slot mid-flow.

## Logging and privacy

21. **Mojang's Log4j `${...}` deny filter must be present.** Exactly one
    `<filters>` element and exactly one matching `RegexFilter`, or the launch
    fails closed. OPUS adds a second filter denying `Session ID is token:`.
22. **Captured output is redacted.** Known access tokens are replaced with
    `<redacted>` line by line in `game.stdout.log` / `game.stderr.log`.
23. **Telemetry is local and payload-free.** Timing, counters, lifecycle, and
    JVM environment only — no username, chat, server address, packet payload, or
    credential. Nothing is uploaded automatically.
24. **The session manifest carries no payload.** It is an index for the session,
    not a replay.
25. **Progress events carry five fields.** `phase`, `completedFiles`,
    `totalFiles`, `downloadedFiles`, `cachedFiles` — no URL, path, or artifact
    name.
26. **Private files stay private.** Session, natives, and log directories are
    created `0700`; `diagnostics.jsonl` and `gc.log` are pre-created `0600` so a
    permissive umask cannot widen them.

## Build separation

27. **There is one live launcher root.** `OPUS_HOME` selects the unified
    storage root. The former QA root is migration input only and is never used
    to launch, stage, or write a managed runtime.
28. **One IPC surface owns both identity kinds.** Microsoft and offline
    profiles live in one account catalog. Offline profiles never access the
    credential vault; official profiles must have a file-owned refresh token.
29. **Non-default developer features cannot ship in release.** `compile_error!`
    blocks `developer-test-profile` in release builds; the developer profile is
    additionally `debug_assertions`-gated at every use site. The quarantined
    legacy-preview runner is disabled.
30. **The developer test profile cannot touch the real system.**
    `require_runtime_mode()` blocks install, launch, login, and logout, and it
    cannot be disabled mid-session.
31. **Offline launch remains explicit.** A selected `[UNOFFICIAL]` identity
    creates only a deterministic offline Minecraft session; it does not relax
    the official-account refresh or entitlement path.

## Game side

32. **Forge owns the game classloader.** The sole product launch enters
    LaunchWrapper and its `LaunchClassLoader`; OPUS transforms game classes only
    through the verified Forge `IClassTransformer` adapter. The standalone
    loader is excluded from production artifacts and cannot be selected by the
    launcher.
33. **The OPUS Forge artifacts are exact and instance-owned.** Before staging,
    the Core Mod checksum, size, and `FMLCorePlugin` manifest attributes are
    verified. Every launch atomically replaces same-named stale files in the
    selected identity directory, and Forge discovers only locked OptiFine and
    the Core Mod there, never from the shared game directory or an arbitrary
    launcher path.
34. **Transformers fail closed.** A patch asserts its anchor and throws when the
    count is unexpected — zero or multiple matches fail the class load rather
    than applying an uncertain patch.
35. **The Forge patch chain is deterministic and unique.** Sorted by priority
    then id; duplicate ids, empty ids, null entries, and a `null` return are all
    errors.
36. **Lifecycle transitions are compare-and-set.** An unexpected transition
    throws instead of continuing in an ambiguous state.
37. **Forge bootstrap arguments are exact.** The stdin-protocol marker is
    required; unknown or duplicated control arguments and trailing payload bytes
    throw. Forge's `FMLTweaker` is guaranteed before `Launch.main`.
38. **Vanilla owns UI and input.** The Core Mod artifact is rejected if it
    contains a GUI hook, key binding/input class, CEF/webview symbol, or the
    retired standalone loader.
39. **Java 8 bytecode, no warnings.** `options.release = 8` with `-Werror` for
    every module.

## macOS specifics

40. **The game launches through the compiled app stub.** A raw child `bin/java`
    is `BackgroundOnly` — it can render and play audio with no operable window.
    The stub owns the LaunchServices foreground identity, forks the managed
    Java child, activates its real window, and preserves the stdin protocol.
    It must reap the child with `waitpid(..., WNOHANG)` while waiting so native
    JVM crashes cannot become a false 60-second startup hang.
41. **No `-XstartOnFirstThread` for 1.8.9.** It is a later-metadata trait and is
    not valid on this launch path. No Carbon process transform or JNI is used;
    AppKit/CoreGraphics are limited to activating the stub-owned game window.
42. **Rosetta is required and checked.** Verified with
    `/usr/bin/arch -x86_64 /usr/bin/true` before launch, with a clear failure
    message.
43. **The macOS outcome comes from the status file, not `open`'s exit code.**
    The foreground app stub supervises Java, so OPUS must not use `open -W`:
    LaunchServices can falsely report that its tracked process disappeared.
    OPUS starts the app with `open -n` and waits for the bootstrap
    status file to report `running`, then a terminal lifecycle state. The
    stub's `game.window.failure` marker and JVM crash markers are authoritative
    early-failure signals and are checked before the startup timeout.
44. **A CLI launch on macOS fails closed** and directs the caller to the
    packaged launcher.

## Product boundaries

45. **Minecraft is locked to 1.8.9** and settings cannot change it or escape the
    data root.
46. **The game directory is isolated from `.minecraft`.**
47. **Memory bounds are validated twice** — on save and again before launch
    (512–16384 MiB).
48. **Managed Forge mod sets are allowlisted.** They consist only of the exact
    imported OptiFine JAR and verified OPUS Core Mod. The known stale OPUS client
    JAR is removed before validation; every other unexpected top-level
    managed-mod entry fails launch. A future mod requires a lock, validation
    coverage for every Forge-discoverable directory, ordering review, and
    tests before it is supported.
49. **Offline mode is not an access-control system.** An unofficial profile can
    pick any username; those sessions require operator-controlled servers with a
    whitelist.
50. **The embedded Client ID must be Mojang-approved before authenticated
    launch works.** An application-owner release task; never borrow another
    launcher's identity.
51. **Unknown or unfinished product behavior is never mocked.** If OPUS lacks
    verified reference data or a working runtime implementation, that surface,
    control, widget, value, and state stays absent. Never ship placeholder
    numbers, sample HUD data, fake enabled states, no-op buttons, speculative
    navigation, or a visual-only settings control. A visible interactive control
    must operate on real validated state and persist according to its contract.
52. **Instance ownership is per identity.** Different account IDs may launch
    concurrently in UUID-isolated game directories. A second launch for the
    same account is rejected until its active attempt finishes.
