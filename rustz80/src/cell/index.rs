//! A tiny in-memory **tool index** over cell manifests — the "millions stored, 3–10
//! visible" selection step. A router builds an index over many cartridge manifests and
//! `search`es it by token overlap (tags > id > summary), surfacing a handful of candidates
//! so only their compact manifests — not the whole library — ever reach the model.
use super::Manifest;

/// A searchable set of cell manifests.
#[derive(Default)]
pub struct CellIndex {
    entries: Vec<Manifest>,
}

impl CellIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a cell's manifest to the index.
    pub fn add(&mut self, m: Manifest) {
        self.entries.push(m);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The manifests best matching `query`, best first (ties broken by id), up to `limit`
    /// — scoring token overlap with tags (×3), id (×2), and summary (×1). Only positive
    /// matches are returned.
    pub fn search(&self, query: &str, limit: usize) -> Vec<&Manifest> {
        let q = tokens(query);
        let mut scored: Vec<(i32, &Manifest)> = self
            .entries
            .iter()
            .map(|m| (score(m, &q), m))
            .filter(|(s, _)| *s > 0)
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
        scored.into_iter().take(limit).map(|(_, m)| m).collect()
    }
}

/// Lowercased alphanumeric tokens.
fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn score(m: &Manifest, query: &[String]) -> i32 {
    let id = tokens(&m.id);
    let summary = tokens(&m.summary);
    let tags: Vec<String> = m.tags.iter().flat_map(|t| tokens(t)).collect();
    let mut s = 0;
    for t in query {
        if tags.iter().any(|x| x == t) {
            s += 3;
        }
        if id.iter().any(|x| x == t) {
            s += 2;
        }
        if summary.iter().any(|x| x == t) {
            s += 1;
        }
    }
    s
}
