"""Tests for the supervisor (autonomy plane) + the agent/admin tool surfaces.
Needs SPECTRUM_ROM set to a 48K ROM image."""

import os

import pytest

ROM = os.environ.get("SPECTRUM_ROM")
pytestmark = pytest.mark.skipif(not ROM, reason="set SPECTRUM_ROM to a 48K ROM")

from chuk_mcp_spectrum import server  # noqa: E402
from chuk_mcp_spectrum.supervisor import Config, Supervisor  # noqa: E402


@pytest.fixture
def sup(monkeypatch):
    s = Supervisor(Config())
    monkeypatch.setattr(server, "SUPERVISOR", s)
    return s


def test_two_surfaces_are_split():
    def names(mcp):
        return sorted(t.name for t in mcp.get_tools())

    agent = names(server.build_agent())
    admin = names(server.build_admin())
    assert "screenshot" in agent and "press_keys" in agent and "disassemble" in agent
    # agent is policy-free: no lifecycle / pokes / recording / library loading
    assert not any(x in agent for x in ("create_machine", "write_memory", "stop_recording", "destroy_session", "load_game"))
    assert len(agent) < len(admin)
    assert {"write_memory", "stop_recording", "restore_snapshot", "list_sessions", "search_games", "load_game"} <= set(admin)


def test_provision_records_by_default(sup):
    s = sup.session("s1")
    assert s.recording is True and s.audio is True
    assert "1982 Sinclair Research Ltd" in s.machine.screen_text()


def test_drive_and_observe(sup):
    sid = "s2"
    sup.session(sid)
    server._press(sid, ["p"], 3)
    server._type(sid, "6*7\n")
    server._run_frames(sid, 30)
    top = [l for l in server._screen_text(sid).splitlines() if l.strip()][0]
    assert top.startswith("42")


def test_screenshot_is_png(sup):
    sup.session("s3")
    shot = server._screenshot("s3")
    assert shot.mimeType == "image/png" and len(shot.data) > 0


def test_disassemble(sup):
    sid = "s3d"
    sup.session(sid)
    # Poke a tiny program into RAM and disassemble it back.
    sup.machine(sid).write_memory(0x8000, b"\x21\x00\x40\x76")  # LD HL,$4000 ; HALT
    out = server._disassemble(sid, 0x8000, 2)
    assert out["lines"][0]["text"] == "LD HL,$4000"
    assert out["lines"][0]["bytes"] == "210040"
    assert out["lines"][1]["text"] == "HALT"


def test_auto_snapshot_cadence(sup):
    sup.config.snapshot_every_frames = 25
    sid = "s4"
    sup.session(sid)
    sup.advance_frames(sid, 30)
    sup.advance_frames(sid, 30)
    assert len(sup.get(sid).snapshots) >= 1


def test_record_and_finalize_has_audio(sup, tmp_path):
    sid = "s5"
    sup.session(sid)
    sup.advance_frames(sid, 60)
    info = sup.finalize_recording(sid, filename=str(tmp_path / "rec.mp4"))
    assert info["has_audio"] is True
    assert os.path.getsize(info["path"]) > 0


@pytest.mark.skipif(not os.environ.get("SPECTRUM_NET_TESTS"), reason="set SPECTRUM_NET_TESTS=1 for World of Spectrum network tests")
def test_search_and_load_game(sup):
    res = server._search_games("Jet Set Willy", 3)
    assert res["results"]
    assert any("willy" in r["title"].lower() for r in res["results"])

    out = server._load_game("g-net", "Spy vs Spy")
    assert out["ok"] is True
    assert out["format"] in ("tap", "z80", "sna")
    # The downloaded game rendered something (not a blank screen).
    idx = bytes(sup.machine("g-net").screen_indexed())
    assert sum(1 for b in idx if b) > 0


def test_host_trap_abi():
    """The ED FE host-trap ABI: dispatch, register/memory access, carry, the
    liveness guard, and clean NOP without a dispatcher."""
    import zxspec_py

    m = zxspec_py.Machine(open(ROM, "rb").read())
    retained = {}

    def on_trap(ctx):
        retained["ctx"] = ctx  # keep it to test the guard
        if ctx.a == 0x10:  # mul16
            ctx.set_hl((ctx.bc * ctx.de) & 0xFFFF)
            ctx.set_carry(False)
        else:
            ctx.set_carry(True)

    m.register_host_dispatcher(on_trap)
    m.write_memory(0x8000, b"\xED\xFE")
    for name, val in (("pc", 0x8000), ("a", 0x10), ("bc", 7), ("de", 6)):
        m.set_register(name, val)
    m.step(1)
    assert m.registers()["hl"] == 42
    assert m.registers()["pc"] == 0x8002

    # A retained ctx raises instead of dereferencing freed state.
    with pytest.raises(RuntimeError):
        retained["ctx"].read(0x4000, 1)

    # Unknown id → carry set.
    m.set_register("pc", 0x8000)
    m.set_register("a", 0x99)
    m.set_register("f", 0)
    m.step(1)
    assert m.registers()["f"] & 1

    # No dispatcher → ED FE is a clean NOP (the fidelity dial).
    m.clear_host_dispatcher()
    m.set_register("pc", 0x8000)
    m.set_register("hl", 0)
    m.set_register("a", 0)
    m.step(1)
    assert m.registers()["hl"] == 0


def test_chat_trap_protocol():
    """The CHAT_* host protocol over the ED FE trap: BEGIN a turn, then POLL the
    streamed reply events back out of RAM (echo responder — no LLM needed)."""
    import zxspec_py
    from chuk_mcp_spectrum import chat

    m = zxspec_py.Machine(open(ROM, "rb").read())
    session = chat.ChatSession()  # echo responder
    m.register_host_dispatcher(chat.make_dispatcher(session))
    m.write_memory(0x8000, b"\xED\xFE")  # HOSTCALL

    def trap(sid, hl=0, bc=0):
        for name, val in (("pc", 0x8000), ("a", sid), ("hl", hl), ("bc", bc)):
            m.set_register(name, val)
        m.step(1)

    prompt = b"HI"
    m.write_memory(0x9000, prompt)
    trap(chat.CHAT_BEGIN, hl=0x9000, bc=len(prompt) << 8)  # B = length

    out = bytearray()
    for _ in range(64):
        trap(chat.CHAT_POLL, hl=0x9100, bc=64 << 8)  # B = capacity
        code, n = m.registers()["a"], m.registers()["bc"]
        if code == chat.EV_TEXT:
            out += bytes(m.read_memory(0x9100, n))
        elif code == chat.EV_DONE:
            break
    assert out.decode("latin-1") == "You said: HI"

    # POLL after the queue drains → EV_NONE.
    trap(chat.CHAT_POLL, hl=0x9100, bc=64 << 8)
    assert m.registers()["a"] == chat.EV_NONE


def test_restore_snapshot_rewinds(sup):
    sid = "s6"
    sup.session(sid)
    sup.advance_frames(sid, 10)
    idx = sup.take_snapshot(sup.get(sid)) - 1
    # mutate, then restore
    sup.machine(sid).write_memory(0x4000, b"\xff\xff\xff\xff")
    _, sna = sup.get(sid).snapshots[idx]
    sup.machine(sid).load_snapshot("sna", sna)
    assert bytes(sup.machine(sid).read_memory(0x4000, 4)) != b"\xff\xff\xff\xff"
