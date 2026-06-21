"""Two MCP endpoints over one shared `Supervisor` (`docs/06-roles-autonomy-spec.md`):

* **agent** — a tiny consumer surface: observe + drive the machine bound to *this*
  session (implicit via `get_session_id`). No lifecycle, no recording knobs, no
  pokes. Small tool list = small context.
* **admin** — everything: operate any session, poke memory/registers, manage
  recordings and the snapshot timeline, download artifacts.

Built on `chuk-mcp-server` (pydantic-native `@tool`, built-in VFS + session id).
Recordings land in the artifact VFS when available (downloadable), else a local
file. The autonomy plane (recording, snapshot cadence, reaping) lives in the
`Supervisor`; tools never carry that policy.
"""

from __future__ import annotations

import base64
import os

from chuk_mcp_server import ChukMCPServer, get_session_id
from chuk_mcp_server.types import ImageContent
from PIL import Image
import io

from chuk_mcp_spectrum.supervisor import SUPERVISOR

# ── helpers (shared by both planes; take an explicit session id) ──────────────


def _sid() -> str:
    """The agent's implicit session — bound to the MCP connection."""
    return get_session_id() or "default"


def _png(rgba: bytes, scale: int = 2) -> ImageContent:
    img = Image.frombytes("RGBA", (256, 192), rgba)
    if scale > 1:
        img = img.resize((256 * scale, 192 * scale), Image.NEAREST)
    buf = io.BytesIO()
    img.save(buf, "PNG")
    return ImageContent(type="image", data=base64.b64encode(buf.getvalue()).decode(), mimeType="image/png")


def _screenshot(sid: str, scale: int = 2) -> ImageContent:
    return _png(bytes(SUPERVISOR.machine(sid).screen_rgba()), scale)


def _registers(sid: str) -> dict:
    return dict(SUPERVISOR.machine(sid).registers())


def _read_memory(sid: str, addr: int, length: int) -> dict:
    data = bytes(SUPERVISOR.machine(sid).read_memory(addr, length))
    return {"addr": addr, "length": len(data), "hex": data.hex()}


def _screen_text(sid: str) -> str:
    return SUPERVISOR.machine(sid).screen_text()


def _disassemble(sid: str, addr: int, count: int) -> dict:
    lines = SUPERVISOR.machine(sid).disassemble(addr, count)
    return {
        "lines": [{"addr": ln["addr"], "bytes": bytes(ln["bytes"]).hex(), "text": ln["text"]} for ln in lines]
    }


def _press(sid: str, keys: list[str], frames: int) -> dict:
    SUPERVISOR.machine(sid).press(keys, frames)
    SUPERVISOR.note_activity(sid)
    return {"ok": True, "keys": keys}


def _type(sid: str, text: str) -> dict:
    typed = SUPERVISOR.machine(sid).type_text(text)
    SUPERVISOR.note_activity(sid)
    return {"ok": True, "typed": typed}


def _run_frames(sid: str, n: int) -> dict:
    s = SUPERVISOR.advance_frames(sid, n)
    return {"ok": True, "frames": n, "pc": s.machine.registers()["pc"], "total_frames": s.frames_run}


def _run_until(sid: str, pc: int | None, max_steps: int) -> dict:
    res = dict(SUPERVISOR.machine(sid).run_until(pc, max_steps))
    SUPERVISOR.note_activity(sid)
    return res


def _search_games(query: str, limit: int) -> dict:
    import zxspec_py

    return {"results": [dict(r) for r in zxspec_py.search_games(query, limit)]}


def _load_game(sid: str, query: str) -> dict:
    """Find a game on World of Spectrum, download it, and load it into a session."""
    import zxspec_py

    g = zxspec_py.fetch_game(query)
    SUPERVISOR.load_game(sid, g["format"], bytes(g["data"]))
    return {
        "ok": True,
        "title": g["title"],
        "year": g["year"],
        "format": g["format"],
        "source": g["source"],
    }


def _store_recording(session_id: str, info: dict) -> dict:
    """Put the encoded MP4 into the artifact VFS if available (downloadable),
    always returning at least the local path; inline base64 for small files."""
    path = info["path"]
    with open(path, "rb") as fh:
        data = fh.read()
    try:
        from chuk_mcp_server import has_artifact_store, write_workspace_file, create_workspace_namespace

        if has_artifact_store():
            ns = create_workspace_namespace(session_id=session_id)
            ns_id = getattr(ns, "id", None) or getattr(ns, "namespace_id", None)
            name = os.path.basename(path)
            write_workspace_file(ns_id, name, data)
            info["vfs_namespace"] = ns_id
            info["vfs_path"] = name
    except Exception as e:  # VFS is a best-effort enhancement
        info["vfs_error"] = str(e)
    if len(data) <= 12 * 1024 * 1024:
        info["data_b64"] = base64.b64encode(data).decode()
    return info


# ── agent plane (observe + drive THIS session) ───────────────────────────────


def register_agent_tools(mcp: ChukMCPServer) -> None:
    @mcp.tool(read_only_hint=True, description="See the current Spectrum screen as a PNG.")
    def screenshot(scale: int = 2) -> ImageContent:
        return _screenshot(_sid(), scale)

    @mcp.tool(read_only_hint=True, description="Read the 32x24 text screen (menus, BASIC, prompts).")
    def read_screen_text() -> dict:
        return {"text": _screen_text(_sid())}

    @mcp.tool(read_only_hint=True, description="Z80 register/flag state.")
    def get_registers() -> dict:
        return _registers(_sid())

    @mcp.tool(read_only_hint=True, description="Read `length` bytes from `addr` (hex).")
    def read_memory(addr: int, length: int = 16) -> dict:
        return _read_memory(_sid(), addr, length)

    @mcp.tool(read_only_hint=True, description="Disassemble `count` Z80 instructions from `addr`.")
    def disassemble(addr: int, count: int = 16) -> dict:
        return _disassemble(_sid(), addr, count)

    @mcp.tool(description="Hold keys (chars, or 'enter'/'space'/'caps'/'sym') for `frames`, then release.")
    def press_keys(keys: list[str], frames: int = 2) -> dict:
        return _press(_sid(), keys, frames)

    @mcp.tool(description="Type a string through the keyboard (BASIC keyword rules apply).")
    def type_text(text: str) -> dict:
        return _type(_sid(), text)

    @mcp.tool(description="Advance N full frames (~50/s). The normal way to let the machine run.")
    def run_frames(n: int = 1) -> dict:
        return _run_frames(_sid(), n)

    @mcp.tool(description="Run until PC reaches `pc` or `max_steps` elapse.")
    def run_until(pc: int | None = None, max_steps: int = 2_000_000) -> dict:
        return _run_until(_sid(), pc, max_steps)


# ── admin plane (everything; explicit session_id) ────────────────────────────


def register_admin_tools(mcp: ChukMCPServer) -> None:
    # Observe / drive ANY session.
    @mcp.tool(read_only_hint=True, description="Screenshot any session.")
    def admin_screenshot(session_id: str, scale: int = 2) -> ImageContent:
        return _screenshot(session_id, scale)

    @mcp.tool(read_only_hint=True, description="Registers of any session.")
    def admin_get_registers(session_id: str) -> dict:
        return _registers(session_id)

    @mcp.tool(read_only_hint=True, description="Read memory of any session.")
    def admin_read_memory(session_id: str, addr: int, length: int = 16) -> dict:
        return _read_memory(session_id, addr, length)

    @mcp.tool(read_only_hint=True, description="Disassemble any session.")
    def admin_disassemble(session_id: str, addr: int, count: int = 16) -> dict:
        return _disassemble(session_id, addr, count)

    @mcp.tool(description="Run frames on any session.")
    def admin_run_frames(session_id: str, n: int = 1) -> dict:
        return _run_frames(session_id, n)

    @mcp.tool(description="Type into any session.")
    def admin_type_text(session_id: str, text: str) -> dict:
        return _type(session_id, text)

    # Operate: list / inspect / lifecycle.
    @mcp.tool(read_only_hint=True, description="List all sessions and their state.")
    def list_sessions() -> dict:
        return {sid: SUPERVISOR.info(sid) for sid in SUPERVISOR.ids()}

    @mcp.tool(read_only_hint=True, description="Detailed state of one session.")
    def session_info(session_id: str) -> dict:
        return SUPERVISOR.info(session_id)

    @mcp.tool(description="Provision a session's machine now (otherwise lazy on first agent call).")
    def provision_session(session_id: str) -> dict:
        SUPERVISOR.session(session_id)
        return {"ok": True, "session_id": session_id}

    @mcp.tool(destructive_hint=True, description="Destroy a session (finalising its recording).")
    def destroy_session(session_id: str) -> dict:
        info = SUPERVISOR.destroy(session_id)
        return {"ok": True, "final_recording": _store_recording(session_id, info) if info else None}

    @mcp.tool(destructive_hint=True, description="Reset a session's machine to ROM boot.")
    def reset_session(session_id: str) -> dict:
        SUPERVISOR.machine(session_id).reset()
        return {"ok": True}

    # Game library: search + download from World of Spectrum.
    @mcp.tool(read_only_hint=True, description="Search World of Spectrum for games by title.")
    def search_games(query: str, limit: int = 10) -> dict:
        return _search_games(query, limit)

    @mcp.tool(destructive_hint=True, description="Find a game on World of Spectrum, download it, and load it into a session.")
    def load_game(session_id: str, query: str) -> dict:
        return _load_game(session_id, query)

    # Provisioning / state.
    @mcp.tool(destructive_hint=True, description="Load a .sna/.z80 snapshot (base64 or path) into a session.")
    def load_snapshot(session_id: str, fmt: str = "z80", data_b64: str | None = None, path: str | None = None) -> dict:
        data = _decode(data_b64, path)
        SUPERVISOR.machine(session_id).load_snapshot(fmt, data)
        return {"ok": True}

    @mcp.tool(destructive_hint=True, description="Insert a .tap and LOAD it into a session.")
    def load_tape(session_id: str, data_b64: str | None = None, path: str | None = None) -> dict:
        SUPERVISOR.machine(session_id).autoload_tape(_decode(data_b64, path))
        return {"ok": True}

    @mcp.tool(description="Manual checkpoint: save a session's state as base64 .sna.")
    def save_snapshot(session_id: str) -> dict:
        data = bytes(SUPERVISOR.machine(session_id).save_snapshot("sna"))
        return {"data_b64": base64.b64encode(data).decode(), "bytes": len(data)}

    @mcp.tool(destructive_hint=True, description="Poke bytes (base64) into a session's memory.")
    def write_memory(session_id: str, addr: int, data_b64: str) -> dict:
        data = base64.b64decode(data_b64)
        SUPERVISOR.machine(session_id).write_memory(addr, data)
        return {"ok": True, "addr": addr, "bytes": len(data)}

    @mcp.tool(destructive_hint=True, description="Set a Z80 register on a session.")
    def set_register(session_id: str, name: str, value: int) -> dict:
        SUPERVISOR.machine(session_id).set_register(name, value)
        return {"ok": True}

    # Snapshot timeline (rewind).
    @mcp.tool(read_only_hint=True, description="List the auto-snapshot timeline of a session.")
    def list_snapshots(session_id: str) -> dict:
        s = SUPERVISOR.get(session_id)
        return {"snapshots": [{"index": i, "at_frame": f} for i, (f, _) in enumerate(s.snapshots)] if s else []}

    @mcp.tool(description="Take a manual snapshot into the session's timeline.")
    def snapshot_now(session_id: str) -> dict:
        s = SUPERVISOR.session(session_id)
        return {"count": SUPERVISOR.take_snapshot(s)}

    @mcp.tool(destructive_hint=True, description="Rewind a session to a timeline snapshot by index.")
    def restore_snapshot(session_id: str, index: int) -> dict:
        s = SUPERVISOR.get(session_id)
        if not s or not (0 <= index < len(s.snapshots)):
            return {"ok": False, "error": "no such snapshot"}
        _, sna = s.snapshots[index]
        s.machine.load_snapshot("sna", sna)
        return {"ok": True, "restored_index": index}

    # Recording control + download.
    @mcp.tool(description="Stop+encode a session's recording (MP4 with sound); returns a download.")
    def stop_recording(session_id: str, filename: str | None = None, scale: int = 2) -> dict:
        info = SUPERVISOR.finalize_recording(session_id, filename, scale)
        if info is None:
            return {"ok": False, "error": "session was not recording"}
        return _store_recording(session_id, info)

    @mcp.tool(description="(Re)start recording a session.")
    def start_recording(session_id: str) -> dict:
        SUPERVISOR.restart_recording(session_id)
        return {"ok": True}


def _decode(data_b64: str | None, path: str | None) -> bytes:
    if path:
        with open(path, "rb") as fh:
            return fh.read()
    if data_b64:
        return base64.b64decode(data_b64)
    raise ValueError("provide data_b64 or path")


# ── builders ──────────────────────────────────────────────────────────────────


def build_agent() -> ChukMCPServer:
    mcp = ChukMCPServer(
        name="chuk-mcp-spectrum-agent",
        version="0.1.0",
        description="Play a ZX Spectrum: see the screen, press keys, let it run.",
    )
    register_agent_tools(mcp)
    return mcp


def build_admin() -> ChukMCPServer:
    mcp = ChukMCPServer(
        name="chuk-mcp-spectrum-admin",
        version="0.1.0",
        description="Operate ZX Spectrum sessions: lifecycle, pokes, recordings, snapshot timeline.",
    )
    register_admin_tools(mcp)
    return mcp
