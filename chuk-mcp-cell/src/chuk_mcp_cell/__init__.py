"""chuk-mcp-cell — an MCP server over a warm library of deterministic micro-tools (cells).

The architecture in one line: **millions of cells can be stored/indexed, but only a handful
ever reach the model** — the agent `cell_search`es the library, `cell_inspect`s a few, and
`cell_run`s the one it wants. The host (index + warm runners) lives in this one process, so
runs stay microsecond-warm across calls — the warm-path a per-invocation CLI can't give.
"""

__version__ = "0.1.0"
