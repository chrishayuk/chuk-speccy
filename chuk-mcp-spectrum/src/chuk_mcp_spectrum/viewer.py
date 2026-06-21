"""Live viewer — a web "head" that mirrors a session's screen in real time so a
human can watch the agent play (`docs/05-frontends-display-spec.md`, the
web/streamed head). It reads the shared `Supervisor`, so it shows whatever the
agent has driven the machine to; as the agent advances frames, the stream
updates.

Plain MJPEG (`multipart/x-mixed-replace`) — a browser `<img>` renders it with no
JavaScript. Open `/` for the session list, `/view/<id>` to watch one.
"""

from __future__ import annotations

import asyncio
import io

from PIL import Image
from starlette.applications import Starlette
from starlette.responses import HTMLResponse, Response, StreamingResponse
from starlette.routing import Route

from chuk_mcp_spectrum.supervisor import SUPERVISOR

W, H, SCALE = 256, 192, 2
FPS = 25


def _jpeg(session_id: str) -> bytes | None:
    s = SUPERVISOR.get(session_id)
    if s is None:
        return None
    rgba = bytes(s.machine.screen_rgba())
    img = Image.frombytes("RGBA", (W, H), rgba).convert("RGB")
    if SCALE > 1:
        img = img.resize((W * SCALE, H * SCALE), Image.NEAREST)
    buf = io.BytesIO()
    img.save(buf, "JPEG", quality=80)
    return buf.getvalue()


async def index(_request):
    rows = "".join(
        f'<li><a href="/view/{sid}">{sid}</a> '
        f'<span class=meta>frames {info["frames_run"]} · '
        f'{"● rec" if info["recording"] else "○"} · pc {info["pc"]:#06x}</span></li>'
        for sid, info in ((sid, SUPERVISOR.info(sid)) for sid in SUPERVISOR.ids())
    )
    if not rows:
        rows = "<li><em>no active sessions — connect an agent and drive it</em></li>"
    html = f"""<!doctype html><meta charset=utf-8><title>chuk-speccy live</title>
<style>body{{background:#111;color:#ddd;font:14px ui-monospace,monospace;margin:2rem}}
a{{color:#6cf}} .meta{{color:#888}} li{{margin:.4rem 0}}</style>
<h1>ZX Spectrum — live sessions</h1><ul>{rows}</ul>
<p class=meta>auto-refreshes every 2s</p>
<script>setTimeout(()=>location.reload(),2000)</script>"""
    return HTMLResponse(html)


async def view(request):
    sid = request.path_params["sid"]
    if SUPERVISOR.get(sid) is None:
        return HTMLResponse(f"<p>no such session {sid}</p>", status_code=404)
    html = f"""<!doctype html><meta charset=utf-8><title>{sid} — live</title>
<style>body{{background:#000;color:#888;font:13px ui-monospace,monospace;text-align:center;margin:1rem}}
img{{image-rendering:pixelated;border:2px solid #333;max-width:96vw}} a{{color:#6cf}}</style>
<p><a href="/">← sessions</a> · watching <b>{sid}</b> (live)</p>
<img src="/stream/{sid}" alt="live screen">"""
    return HTMLResponse(html)


async def frame(request):
    data = _jpeg(request.path_params["sid"])
    if data is None:
        return Response("no such session", status_code=404)
    return Response(data, media_type="image/jpeg")


async def stream(request):
    sid = request.path_params["sid"]
    if SUPERVISOR.get(sid) is None:
        return Response("no such session", status_code=404)

    async def gen():
        while SUPERVISOR.get(sid) is not None:
            data = _jpeg(sid)
            if data is not None:
                yield b"--frame\r\nContent-Type: image/jpeg\r\n\r\n" + data + b"\r\n"
            await asyncio.sleep(1 / FPS)

    return StreamingResponse(gen(), media_type="multipart/x-mixed-replace; boundary=frame")


def build_viewer() -> Starlette:
    return Starlette(
        routes=[
            Route("/", index),
            Route("/view/{sid}", view),
            Route("/frame/{sid}.jpg", frame),
            Route("/stream/{sid}", stream),
        ]
    )
