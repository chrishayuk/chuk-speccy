# Roles, Sessions & Autonomy — Admin vs Agent

Companion to the [MCP spec](./02-mcp-server-layer-spec.md). It answers a forward
question: as the server grows, **agents should be pure consumers** of a machine
that is provisioned, recorded, and checkpointed *for* them — while **admins keep
full control**, and a lot of the housekeeping (recording, snapshotting, session
lifecycle) happens **automatically** rather than as agent tool calls.

Today every tool is flat in one module ([§5 of the MCP spec](./02-mcp-server-layer-spec.md#5-mcp-tool-catalog)).
That's right for v1 and wrong for this future. This doc draws the line.

---

## 1. Three planes, not two

```
  ┌─ agent plane (consumers) ──────────────────────────────┐
  │  perceive + act on THEIR session's machine             │
  │  screenshot · read_screen_text · get_registers ·       │
  │  read_memory · press_keys · type_text · run_frames …    │   policy-free
  └──────────────────────┬─────────────────────────────────┘
                         │  resolves "my machine" from session id
  ┌──────────────────────▼─────────────────────────────────┐
  │  autonomy plane (the server itself, no tool calls)      │
  │  • provision a machine per session, reap when idle      │
  │  • record every session → MP4 (always-on)               │
  │  • snapshot on a cadence → a rewindable timeline        │
  │  • enforce quotas / budgets                             │
  └──────────────────────┬─────────────────────────────────┘
                         │  operates the registry + supervisor
  ┌──────────────────────▼─────────────────────────────────┐
  │  admin plane (operators) — EVERYTHING                   │
  │  create/destroy · load game · write_memory · set_reg ·  │
  │  recording control · set_display · list ALL sessions ·  │
  │  download any artifact · policy config                  │
  └─────────────────────────────────────────────────────────┘
```

The organising rule: **an agent tool never carries policy.** It cannot create or
destroy a machine, cannot start/stop recording, cannot poke memory or registers,
cannot see another session. It observes and drives. Everything else is the
server's job (autonomy plane) or the operator's (admin plane).

---

## 2. Capability tiers

| Tier | Who | Tools | Nature |
|---|---|---|---|
| **Observe** | agent | `screenshot`, `read_screen_text`, `get_registers`, `read_memory`, `disassemble`, `trace` | read-only |
| **Drive** | agent | `press_keys`, `type_text`, `run_frames`, `run_until`, `step` | input + time, no state mutation |
| **Provision** | admin | `create_machine`, `destroy_machine`, `reset`, `load_snapshot`, `load_tape` | lifecycle |
| **Mutate** | admin | `write_memory`, `set_register`, `set_display` | arbitrary state / presentation |
| **Capture** | admin + autonomy | `start_recording`, `stop_recording`, `record_video`, `save_snapshot` | artifacts |
| **Operate** | admin | `list_machines` (all), download artifacts, policy config, per-session inspect | cross-session |

Notes:
- `get_registers`/`read_memory`/`disassemble`/`trace` are **agent** tools — the
  LLM-as-Z80-debugger framing ([MCP spec §1](./02-mcp-server-layer-spec.md#1-why-this-is-more-than-a-toy))
  needs them. Observation is safe; mutation is not.
- `save_snapshot` is **Capture**, not agent-facing: in the autonomy model the
  server checkpoints on a cadence, so the agent never asks. An admin can still
  snapshot on demand.
- `load_snapshot` (loading a game) is **Provision** = admin: the agent receives an
  already-running game, it doesn't choose one.

---

## 3. Sessions become implicit

Today the agent passes a `machine_id` to every tool. In the consumer model that
leaks lifecycle into the agent surface. Instead:

- **A machine is bound to the MCP session.** `chuk-mcp-runtime` already has a
  session notion (`current_session`, `create_session`). On first agent tool call
  in a session, the autonomy plane **provisions** a machine (default config, or a
  game the admin pre-loaded for that session) and binds it.
- **Agent tools take no `machine_id`.** `screenshot()` resolves "my machine" from
  the session. The agent literally cannot address another session's machine.
- **Admin tools still take an explicit `machine_id`** (or `session_id`) — admins
  operate across all sessions.
- **Reaping:** the supervisor destroys idle/disconnected sessions' machines (and
  finalises their recording) on a timeout.

This is the single biggest simplification for agents: *there is just "the
machine," and it's already set up.*

---

## 4. Autonomy plane (the supervisor)

A `Supervisor` wraps the registry and owns policy. Configured by the admin, it
runs without agent involvement:

- **Always-on recording.** Every session records from provisioning to reaping;
  the MP4 (video + beeper audio, [MCP recording](./02-mcp-server-layer-spec.md))
  is an artifact the admin downloads. The agent never calls `record_*`. Because
  capture is at the `run_frame` chokepoint in the core, this is free — it already
  catches every advance.
- **Snapshot timeline.** Auto-checkpoint every *N* frames (and/or on events:
  level change, score delta, score address write). Snapshots form a tree the
  admin can browse/rewind — the determinism dividend ([MCP spec §3](./02-mcp-server-layer-spec.md#3-execution-model--headless-stepped-deterministic))
  turned into an automatic feature rather than an agent chore. Feeds the RL
  branching story for free.
- **Budgets / quotas.** Per-session caps (frames/sec, total frames, memory,
  recording length); the supervisor throttles or reaps. Stops a runaway agent.
- **Lifecycle.** Provision-on-first-use, idle reap, graceful finalise (flush the
  recording, write the snapshot tree index).

The agent tools are thin shims that ask the supervisor for "my machine" and call
through; the supervisor is where recording/snapshot/quota hooks live.

---

## 5. Mapping onto chuk-mcp-server  *(implemented)*

Built on **`chuk-mcp-server`** (pydantic-native `@tool`, `get_session_id()`,
artifact VFS) as **two endpoints** over one shared `Supervisor`:

- **Two `ChukMCPServer` instances** — `build_agent()` (8 tools: Observe + Drive)
  and `build_admin()` (20 tools: everything). Two endpoints, *not* one role-gated
  server — a small agent tool list means little context, and it's a cleaner trust
  boundary. Run them as two processes, or co-host in one (`serve.py`) so they
  share the live machines.
- **Implicit session** via `get_session_id()` — agent tools take no
  `machine_id`; the supervisor binds a machine to the connection's session. Admin
  tools take an explicit `session_id` and reach every session.
- **One shared `Supervisor` singleton** holds the live machines (in-process
  Python objects) and the autonomy policy. Across *separate* processes the
  framework's multi-server session store + VFS share metadata and artifacts; live
  real-time control of another process's machine is the co-host case.
- **Artifacts → VFS.** Recordings are written to the artifact workspace when a
  store is configured (`write_workspace_file`), downloadable via the built-in file
  tools; otherwise a local file + inline base64 for small clips. No base64-stuffing
  of large MP4s through tool results.
- **Hints.** Tools carry `read_only_hint` / `destructive_hint` annotations, so even
  within the admin surface the dangerous ones are flagged.

---

## 6. Phased plan

1. **Tier the tools (cheap, non-breaking).** Split the current flat module into
   `agent_tools` + `admin_tools`, tag each with its tier, keep one config that
   loads both. No behaviour change; the line is now real and the surface is
   greppable.
2. **Introduce the `Supervisor`** over the registry. Agent tools resolve "my
   machine" from the session; `machine_id` becomes optional/admin-only.
3. **Always-on recording** in the supervisor (provision → record → finalise on
   reap). Remove `record_*` from the agent surface.
4. **Snapshot timeline** on a cadence + event hooks; admin browse/rewind.
5. **Budgets + idle reaping.**
6. **Role-gated transport** (HTTP/SSE + auth scopes) so one server serves both
   planes with the trust boundary enforced.

Phase 1 is safe to do now; 2–6 are sequenced and each is independently useful.

---

## 7. Decisions to lock before building

- **Implicit vs explicit session.** Recommend implicit for agents (machine bound
  to MCP session), explicit `machine_id` for admins. Confirm the runtime exposes a
  stable per-connection session id.
- **Always-record default.** On by default (cheap, the capture hook already
  exists), with an admin off-switch and a retention/size policy. Confirm storage
  location + retention.
- **Snapshot cadence.** Time-based (every N frames) vs event-based (memory-watch
  on a score/lives address) vs both. Event-based needs the admin to declare watch
  addresses per game.
- **Trust boundary.** Role-gated single server (needs runtime auth) vs separate
  deployments. Determines how hard the admin/agent split is enforced.
- **Artifact transport.** Inline base64 (small clips) vs artifact-server +
  download URL (everything). Recommend the latter as the default once §5 lands.
