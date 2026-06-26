"""The warm cell library — a thin Python wrapper over the PyO3 `cellz_py.CellHost`.

It loads a directory of cells once (`.rs` sources, metadata from a `//!` header; or
precompiled `.cell` cartridges), then serves `search` / `inspect` / `run`. `run` hides
handles: it lazily `load`s a warm runner per id and reuses it, so the model just names a
tool + args and the host keeps it warm across calls.
"""

from __future__ import annotations

import pathlib

import cellz_py


def _parse_header(src: str) -> tuple[str, list[str], str | None]:
    """Read a cell source's leading `//!` header → (summary, tags, entry)."""
    summary, tags, entry = "", [], None
    for line in src.splitlines():
        s = line.strip()
        if s.startswith("//!"):
            r = s[3:].strip()
            if r.startswith("tags:"):
                tags = [t.strip() for t in r[5:].split(",") if t.strip()]
            elif r.startswith("entry:"):
                entry = r[6:].strip()
            elif not summary:
                summary = r
        elif s and not s.startswith("//"):
            break  # first code line — header done
    return summary, tags, entry


class CellLibrary:
    """A warm host over a directory of cells."""

    def __init__(self, directory: str):
        self.directory = directory
        self.host = cellz_py.CellHost()
        self._handles: dict[str, int] = {}
        self._ids: list[str] = []
        self._load(pathlib.Path(directory))

    def _load(self, d: pathlib.Path) -> None:
        if not d.is_dir():
            raise FileNotFoundError(f"cell library dir not found: {d}")
        for f in sorted(d.iterdir()):
            if f.suffix == ".rs":
                src = f.read_text()
                summary, tags, entry = _parse_header(src)
                self.host.add_source(f.stem, src, summary, tags, entry)
                self._ids.append(f.stem)
            elif f.suffix == ".cell":
                self.host.add_cell(f.read_bytes())

    # ── discover ────────────────────────────────────────────────────────────
    def search(self, query: str, limit: int = 10) -> list[dict]:
        return list(self.host.search(query, limit))

    def inspect(self, cell_id: str) -> dict | None:
        return self.host.manifest(cell_id)

    def list(self) -> list[dict]:
        return [m for i in self._ids if (m := self.host.manifest(i)) is not None]

    # ── run (warm, handles hidden) ────────────────────────────────────────────
    def run(self, cell_id: str, args: list[int]) -> dict:
        if cell_id not in self._handles:
            if self.host.manifest(cell_id) is None:
                raise ValueError(f"no cell `{cell_id}`")
            self._handles[cell_id] = self.host.load(cell_id)
        return self.host.run(self._handles[cell_id], list(args))

    def __len__(self) -> int:
        return len(self.host)
