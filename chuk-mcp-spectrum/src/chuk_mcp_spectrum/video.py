"""Encode a captured session (indexed frames + beeper audio) to an MP4 with sound.

The Rust core hands back *indexed* frames (one logical colour per pixel) and raw
``f32`` audio; presentation lives here: apply the authentic palette, scale, and
mux video + audio with the ffmpeg bundled by imageio-ffmpeg.
"""

from __future__ import annotations

import os
import subprocess
import tempfile
import wave

import imageio.v2 as imageio
import imageio_ffmpeg
import numpy as np

W, H = 256, 192
# 48K PAL frame rate: 3.5 MHz / 69888 T-states.
FRAMES_PER_SEC = 3_500_000 / 69888

# The 16 logical Spectrum colours (0–7 normal, 8–15 bright) as authentic RGB —
# matches the `display` crate's AUTHENTIC palette.
AUTHENTIC = np.array(
    [
        [0x00, 0x00, 0x00], [0x00, 0x00, 0xD7], [0xD7, 0x00, 0x00], [0xD7, 0x00, 0xD7],
        [0x00, 0xD7, 0x00], [0x00, 0xD7, 0xD7], [0xD7, 0xD7, 0x00], [0xD7, 0xD7, 0xD7],
        [0x00, 0x00, 0x00], [0x00, 0x00, 0xFF], [0xFF, 0x00, 0x00], [0xFF, 0x00, 0xFF],
        [0x00, 0xFF, 0x00], [0x00, 0xFF, 0xFF], [0xFF, 0xFF, 0x00], [0xFF, 0xFF, 0xFF],
    ],
    dtype=np.uint8,
)


def encode_session(
    indexed: bytes,
    count: int,
    decimate: int,
    audio: list[float],
    audio_rate: int,
    out_path: str,
    scale: int = 2,
) -> dict:
    """Mux captured frames + audio into ``out_path`` (MP4/H.264 + AAC).

    Returns metadata about the produced file.
    """
    if count == 0:
        raise ValueError("nothing recorded — call start_recording then run frames")

    # Indexed frames -> RGB via palette lookup, then nearest-neighbour upscale.
    frames = np.frombuffer(indexed, dtype=np.uint8).reshape(count, H, W)
    rgb = AUTHENTIC[frames]  # (count, H, W, 3)
    if scale > 1:
        rgb = rgb.repeat(scale, axis=1).repeat(scale, axis=2)

    fps = FRAMES_PER_SEC / max(decimate, 1)
    tmp = tempfile.mkdtemp(prefix="speccy-rec-")
    vtmp = os.path.join(tmp, "video.mp4")
    atmp = os.path.join(tmp, "audio.wav")

    writer = imageio.get_writer(
        vtmp, format="FFMPEG", mode="I", fps=fps,
        codec="libx264", quality=8, macro_block_size=None,
    )
    for frame in rgb:
        writer.append_data(frame)
    writer.close()

    has_audio = len(audio) > 0
    if has_audio:
        pcm = np.clip(np.asarray(audio, dtype=np.float32), -1.0, 1.0)
        pcm16 = (pcm * 32767.0).astype("<i2")
        with wave.open(atmp, "wb") as wav:
            wav.setnchannels(1)
            wav.setsampwidth(2)
            wav.setframerate(audio_rate)
            wav.writeframes(pcm16.tobytes())

    ffmpeg = imageio_ffmpeg.get_ffmpeg_exe()
    if has_audio:
        cmd = [
            ffmpeg, "-y", "-i", vtmp, "-i", atmp,
            "-c:v", "copy", "-c:a", "aac", "-b:a", "128k",
            "-shortest", out_path,
        ]
    else:
        cmd = [ffmpeg, "-y", "-i", vtmp, "-c:v", "copy", out_path]
    proc = subprocess.run(cmd, capture_output=True)
    if proc.returncode != 0:
        raise RuntimeError(f"ffmpeg mux failed: {proc.stderr.decode()[-500:]}")

    for f in (vtmp, atmp):
        if os.path.exists(f):
            os.remove(f)
    os.rmdir(tmp)

    return {
        "path": out_path,
        "frames": count,
        "fps": round(fps, 3),
        "duration_s": round(count / fps, 3),
        "width": W * scale,
        "height": H * scale,
        "has_audio": has_audio,
        "bytes": os.path.getsize(out_path),
    }
