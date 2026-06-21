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
    assert "screenshot" in agent and "press_keys" in agent
    # agent is policy-free: no lifecycle / pokes / recording
    assert not any(x in agent for x in ("create_machine", "write_memory", "stop_recording", "destroy_session"))
    assert len(agent) < len(admin)
    assert {"write_memory", "stop_recording", "restore_snapshot", "list_sessions"} <= set(admin)


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
