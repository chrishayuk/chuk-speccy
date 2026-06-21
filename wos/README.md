# wos

Search **World of Spectrum** and download a game ready to load into the core.

A small, standalone host utility (no dependency on the emulator crates) shared by
the `speccy-gui` CLI and — via `zxspec_py` — the MCP server, so there's one
fetcher. Metadata comes from the **ZXInfo API** (`api.zxinfo.dk`, the programmatic
World of Spectrum / Spectrum Computing backend); files come from the
`worldofspectrum.net` mirror, unzipped on the way out.

```rust
let game = wos::fetch("Jet Set Willy")?;   // best loadable build for the title
// game.format ∈ {"tap","z80","sna"};  game.data is the raw file bytes
```

- `search(query, limit)` → ranked `Entry` hits.
- `fetch(query)` → walks hits in relevance order and downloads the best loadable
  file, returning a `Game { title, year, format, data, source }`.

Notes:

- The core loads `.tap` (ROM trap fast-load), `.z80`, and `.sna` — **not `.tzx`**
  (custom/turbo loaders need real-time tape loading). `fetch` prefers authentic
  tape, then snapshots, and reports `.tzx`-only titles as unsupported.
- **48K builds are preferred** over 128K/+2/+3 (the core is a 48K).
- A title-similarity guard keeps `"Treasure Island Dizzy"` from falling back to
  `"Treasure Island"`.

```bash
cargo test -p wos                 # unit tests (matching, encoding)
cargo test -p wos -- --ignored    # network-gated end-to-end fetch
```
