"""Admin endpoint — full control across all sessions."""

import os

from chuk_mcp_spectrum.server import build_admin


def main() -> None:
    mcp = build_admin()
    if os.environ.get("SPECTRUM_STDIO", "").lower() in ("1", "true", "yes"):
        mcp.run(stdio=True)
    else:
        mcp.run(host=os.environ.get("SPECTRUM_HOST", "127.0.0.1"),
                port=int(os.environ.get("SPECTRUM_ADMIN_PORT", "8012")))


if __name__ == "__main__":
    main()
