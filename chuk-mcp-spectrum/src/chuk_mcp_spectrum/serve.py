"""Co-host all three heads in one process so they share the live `Supervisor`
(and thus the live machines):

* agent MCP endpoint  — what LLMs connect to
* admin MCP endpoint  — operators
* live viewer (web)   — open in a browser to watch the agent play, real time

For scale-out you can run `chuk-mcp-spectrum-agent` / `-admin` as separate
processes; session metadata and recordings are shared through the framework's
multi-server session store + artifact VFS, while live machines live in whichever
process provisioned them. Live viewing requires sharing the live machine, so the
viewer co-hosts here.
"""

import os
import threading

import uvicorn

from chuk_mcp_spectrum.server import build_admin, build_agent
from chuk_mcp_spectrum.viewer import build_viewer


def _thread(target, name):
    threading.Thread(target=target, daemon=True, name=name).start()


def main() -> None:
    host = os.environ.get("SPECTRUM_HOST", "127.0.0.1")
    agent_port = int(os.environ.get("SPECTRUM_AGENT_PORT", "8011"))
    admin_port = int(os.environ.get("SPECTRUM_ADMIN_PORT", "8012"))
    viewer_port = int(os.environ.get("SPECTRUM_VIEWER_PORT", "8010"))

    admin = build_admin()
    _thread(lambda: admin.run(host=host, port=admin_port), "spectrum-admin")
    _thread(
        lambda: uvicorn.run(build_viewer(), host=host, port=viewer_port, log_level="warning"),
        "spectrum-viewer",
    )

    print(
        f"viewer → http://{host}:{viewer_port}/   "
        f"agent → http://{host}:{agent_port}/mcp   "
        f"admin → http://{host}:{admin_port}/mcp"
    )
    build_agent().run(host=host, port=agent_port)


if __name__ == "__main__":
    main()
