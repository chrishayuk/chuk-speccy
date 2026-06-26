//! A persistent **cell host** — the long-lived, in-process session layer that keeps the
//! library index + warm runners alive *across* calls. This is what the per-call CLI can't
//! be: a process spawn (~ms) re-pays startup every call and throws away the warm runner,
//! defeating the whole compile-once/run-many advantage. A host holds a catalog of compiled
//! cartridges (searchable via [`CellIndex`]) and a slab of **loaded** runners drawn from a
//! [`CellPool`], so `load` once → `run` many reuses one warm machine (an O(touched) reset
//! between runs, no re-instantiate). Transport-agnostic: an MCP server, a daemon, or a
//! library caller all sit on top of the same `search`/`inspect`/`load`/`run` verbs.
use super::*;
use std::collections::HashMap;

/// A loaded cell: a warm runner plus the entry to invoke on it.
struct Loaded {
    runner: Runner,
    entry: String,
}

/// A persistent host over a library of cells: discover (`search`/`manifest`), then
/// `load` → `run` many → `unload`, keeping runners warm between calls.
#[derive(Default)]
pub struct CellHost {
    catalog: HashMap<String, Cartridge>,
    index: CellIndex,
    pool: CellPool,
    live: Vec<Option<Loaded>>,
}

impl CellHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a cartridge in the catalog + search index (keyed by its manifest id).
    pub fn add(&mut self, cart: Cartridge) {
        self.index.add(cart.manifest.clone());
        self.catalog.insert(cart.manifest.id.clone(), cart);
    }

    /// How many cells are in the catalog.
    pub fn len(&self) -> usize {
        self.catalog.len()
    }
    pub fn is_empty(&self) -> bool {
        self.catalog.is_empty()
    }

    /// Discover: rank the catalog by relevance to `query` (returns manifests, not cells).
    pub fn search(&self, query: &str, limit: usize) -> Vec<&Manifest> {
        self.index.search(query, limit)
    }

    /// Inspect a cell's manifest by id (the typed signature, caps, tags, …).
    pub fn manifest(&self, id: &str) -> Option<&Manifest> {
        self.catalog.get(id).map(|c| &c.manifest)
    }

    /// Instantiate a **warm** runner for `id` (from the pool) and return a handle. Cheap to
    /// `run` repeatedly; release it with [`unload`](Self::unload).
    pub fn load(&mut self, id: &str) -> Result<usize, String> {
        let cart = self
            .catalog
            .get(id)
            .ok_or_else(|| format!("no cell `{id}`"))?;
        let loaded = Loaded {
            runner: self.pool.acquire(&cart.program),
            entry: cart.manifest.entry.clone(),
        };
        // Reuse a freed handle slot if there is one.
        match self.live.iter().position(Option::is_none) {
            Some(h) => {
                self.live[h] = Some(loaded);
                Ok(h)
            }
            None => {
                self.live.push(Some(loaded));
                Ok(self.live.len() - 1)
            }
        }
    }

    fn loaded(&mut self, handle: usize) -> Result<&mut Loaded, String> {
        self.live
            .get_mut(handle)
            .and_then(Option::as_mut)
            .ok_or_else(|| format!("invalid cell handle {handle}"))
    }

    /// Run a loaded cell with typed `inputs` (and `args` in the convention registers),
    /// returning the rich [`Report`]. Reuses the warm runner — no re-instantiate.
    pub fn run(
        &mut self,
        handle: usize,
        args: &[u16],
        inputs: &[(u16, Ty, u64)],
        budget: u64,
    ) -> Result<Report, String> {
        let l = self.loaded(handle)?;
        let entry = l.entry.clone();
        l.runner.run_with_inputs(Some(&entry), args, inputs, budget)
    }

    /// The hot path: run a loaded cell for just the result registers/cycles/halt.
    pub fn run_fast(&mut self, handle: usize, args: &[u16], budget: u64) -> Result<Fast, String> {
        let l = self.loaded(handle)?;
        let entry = l.entry.clone();
        l.runner.run_fast(Some(&entry), args, budget)
    }

    /// Read a named typed field from a loaded cell's post-run memory.
    pub fn read_named(
        &mut self,
        handle: usize,
        fields: &[(String, u16, Ty)],
    ) -> Result<Vec<(String, u64)>, String> {
        Ok(self.loaded(handle)?.runner.read_named(fields))
    }

    /// Release a loaded cell, returning its bus to the pool for reuse.
    pub fn unload(&mut self, handle: usize) -> Result<(), String> {
        let l = self
            .live
            .get_mut(handle)
            .and_then(Option::take)
            .ok_or_else(|| format!("invalid cell handle {handle}"))?;
        self.pool.release(l.runner);
        Ok(())
    }

    /// How many cells are currently loaded (warm).
    pub fn live_count(&self) -> usize {
        self.live.iter().filter(|s| s.is_some()).count()
    }
}
