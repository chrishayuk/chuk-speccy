"""The MCP surface over a warm cell library — a thin router, **not** a tool per cell.

The whole point: a library can hold millions of cells, but MCP exposes only a few fixed
verbs. The agent `cell_search`es to surface a handful of candidate manifests, `cell_inspect`s
the typed interface, and `cell_run`s the one it picks — so the model's context never holds
more than the few cells it's actually considering. The host (index + warm runners) stays
alive in this process; `cell_run` keeps a runner warm per cell across calls.

Built on `chuk-mcp-server` (`@mcp.tool`); the same tool bodies would back a socket daemon.
The session/warmth lives in `CellLibrary`; tools carry no policy.
"""

from __future__ import annotations

import os

from chuk_mcp_server import ChukMCPServer

from chuk_mcp_cell.library import CellLibrary

_LIBRARY: CellLibrary | None = None


def library() -> CellLibrary:
    """The process-wide warm library (lazily built from `$CELL_LIBRARY`)."""
    global _LIBRARY
    if _LIBRARY is None:
        _LIBRARY = CellLibrary(os.environ.get("CELL_LIBRARY", "rustz80/cells"))
    return _LIBRARY


def build_server() -> ChukMCPServer:
    mcp = ChukMCPServer(
        name="chuk-mcp-cell",
        version="0.1.0",
        description="Discover and run deterministic micro-tools (cells): search, inspect, run.",
    )
    lib = library()

    @mcp.tool(
        read_only_hint=True,
        description="Search the cell library by relevance; returns brief manifests "
        "(id, summary, tags, signature). Inspect/run only the few you pick — the library "
        "may hold far more cells than belong in context.",
    )
    def cell_search(query: str, limit: int = 10) -> dict:
        return {"results": lib.search(query, limit)}

    @mcp.tool(
        read_only_hint=True,
        description="Full manifest for a cell id: typed signature (params/ret/state), "
        "abi version, source hash.",
    )
    def cell_inspect(id: str) -> dict:
        m = lib.inspect(id)
        return m if m is not None else {"error": f"no cell `{id}`"}

    @mcp.tool(
        read_only_hint=True,
        description="List every cell in the library (brief manifests).",
    )
    def cell_list() -> dict:
        return {"cells": lib.list()}

    @mcp.tool(
        description="Run a cell by id with register args (u16 each); returns result + regs "
        "+ cost (cycles, trapped_ops) + halt. The runner stays warm across calls.",
    )
    def cell_run(id: str, args: list[int] | None = None) -> dict:
        try:
            return lib.run(id, args or [])
        except ValueError as e:
            return {"error": str(e)}

    return mcp
