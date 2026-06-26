"""Entry point — serve the cell tools over stdio (for an MCP client) or HTTP.

`CELL_LIBRARY` selects the directory of cells (default `rustz80/cells`). Set `CELL_STDIO=1`
to speak MCP over stdio (the usual client transport); otherwise it listens on HTTP.
"""

import os

from chuk_mcp_cell.server import build_server


def main() -> None:
    mcp = build_server()
    if os.environ.get("CELL_STDIO", "").lower() in ("1", "true", "yes"):
        mcp.run(stdio=True)
    else:
        mcp.run(
            host=os.environ.get("CELL_HOST", "127.0.0.1"),
            port=int(os.environ.get("CELL_PORT", "8021")),
        )


if __name__ == "__main__":
    main()
