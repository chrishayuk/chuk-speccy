"""Agent endpoint — the small consumer surface (observe + drive this session)."""

import os

from chuk_mcp_spectrum.server import build_agent


def main() -> None:
    mcp = build_agent()
    if os.environ.get("SPECTRUM_STDIO", "").lower() in ("1", "true", "yes"):
        mcp.run(stdio=True)
    else:
        mcp.run(host=os.environ.get("SPECTRUM_HOST", "127.0.0.1"),
                port=int(os.environ.get("SPECTRUM_AGENT_PORT", "8011")))


if __name__ == "__main__":
    main()
