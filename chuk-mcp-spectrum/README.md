# chuk-mcp-spectrum

A headless **ZX Spectrum** emulator exposed over MCP as **two endpoints** — built
on `chuk-mcp-server` (pydantic-native `@tool`, built-in session ids + artifact
VFS). See `../docs/02-mcp-server-layer-spec.md` (tools) and
`../docs/06-roles-autonomy-spec.md` (roles & autonomy).

## Two planes, one shared brain

| Endpoint | Surface | Who |
|---|---|---|
| **agent** (`chuk-mcp-spectrum-agent`) | 8 tools — `screenshot`, `read_screen_text`, `get_registers`, `read_memory`, `press_keys`, `type_text`, `run_frames`, `run_until` | LLMs / consumers. Tiny tool list = little context. Operates the machine bound to *this* session (implicit via the framework's session id). **Policy-free**: no lifecycle, no pokes, no recording knobs. |
| **admin** (`chuk-mcp-spectrum-admin`) | 20 tools — everything: operate any session, `write_memory`/`set_register`, `load_snapshot`/`load_tape`, recording control, snapshot timeline (`list_snapshots`/`restore_snapshot`), `list_sessions`, downloads | Operators. |

Both share one in-process **`Supervisor`** (`supervisor.py`) — the *autonomy
plane* that runs policy **without tool calls**:

- **Record every session by default** (configurable) → MP4 (H.264 + AAC, with
  beeper sound), stored in the artifact VFS when available / a local file
  otherwise, downloadable.
- **Snapshot on a cadence** (frame-based now — that *is* time at 50 fps; wall-clock
  for real-time later) → a rewindable timeline the admin can `restore_snapshot`.
- **Provision per session, reap when idle.**

The agent just plays; the server records, checkpoints, and manages lifecycle.

## Run

```bash
uv venv .venv && source .venv/bin/activate
( cd ../zxspec_py && maturin develop --release )      # build the Rust binding
uv pip install -e .

export SPECTRUM_ROM=/path/to/48.rom
# optional: SPECTRUM_DEFAULT_GAME=/path/to/game.z80  (agents join it already running)

chuk-mcp-spectrum-agent      # http://127.0.0.1:8011/mcp   (or SPECTRUM_STDIO=1)
chuk-mcp-spectrum-admin      # http://127.0.0.1:8012/mcp
# or co-host both in one process (shared live machines):
python -m chuk_mcp_spectrum.serve
```

### Policy knobs (env)

`SPECTRUM_RECORD` (default on) · `SPECTRUM_SNAPSHOT_FRAMES` (0 = off) ·
`SPECTRUM_VIDEO_DECIMATE` (2) · `SPECTRUM_AUDIO_RATE` (44100) ·
`SPECTRUM_IDLE_REAP_SECONDS` (1800) · `SPECTRUM_DEFAULT_GAME` ·
`SPECTRUM_RECORDINGS_DIR`.

## Test

```bash
SPECTRUM_ROM=/path/to/48.rom pytest -q   # surfaces split, autonomy, record-with-audio, rewind
```
