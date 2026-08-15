# Authentication

All Microsoft/Minecraft identity code lives in `crates/auth/src/lib.rs`.
The desktop backend calls it from `desktop/src-tauri/src/lib.rs`; the CLI calls
it from `crates/cli/src/main.rs`. The QA build does not register Microsoft
authentication commands and offline profiles do not access the keychain.

## Client ID

OPUS ships a first-party **public** OAuth client ID, embedded as
`OPUS_MICROSOFT_CLIENT_ID` in `desktop/src-tauri/src/lib.rs`. Public desktop
clients have no secret, so distributing the identifier is expected. The CLI
instead reads `OPUS_MICROSOFT_CLIENT_ID` from the environment
(`MICROSOFT_CLIENT_ID_ENV`) — that path exists for non-desktop development only.

`validate_client_id` accepts at most 128 characters of ASCII alphanumerics and
hyphens.

> **Release prerequisite.** Minecraft Services separately reviews and
> allow-lists Java game-service integrations. Until Mojang approves this exact
> AppID through its review form, `login_with_xbox` returns HTTP 403 with
> `Invalid app registration`, which OPUS maps to the typed
> `AuthError::MinecraftAppRegistrationRejected`. No user action or code change
> can bypass it. Never substitute another launcher's client ID.

## Browser flow (Authorization Code + PKCE)

This is the desktop path. `begin_browser_authorization`:

1. Binds a loopback listener on `127.0.0.1:0`, then best-effort binds `[::1]` on
   the same port, because `localhost` may resolve to IPv6 first. Both are
   non-blocking. **The listener is bound before the browser opens**, so no
   callback can race it.
2. Generates a 32-byte `state` and a 64-byte PKCE `code_verifier`
   (`getrandom`, URL-safe base64 without padding) and derives an S256
   `code_challenge`.
3. Builds the authorize URL against
   `login.microsoftonline.com/consumers/oauth2/v2.0/authorize` with
   `response_type=code`, `response_mode=query`, `scope=XboxLive.signin
   offline_access`, `prompt=select_account`, and
   `code_challenge_method=S256`.

The caller opens that URL in the system browser (`open::that`) and then calls
`complete_browser_authorization`.

### Callback handling

The loopback server is intentionally strict. `read_callback_target` accepts only
a `GET` with `HTTP/1.0` or `HTTP/1.1`, a target starting with a single `/`, at
most 8 KiB of request headers, exactly one `Host` header, and that host must
equal `localhost:<bound port>`. Non-loopback peers are rejected outright.

`parse_browser_callback` then requires the path to be exactly `/`, rejects any
duplicated `code` / `state` / `error` parameter, requires `state` to match, and
caps the code at 4096 characters. `error` without `code` is a decline; anything
else unexpected is ignored so the loop keeps waiting.

The three response pages are static HTML with `Cache-Control: no-store`, a
`default-src 'none'` CSP, `no-referrer`, and `nosniff`. **The authorization code
is never echoed back to the browser** — there is a regression test for exactly
that.

Waiting is bounded: a 5-minute overall deadline, 50 ms polling, 2 s per-socket
read timeout. `BrowserLoginCancellation` (an `Arc<AtomicBool>`) is checked
before accepting, after reading, before the token exchange, and again before the
credential is persisted, so a cancelled flow can never store a token.

## Device code flow

`begin_device_authorization` / `complete_device_authorization` implement the
CLI's flow. The returned `verification_uri` is validated by
`validate_verification_uri`: HTTPS, host exactly `microsoft.com` or a
`.microsoft.com` subdomain, and no embedded userinfo — so a compromised response
cannot send the user to `microsoft.com.evil.test`. Polling honors
`authorization_pending`, backs off 5 s on `slow_down`, and maps
`authorization_declined` / `expired_token` to typed errors.

## Token chain

Both flows converge on `finish_login`:

```
Microsoft access token
  → POST user.auth.xboxlive.com/user/authenticate      (RPS ticket "d=<token>")
  → POST xsts.auth.xboxlive.com/xsts/authorize         (RelyingParty rp://api.minecraftservices.com/)
  → POST api.minecraftservices.com/authentication/login_with_xbox
        identityToken = "XBL3.0 x=<userHash>;<xstsToken>"
  → GET  api.minecraftservices.com/entitlements/mcstore     (must be non-empty)
  → GET  api.minecraftservices.com/minecraft/profile        (32-hex id, non-empty name)
```

The Xbox user hash from the first call must equal the one returned by XSTS, or
the exchange is rejected. An empty entitlement list is
`MinecraftOwnershipRequired`. The result is a `MinecraftSession` with
`user_type = "msa"` plus a refresh token.

`refresh_session` loads the stored refresh token, exchanges it, and adopts a
rotated refresh token when Microsoft returns one — otherwise it keeps the
existing one. Callers must re-save after a refresh; both the desktop backend and
the CLI do.

## Credential storage

`RefreshTokenStore` wraps one OS keychain entry per Microsoft/Minecraft profile:

- service `org.polydevs.opusmc.launcher.microsoft.refresh-token`, with one
  account entry per Minecraft profile UUID
- `save` refuses an empty token
- `load` maps `NoEntry` to `Ok(None)`
- `delete` returns whether an entry existed

**Only the refresh token is persisted.** The Minecraft access token exists in
memory for the duration of a launch, is passed to the game through the stdin
protocol, and is registered as a redaction string so it is stripped from
launcher-captured output.

## Error hygiene

Service errors are deliberately lossy. `service_rejected` extracts only a
`code`/`error` field and otherwise reports `request_rejected`; response bodies
are never interpolated into an error message. The one exception is the
documented `Invalid app registration` message, matched exactly (403 only) to
produce `MinecraftAppRegistrationRejected` — and its user-facing text contains
neither the URL nor the request id. `MinecraftSession::redacted_summary` returns
`username (uuid)` and never the token.

## What the frontend sees

Never a token, authorization code, refresh credential, password, or client
secret. `login_with_microsoft` returns a redacted profile plus a non-secret
account summary. `launcher_snapshot` returns the complete non-secret account
catalog, selected account ID, and active account IDs. See
[desktop-ipc.md](desktop-ipc.md).

## Concurrency

`AppState::begin_login` allows exactly one active login. The `LoginAttempt`
guard is moved *into* the blocking task, so a dropped IPC future (webview
reload, for example) does not free the slot while the OAuth flow is still
running; `Drop` clears it only if it still owns the current attempt.
`cancel_microsoft_login` signals the cancellation without freeing the slot —
the task frees it when it actually stops.

## When you change this crate

- Do not log, serialize, or return response bodies from identity services.
- Do not relax callback validation (path, state, host, single-parameter rules);
  each check has a test.
- Do not add a second credential store or a file-backed token cache.
- New failure modes get a typed `AuthError` variant with a message that carries
  no secret material.
