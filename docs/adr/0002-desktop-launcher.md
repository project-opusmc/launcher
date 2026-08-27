# ADR 0002: Native desktop launcher and direct macOS JVM launch

Status: accepted

## Context

The initial Rust CLI can resolve and boot Minecraft 1.8.9. On macOS, Minecraft
1.8.9 uses the legacy x86_64 Java 8 runtime and LWJGL 2. A raw child
`bin/java` process is registered as `BackgroundOnly` on the supported macOS
environment: it can render and play audio while owning no visible game window.
LWJGL 2 performs Minecraft window operations through AppKit, which requires the
managed JVM to reserve its first thread for Cocoa. A production client also
needs a clear home for account, settings, installation state and launch
progress.

## Decision

- Opus Launcher is a native desktop application built with Tauri 2, React and
  TypeScript. The UI is a launcher, not an overlay inside Minecraft.
- Existing Rust crates remain authoritative for platform detection, verified
  installation, authentication, token handling and game launch planning.
- The desktop frontend uses narrowly scoped Tauri commands and status events;
  browser code never receives refresh tokens or Minecraft access tokens.
- Non-secret settings live in OPUS's isolated data directory. Microsoft refresh
  credentials remain solely in Opus's private local credential vault; the
  launcher does not invoke the macOS Keychain.
- The launcher ships with OPUS's first-party public Microsoft application Client
  ID. Choosing **Sign in with Microsoft** opens Microsoft's official
  account page in the system browser. Authorization Code + PKCE and a
  state-validated localhost callback return the result automatically; no
  password, authorization code, token or client secret enters the frontend.
- Mojang's manual AppID review and allow-list approval is an external release
  prerequisite for the embedded public Client ID. It is owned by the OPUS
  publisher, never delegated to end users, and cannot be replaced with another
  launcher's identity.
- On macOS, OPUS opens a small native universal `Minecraft.app` through
  LaunchServices. The stub sets the game working directory, starts the managed
  x86_64 Java 8 child, activates its real game window, and forwards termination
  signals while preserving the Forge bootstrap stdin protocol. It uses only
  AppKit/CoreGraphics window activation; no Carbon process transform or JNI is
  used. The managed JVM launches with `-XstartOnFirstThread` so LWJGL's window
  operations stay on AppKit's main thread. The stub uses `posix_spawn`, rather
  than `fork()`, so Java does not inherit partially initialized AppKit state.
- On Windows and Linux, the desktop app uses the same launch-plan protocol
  without macOS-specific JVM options.

## Consequences

- The launcher and game are separate processes, like established Minecraft
  clients. Closing or minimizing the launcher does not imply closing the game.
- The Launch button remains disabled until the verified Forge runtime, imported
  OptiFine artifact, macOS game stub, and (for Premium) a stored account are
  present. The stubbed launch is verified to own an on-screen game window rather
  than silently rendering in the background.
- Packaging, code signing and notarization become explicit release work rather
  than implicit side effects of a command-line tool.
