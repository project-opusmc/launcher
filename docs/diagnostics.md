# Per-session diagnostics

Each managed game launch has a local session directory. The launcher returns
that directory with the game-finished event, and it normally lives under
`<OPUS data root>/logs/<session-id>/`.

The default data root is `~/.opus-launcher` on macOS and Linux for Premium, and
`~/.opus-launcher-qa` for the offline QA build. On Windows the corresponding
roots are under `%LOCALAPPDATA%/OpusLauncher` and
`%LOCALAPPDATA%/OpusLauncherQA`.
`OPUS_HOME` overrides the Premium root; `OPUS_QA_HOME` overrides only the QA
root.

## Files in a session

| File | Purpose | When it may be absent |
| --- | --- | --- |
| `session-manifest.json` | Redacted launch metadata and the session's diagnostic policy. Treat it as an index for the session, not as a replay of gameplay. | The launcher failed before it created the session manifest. |
| `diagnostics.jsonl` | Local, newline-delimited telemetry. Each line is an independently parseable JSON event with a schema version and a process-relative `t_ms`. | The JVM did not reach the telemetry bootstrap, or stopped before it flushed output. |
| `launcher.log` / `launcher-summary.json` | Redacted launch record and final launcher outcome. | The launcher failed before it could create the relevant file. |
| `game.stdout.log` / `game.stderr.log` | The launcher-managed JVM, Forge bridge, Forge/OptiFine, and OPUS coremod output. Start here for a failed startup, transformer error, or Java exception. | A failure happened before the process streams were attached. |
| `minecraft.latest.log` | Private snapshot of Minecraft's own `game/logs/latest.log` at the end of this launch. It is the most complete game-side text log for the session. | The game did not create `latest.log`, or the launch stopped before it could be captured. |
| `gc.log` | The JVM's garbage-collection trace for this session. Use it to correlate a hitch with a collection pause; it is not a frame-time log. | The JVM stopped before GC logging initialized. |
| `jvm_crash_<pid>.log` | A HotSpot crash report, written only for a JVM-level fatal error. | Most application failures are Java exceptions, not JVM crashes. |

Do not assume a missing file means a clean launch. It usually only identifies
the stage that did not get far enough to create that artifact.

## Privacy boundary

The telemetry writer is local-only and is deliberately limited to timing,
counter, lifecycle, and JVM-environment fields. It does not record usernames,
chat, server addresses, packet payloads, or authentication material. Final
launcher logs redact known access tokens before they are retained.

The text logs (`game.stdout.log`, `game.stderr.log`, and especially
`minecraft.latest.log`) are intentionally more complete and can include chat,
server endpoints, local filesystem paths, Java/library versions, or exception
text. Review them before sharing. If an interrupted launch leaves temporary raw
stream or stdin artifacts, do not share them: a launcher input artifact can
contain a session argument payload.

These are local files. OPUS does not automatically upload the diagnostics
described here; sharing any selection of files is a deliberate user action.
This document does not promise a retention, deletion, or rotation policy.

## Reading `diagnostics.jsonl`

The writer emits lifecycle and startup events, then approximately five-second
`performance_window` records and a terminal `session_summary`. Timing values
are milliseconds and have this shape:

```json
{
  "event": "performance_window",
  "frame_ms": { "samples": 300, "p50": 4.100, "p95": 9.400, "p99": 18.200, "max": 31.700 },
  "tick_ms": { "samples": 100, "p50": 0.800, "p95": 2.600, "p99": 5.900, "max": 8.100 },
  "render_ms": { "samples": 300, "p50": 2.900, "p95": 7.100, "p99": 15.600, "max": 27.300 }
}
```

The fields measure the instrumented client scopes, not a universal latency
number:

- `frame_ms` is the Minecraft client frame-loop duration.
- `tick_ms` is the client tick duration.
- `render_ms` is the verified renderer invocation within that frame loop.

`p50` is the median sample. `p95` means 95% of samples in that window were at
or below the value; `p99` exposes the rarer long tail. `max` is the single
largest retained sample. Always check `samples` before comparing percentiles.
Small windows and zero-sample windows are not useful for a conclusion.

A stable `p50` with rising `p95`/`p99` usually describes intermittent hitches,
not uniformly slower play. A high render tail is a useful renderer-side lead,
but it is not proof of GPU-bound work: these are client-method durations, not
GPU presentation timestamps. Likewise, high tick time is a client tick signal,
not a server-tick measurement. Correlate timing windows with `gc.collections`,
`gc.collection_time_ms`, the `memory` fields, `gc.log`, and the launcher stderr
before assigning a cause.

The network section is payload-free: it contains inbound/outbound packet
counts and sampled inbound inter-arrival timing. It is not an RTT measurement
and cannot identify a server-side delay. `dropped_events` means the bounded
local telemetry queue was full; treat that window as incomplete.

## Combat and hit-registration signals

The client can record an `attack_input`, the point where Minecraft queues an
attack packet, and a later entity-status packet that falls within a short
correlation window. The latter is explicitly labelled
`unverified_client_signal`.

Those observations do **not** prove that a server accepted a hit, that the
status packet belongs to that attack, or that hit registration was correct.
They are useful only as a client-side timing clue. Reliable hit-registration
analysis requires server-side instrumentation that records the server's combat
decision and correlates it with the relevant action/connection. A client log
alone cannot establish that result.

## Practical triage order

1. Read `session-manifest.json` and `launcher-summary.json` to confirm the session.
2. Read `game.stderr.log`, then `game.stdout.log`, for Forge/OptiFine startup,
   coremod, or transformer failures. Confirm the `[OPUS/FORGE]` bridge/coremod
   markers before diagnosing a Minecraft-side patch.
3. Inspect `diagnostics.jsonl` for the last lifecycle event and the nearby
   `performance_window` records.
4. Compare a timing spike with `gc.log`; inspect `jvm_crash_<pid>.log` only when a
   JVM crash report exists.
5. Share only the minimum reviewed files needed to reproduce or diagnose the
   issue.
