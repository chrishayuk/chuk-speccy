"""The autonomy plane (`docs/06-roles-autonomy-spec.md`).

A process-wide `Supervisor` owns sessions and runs policy *without* agent tool
calls: it provisions a machine per session, records every session by default,
snapshots on a configurable cadence (a rewindable timeline), and reaps idle
sessions. Agent tools are thin shims that resolve "my machine" from the session;
admin tools reach across all sessions. Both planes share this one singleton.
"""

from __future__ import annotations

import os
import tempfile
import threading
import time
from dataclasses import dataclass, field
from typing import Optional

import zxspec_py

from chuk_mcp_spectrum.video import encode_session


def _env_bool(name: str, default: bool) -> bool:
    v = os.environ.get(name)
    return default if v is None else v.strip().lower() in ("1", "true", "yes", "on")


def _env_int(name: str, default: int) -> int:
    try:
        return int(os.environ[name])
    except (KeyError, ValueError):
        return default


@dataclass
class Config:
    """Policy knobs (env-overridable). Snapshot cadence is frame-based now —
    at 50 fps that *is* time-based; a wall-clock variant slots in for real-time."""

    record_by_default: bool = field(default_factory=lambda: _env_bool("SPECTRUM_RECORD", True))
    audio_rate: int = field(default_factory=lambda: _env_int("SPECTRUM_AUDIO_RATE", 44_100))
    decimate: int = field(default_factory=lambda: _env_int("SPECTRUM_VIDEO_DECIMATE", 2))
    # 0 disables auto-snapshots; otherwise checkpoint every N emulated frames.
    snapshot_every_frames: int = field(default_factory=lambda: _env_int("SPECTRUM_SNAPSHOT_FRAMES", 0))
    max_snapshots: int = field(default_factory=lambda: _env_int("SPECTRUM_MAX_SNAPSHOTS", 64))
    idle_reap_seconds: int = field(default_factory=lambda: _env_int("SPECTRUM_IDLE_REAP_SECONDS", 1800))
    boot_frames: int = field(default_factory=lambda: _env_int("SPECTRUM_BOOT_FRAMES", 250))
    default_game: Optional[str] = field(default_factory=lambda: os.environ.get("SPECTRUM_DEFAULT_GAME"))
    recordings_dir: str = field(
        default_factory=lambda: os.environ.get("SPECTRUM_RECORDINGS_DIR")
        or os.path.join(tempfile.gettempdir(), "speccy-recordings")
    )


@dataclass
class Session:
    machine: "zxspec_py.Machine"
    recording: bool
    audio: bool
    created: float
    last_active: float
    frames_run: int = 0
    last_snapshot_frame: int = 0
    # Timeline: (frames_run_at_capture, sna_bytes). The newest-trimmed history.
    snapshots: list = field(default_factory=list)


class Supervisor:
    def __init__(self, config: Optional[Config] = None) -> None:
        self.config = config or Config()
        self._sessions: dict[str, Session] = {}
        self._rom: Optional[bytes] = None
        self._lock = threading.RLock()

    # --- ROM ----------------------------------------------------------------

    def rom(self) -> bytes:
        if self._rom is None:
            path = os.environ.get("SPECTRUM_ROM")
            if not path:
                raise RuntimeError("set SPECTRUM_ROM to a 48K ROM image")
            with open(path, "rb") as fh:
                self._rom = fh.read()
        return self._rom

    # --- provisioning -------------------------------------------------------

    def provision(self, session_id: str) -> Session:
        """Create + bind a machine for `session_id`, applying policy."""
        m = zxspec_py.Machine(self.rom())
        cfg = self.config
        recording = cfg.record_by_default
        audio = recording  # only bother capturing audio when we'll encode it

        if cfg.default_game and os.path.exists(cfg.default_game):
            self._load_media(m, cfg.default_game)
        else:
            m.run_frames(cfg.boot_frames)  # boot to the BASIC prompt

        if audio:
            m.enable_audio(cfg.audio_rate)
            m.drain_audio()
        if recording:
            m.start_recording(cfg.decimate)

        now = time.time()
        s = Session(machine=m, recording=recording, audio=audio, created=now, last_active=now)
        self._sessions[session_id] = s
        return s

    @staticmethod
    def _load_media(m: "zxspec_py.Machine", path: str) -> None:
        with open(path, "rb") as fh:
            data = fh.read()
        fmt = "tap" if path.endswith(".tap") else ("sna" if path.endswith(".sna") else "z80")
        Supervisor._load_media_bytes(m, fmt, data)

    @staticmethod
    def _load_media_bytes(m: "zxspec_py.Machine", fmt: str, data: bytes) -> None:
        """Load a game by format. `.tap` trap-loads instantly; `.tzx` plays the
        tape *signal* in real time (turbo/custom loaders), run until it finishes;
        snapshots load directly."""
        if fmt == "tap":
            m.run_frames(250)
            m.autoload_tape(data)
            m.run_frames(300)
        elif fmt == "tzx":
            m.run_frames(250)
            m.type_load()  # LOAD "" — the loader reads the signal
            m.play_tape("tzx", data)
            frames = 0
            while m.tape_playing() and frames < 80_000:
                m.run_frames(200)
                frames += 200
            m.run_frames(200)
        else:
            m.load_snapshot(fmt, data)

    def load_game(self, session_id: str, fmt: str, data: bytes) -> Session:
        """Swap in a freshly-downloaded game: reset the machine, load it, and
        restart the session's recording/snapshot timeline so the capture is the
        new game only."""
        s = self.session(session_id)
        m = s.machine
        m.reset()
        self._load_media_bytes(m, fmt, data)
        s.frames_run = 0
        s.last_snapshot_frame = 0
        s.snapshots.clear()
        if s.audio:
            m.enable_audio(self.config.audio_rate)
            m.drain_audio()
        if s.recording:
            m.start_recording(self.config.decimate)
        s.last_active = time.time()
        return s

    def session(self, session_id: str) -> Session:
        with self._lock:
            s = self._sessions.get(session_id)
            if s is None:
                s = self.provision(session_id)
            s.last_active = time.time()
            return s

    def machine(self, session_id: str) -> "zxspec_py.Machine":
        return self.session(session_id).machine

    def get(self, session_id: str) -> Optional[Session]:
        return self._sessions.get(session_id)

    def ids(self) -> list[str]:
        return list(self._sessions)

    # --- driving (agent advances funnel through here for cadence) ----------

    def advance_frames(self, session_id: str, n: int) -> Session:
        s = self.session(session_id)
        s.machine.run_frames(n)
        s.frames_run += n
        self._maybe_snapshot(s)
        s.last_active = time.time()
        return s

    def note_activity(self, session_id: str) -> Session:
        """For input/step tools that advance time inside the core; keeps the
        session warm and checks the snapshot cadence."""
        s = self.session(session_id)
        # The core advanced frames during press/type; approximate the counter so
        # cadence still fires roughly on time.
        self._maybe_snapshot(s)
        s.last_active = time.time()
        return s

    def _maybe_snapshot(self, s: Session) -> None:
        every = self.config.snapshot_every_frames
        if every <= 0:
            return
        if s.frames_run - s.last_snapshot_frame >= every:
            self.take_snapshot(s)

    def take_snapshot(self, s: Session) -> int:
        sna = bytes(s.machine.save_snapshot("sna"))
        s.snapshots.append((s.frames_run, sna))
        s.last_snapshot_frame = s.frames_run
        if len(s.snapshots) > self.config.max_snapshots:
            s.snapshots.pop(0)
        return len(s.snapshots)

    # --- recording lifecycle ------------------------------------------------

    def finalize_recording(self, session_id: str, filename: Optional[str] = None, scale: int = 2) -> Optional[dict]:
        """Stop the session's recording and encode the MP4. Returns None if the
        session wasn't recording."""
        s = self.get(session_id)
        if s is None or not s.recording:
            return None
        count = s.machine.stop_recording()
        decimate = s.machine.recording_decimate()
        indexed = bytes(s.machine.take_recording())
        audio = list(s.machine.drain_audio()) if s.audio else []
        s.recording = False
        if count == 0:
            return None
        os.makedirs(self.config.recordings_dir, exist_ok=True)
        name = filename or f"{session_id}-{int(s.created)}.mp4"
        out = name if os.path.isabs(name) else os.path.join(self.config.recordings_dir, name)
        return encode_session(indexed, count, decimate, audio, self.config.audio_rate, out, scale)

    def restart_recording(self, session_id: str) -> None:
        s = self.session(session_id)
        if s.audio:
            s.machine.enable_audio(self.config.audio_rate)
            s.machine.drain_audio()
        s.machine.start_recording(self.config.decimate)
        s.recording = True

    # --- lifecycle ----------------------------------------------------------

    def destroy(self, session_id: str, finalize: bool = True) -> Optional[dict]:
        info = self.finalize_recording(session_id) if finalize else None
        self._sessions.pop(session_id, None)
        return info

    def reap_idle(self) -> list[str]:
        now = time.time()
        timeout = self.config.idle_reap_seconds
        reaped = []
        with self._lock:
            for sid, s in list(self._sessions.items()):
                if now - s.last_active > timeout:
                    self.destroy(sid)
                    reaped.append(sid)
        return reaped

    def info(self, session_id: str) -> dict:
        s = self.get(session_id)
        if s is None:
            return {"exists": False}
        return {
            "exists": True,
            "recording": s.recording,
            "frames_run": s.frames_run,
            "snapshots": len(s.snapshots),
            "idle_s": round(time.time() - s.last_active, 1),
            "pc": s.machine.registers()["pc"],
        }


# One supervisor per process — shared by the agent and admin tool planes.
SUPERVISOR = Supervisor()
