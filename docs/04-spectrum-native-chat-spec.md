# Spectrum-Native Chatbot / Agent — L3 Showpiece Spec

Companion to the [core](./01-core-emulator-spec.md) / [MCP](./02-mcp-server-layer-spec.md)
/ [SDK](./03-sdk-spec.md) specs. This is the flagship demo of the whole stack: a
chatbot-and-agent you drive from rubber keys, where the Z80 is a dumb 32-column
terminal and all intelligence lives host-side in your CHUK stack (`chuk-llm` +
`chuk-tool-processor` + your MCP servers). No transformer runs on the Z80; the
host handler behind a trap is free to be anything you can reach, including your own
no-GPU inference.

The whole design rests on **two decoupled clocks** and a **typed-event poll**:

- generation runs at host speed, pushing events into a per-session queue;
- the Z80 drains that queue at a fixed *teletype* rate, so bursty token delivery
  becomes calm green text crawling up the screen at 50 Hz.

Get those two right and it feels magical; get them wrong and the machine just
freezes for five seconds and vomits a paragraph.

---

## 1. Architecture

```
  ┌── emulator thread (Rust, ~50 Hz) ─────────────────────────┐
  │  Z80 terminal program                                     │
  │   input line ──ED70 CHAT_BEGIN──▶ host                     │
  │   render loop ──ED70 CHAT_POLL──▶ drains queue ──▶ screen  │
  └───────────────────────────┬───────────────────────────────┘
                              │  thread-safe per-session queue
  ┌───────────────────────────▼───────────────────────────────┐
  │  host async runtime (Python / CHUK)                        │
  │   chat_begin → schedule run_chat()                         │
  │   run_chat: chuk-tool-processor ⟷ chuk-llm (streaming)     │
  │            └ tool calls ──▶ MCP servers (maritime/her/…)   │
  │   pushes typed events: TOKEN / TOOL_* / DONE / ERROR       │
  └────────────────────────────────────────────────────────────┘
```

The **queue is the contract.** The Z80's poll rate and the model's token rate
never touch each other. Producer is async Python (`chuk-llm`); consumer is the
synchronous trap dispatch in the emulator. In the PyO3 build the queue is a
thread-safe ring the Rust side drains and Python fills via a small `push_event`
method — so the hot poll path never crosses the GIL, only `chat_begin` does.

Conversation history lives **host-side, keyed by session id.** The Z80 ships only
the new line — it has neither the RAM nor the need for context.

---

## 2. The chat trap ABI (the centerpiece)

Reserved syscalls in the chat block (`ED 70 <id>`, ids `0x30–0x3F`). Calling
convention from the [SDK spec §4](./03-sdk-spec.md#4-the-trap-abi-the-distinctive-layer):
small args in registers, results in `A`/`HL`, error in carry.

### `CHAT_BEGIN` (0x30) — fire and return
```
  in:  HL = ptr to prompt bytes (in Spectrum RAM)
       BC = prompt length
        A = session id
  out: CF=0 started, CF=1 busy/failed
```
Host reads the bytes out of emulated memory, appends to that session's history,
schedules `run_chat()` on the async loop, returns immediately. Non-blocking — the
Z80 never stalls here.

### `CHAT_POLL` (0x31) — drain, non-blocking
```
  in:  HL = dest buffer ptr,  B = buffer capacity (bytes),  A = session id
  out:  A = event type,  BC = bytes written,  CF=1 on hard error
```
Pops at most one event from the queue. Returns `IDLE` when nothing's ready — the
Z80 keeps polling and keeps its spinner spinning.

### Event types
| Value | Event | Payload | Z80 renders as |
|---|---|---|---|
| `0x00` | `IDLE` | — | (nothing; keep spinner alive) |
| `0x01` | `TOKEN` | text bytes | white ink, append to transcript |
| `0x02` | `TOOL_START` | tool name | bright cyan `> name…` |
| `0x03` | `TOOL_END` | result summary | bright cyan `  ↳ summary` |
| `0x04` | `DONE` | — | stop spinner, show input cursor |
| `0x05` | `ERROR` | message | red `! message` |
| `0x06` | `STATUS` | short note | dim/yellow (e.g. "thinking") |

**Streaming & chunking.** `TOKEN` payloads can exceed the Z80's poll buffer.
Rule: text events drain across as many polls as needed — the host keeps a cursor
into the current event and returns up to `B` bytes per call, same `TOKEN` type
until exhausted. The Z80 doesn't track boundaries; it just prints whatever bytes
arrive with `TOKEN`. Structured events (`TOOL_*`, names < 32 chars) always fit in
one poll. So poll is a byte-stream drain with event framing on top — trivial on
the Z80 side.

`CHAT_CANCEL` (0x32, `A`=session) aborts the async task; `CHAT_RESET` (0x33)
clears history.

---

## 3. Host handler

```python
SYSTEM = ("You are running on a 1982 ZX Spectrum: 32 columns, plain ASCII only. "
          "Be terse. No markdown, no tables, no em-dashes, no curly quotes, "
          "no emoji. Short lines.")

async def run_chat(session_id, prompt, q):
    hist = sessions[session_id]
    hist.append({"role": "user", "content": prompt})
    try:
        async for ev in agent.stream(hist, system=SYSTEM):   # chuk-tool-processor
            if   ev.type == "token":       q.push(TOKEN,      speccy(ev.text))
            elif ev.type == "tool_call":   q.push(TOOL_START, ev.name.encode())
            elif ev.type == "tool_result": q.push(TOOL_END,   speccy(ev.summary))
        hist.append({"role": "assistant", "content": agent.last_text})
        q.push(DONE, b"")
    except Exception as e:
        q.push(ERROR, speccy(str(e)))
```

Three host-side jobs worth calling out:

- **`speccy()` sanitisation is mandatory, not optional.** LLMs emit em-dashes,
  curly quotes, ellipsis chars, emoji — none of which exist in the Spectrum
  character set. Downgrade to ASCII (`—`→`-`, `"…"`→`"..."`), map `£`/`©` to the
  Speccy's own codes, replace anything left with `?`. Skipping this is the most
  likely thing to make the screen look broken.
- **The tool loop is real, via `chuk-tool-processor`.** Its circuit breaker /
  idempotency wrap each MCP call; `TOOL_START`/`TOOL_END` events are emitted
  around them. So you literally watch the agent reach out — `> maritime_search…`
  then `↳ 833,421 records` — in cyan, live.
- **Scheduling across threads.** Emulator runs on its own thread; schedule
  `run_chat` with `run_coroutine_threadsafe(...)` onto the asyncio loop. The queue
  handles the handoff back.

---

## 4. Z80-side terminal loop

Two responsibilities, kept on separate cadences:

```
each frame (50 Hz):
    poll keyboard → edit input line (echo, DELETE, ENTER, ESC)
    if generating:
        ev, bytes = CHAT_POLL(buf, 64, session)
        case ev:
          TOKEN      -> push bytes onto PRINT_FIFO  (do NOT print directly)
          TOOL_START -> queue a cyan "> {name}…" line
          TOOL_END   -> queue a cyan "  ↳ {summary}" line
          DONE/ERROR -> stop spinner; ERROR queues a red line
        advance spinner UDG frame
    drain_print_fifo(rate = 1..2 chars/frame, beep per char)   # the teletype
```

The crucial separation: **`CHAT_POLL` fills a `PRINT_FIFO`; a separate drain step
empties it at a fixed pretty rate.** One poll can dump 200 bytes in a single frame,
but they surface at reading speed with a beeper blip per character. That decoupling
is the entire reason this looks good. The real stream is faster than is fun to
watch — slow it *on purpose*.

Other Z80-side pieces (all L1 framework primitives from the SDK spec):
- **Custom print routine**, not `RST 10h` — you need per-cell attribute control
  for the colour-by-event scheme and you want silent scrolling (no ROM "scroll?"
  prompt). Writes ink/paper per cell as it goes.
- **Colour by event = clash-free TUI.** Each event gets its own 8-pixel row, so
  white assistant text, cyan tool lines and red errors never share an attribute
  cell. The Spectrum's per-cell colour, the usual curse, becomes the palette.
- **UDG spinner / blinking avatar** while generating (a few of the 21 user-defined
  graphics), driven off `STATUS`/`IDLE`.
- **Input line:** accumulate until ENTER → `CHAT_BEGIN`; ESC → `CHAT_CANCEL`;
  DELETE edits.

### Screen layout (32×24)
```
 row 0      ZX-CHAT            [⠿]  s:01      header + spinner + session
 rows 1–21  transcript (scrolls)                white/cyan/red by event
 row 22     ────────────────────────────────   rule
 row 23     > user input_                       input line, cursor blink
```

---

## 5. Why the constraint *helps*

The 32-column grid isn't only an aesthetic — it's a leash on the model. The system
prompt ("you're on a 1982 Spectrum, 32 cols, be terse, ASCII only") produces
tight, punchy replies instead of the usual three-paragraph essay. The hardware
disciplines the model into a voice that actually fits the screen. Keep that prompt;
it's doing real work.

---

## 6. Corollary — the agent can drive *it*

Because `CHAT_BEGIN`/`CHAT_POLL` are plain traps, the headless MCP mode
([MCP spec §3](./02-mcp-server-layer-spec.md#3-execution-model--headless-stepped-deterministic))
can drive this terminal too: an outer agent types into the Spectrum chatbot via
`type_text` + `press_keys`, watches replies via `read_screen_text`, screenshots the
crawl. An agent operating an agent through a 1982 terminal — pure stack-flex, near
zero extra code.

---

## 7. Build order & reuse

| Step | Build | Reuse |
|---|---|---|
| 1 | `CHAT_BEGIN`/`CHAT_POLL` trap ids + host queue | trap ABI (SDK §4) |
| 2 | `run_chat` wiring | `chuk-llm`, `chuk-tool-processor` |
| 3 | `speccy()` sanitiser + system prompt | — |
| 4 | Z80 terminal: input + custom print + colour | L1 framework |
| 5 | `PRINT_FIFO` teletype drain + beeper | L1 sound |
| 6 | UDG spinner, header, scroll | L1 |
| 7 | tool-event rendering (cyan) | MCP servers |

Prereqs: core through **M6** (beeper) and the **trap ABI** from the SDK spec.
Everything else here is reuse. Nothing in the path needs to be cloud — emulator,
inference, agent loop and tools can all be yours, wearing a 1982 face.
