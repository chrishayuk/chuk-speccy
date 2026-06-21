//! Search World of Spectrum and download a game ready to load into the core.
//!
//! Backed by the **ZXInfo API** (`api.zxinfo.dk`, the programmatic backend for
//! World of Spectrum / Spectrum Computing) for metadata, and the
//! `worldofspectrum.net` file mirror for the actual files. The archive stores
//! games zipped; we unzip and hand back the inner bytes.
//!
//! The core loads `.tap` (ROM trap fast-load), `.z80`, and `.sna` — not `.tzx`
//! (custom/turbo loaders need real-time tape loading, a future item). So we
//! only ever return those three, preferring authentic tape (`.tap`).

use std::io::Read;
use std::time::Duration;

/// ZXInfo metadata API.
const API: &str = "https://api.zxinfo.dk/v3";
/// File mirrors, tried in order. Spectrum Computing serves both the legacy
/// `/pub/sinclair/...` paths and the newer `/zxdb/sinclair/entries/...` ones;
/// World of Spectrum is a fallback for the legacy layout.
const MIRRORS: [&str; 2] = ["https://spectrumcomputing.co.uk", "https://worldofspectrum.net"];
/// Formats the core can load, best first: instant trap/snapshot loads, then
/// `.tzx` (real-time, signal-level — needed for turbo/custom loaders, slower).
const FORMATS: [&str; 4] = ["tap", "z80", "sna", "tzx"];

/// A search hit (metadata only).
#[derive(Debug, Clone)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub year: Option<u32>,
    pub machine: Option<String>,
    pub publisher: Option<String>,
}

/// A downloaded game, ready to load.
#[derive(Debug, Clone)]
pub struct Game {
    pub title: String,
    pub year: Option<u32>,
    /// `"tap"`, `"z80"`, or `"sna"`.
    pub format: String,
    /// The extracted file bytes.
    pub data: Vec<u8>,
    /// The archive path it came from.
    pub source: String,
}

#[derive(Debug)]
pub enum Error {
    Http(String),
    Parse(String),
    NotFound(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Http(m) => write!(f, "network error: {m}"),
            Error::Parse(m) => write!(f, "parse error: {m}"),
            Error::NotFound(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for Error {}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .user_agent("chuk-speccy/0.1 (+https://github.com/chrishayuk/chuk-speccy)")
        .build()
}

/// GET a URL and parse the body as JSON (avoids ureq's optional `json` feature).
fn get_json(agent: &ureq::Agent, url: &str) -> Result<serde_json::Value, Error> {
    let resp = agent.get(url).call().map_err(|e| Error::Http(e.to_string()))?;
    serde_json::from_reader(resp.into_reader()).map_err(|e| Error::Parse(e.to_string()))
}

/// Search the archive for software matching `query`, best relevance first.
pub fn search(query: &str, limit: usize) -> Result<Vec<Entry>, Error> {
    search_with(&agent(), query, limit)
}

fn search_with(agent: &ureq::Agent, query: &str, limit: usize) -> Result<Vec<Entry>, Error> {
    let url = format!(
        "{API}/search?query={}&mode=tiny&size={limit}&contenttype=SOFTWARE&sort=rel_desc",
        pct(query, false)
    );
    let v = get_json(agent, &url)?;

    let hits = v["hits"]["hits"]
        .as_array()
        .ok_or_else(|| Error::Parse("unexpected search response".into()))?;
    Ok(hits.iter().map(parse_entry).collect())
}

fn parse_entry(h: &serde_json::Value) -> Entry {
    let s = &h["_source"];
    Entry {
        id: h["_id"].as_str().unwrap_or_default().to_string(),
        title: s["title"].as_str().unwrap_or("?").to_string(),
        year: s["originalYearOfRelease"].as_u64().map(|y| y as u32),
        machine: s["machineType"].as_str().map(str::to_string),
        publisher: s["publishers"][0]["name"].as_str().map(str::to_string),
    }
}

/// Search, then download the best loadable file for the best matching title.
///
/// Walks hits in relevance order; for each whose title plausibly matches the
/// query, tries its files in format-preference order until one downloads and
/// unzips. This skips wrong-title hits (so "Treasure Island Dizzy" never falls
/// back to "Treasure Island") and dead links.
pub fn fetch(query: &str) -> Result<Game, Error> {
    let agent = agent();
    let entries = search_with(&agent, query, 10)?;
    let want = normalize(query);

    for e in entries.iter().filter(|e| title_matches(&e.title, &want)) {
        for (_, ext, path) in candidates(&agent, &e.id)? {
            if let Ok(data) = download_and_extract(&agent, &path, &ext) {
                return Ok(Game {
                    title: e.title.clone(),
                    year: e.year,
                    format: ext,
                    data,
                    source: path,
                });
            }
        }
    }

    Err(Error::NotFound(format!(
        "no loadable file (.tap/.z80/.sna/.tzx) found on World of Spectrum for {query:?}"
    )))
}

/// A loadable file candidate: `(format_rank, ext, archive_path)`, lower rank first.
type Candidate = (usize, String, String);

/// List a game's loadable files, best first (by format preference, 48K builds
/// ahead of 128K/+2/+3).
fn candidates(agent: &ureq::Agent, id: &str) -> Result<Vec<Candidate>, Error> {
    let url = format!("{API}/games/{id}");
    let v = get_json(agent, &url)?;

    let mut out = Vec::new();
    if let Some(rels) = v["_source"]["releases"].as_array() {
        for rel in rels {
            if let Some(files) = rel["files"].as_array() {
                for f in files {
                    let path = match f["path"].as_str() {
                        Some(p) => p,
                        None => continue,
                    };
                    let lower = path.to_ascii_lowercase();
                    for (rank, ext) in FORMATS.iter().enumerate() {
                        if lower.ends_with(&format!(".{ext}.zip")) || lower.ends_with(&format!(".{ext}")) {
                            // Bias toward 48K builds: the core is a 48K, so a
                            // "128"/"+2"/"+3" file ranks below a plain one.
                            let key = (rank, is_128k(&lower) as usize);
                            out.push((key, (*ext).to_string(), path.to_string()));
                        }
                    }
                }
            }
        }
    }
    out.sort_by_key(|(key, _, _)| *key);
    Ok(out.into_iter().map(|((rank, _), ext, path)| (rank, ext, path)).collect())
}

/// Download `path` (trying each mirror) and extract the inner `.{ext}` file. The
/// archive serves files zipped; a few are stored uncompressed.
fn download_and_extract(agent: &ureq::Agent, path: &str, ext: &str) -> Result<Vec<u8>, Error> {
    let encoded = pct(path, true);
    let mut last = Error::NotFound(format!("no mirror served {path}"));
    for base in MIRRORS {
        match download(agent, &format!("{base}{encoded}")) {
            Ok(bytes) => {
                return if path.to_ascii_lowercase().ends_with(".zip") {
                    extract_from_zip(&bytes, ext)
                } else {
                    Ok(bytes)
                };
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn download(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    agent
        .get(url)
        .call()
        .map_err(|e| Error::Http(e.to_string()))?
        .into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| Error::Http(e.to_string()))?;
    Ok(bytes)
}

fn extract_from_zip(bytes: &[u8], ext: &str) -> Result<Vec<u8>, Error> {
    let mut zip =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| Error::Parse(e.to_string()))?;
    let suffix = format!(".{ext}");
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| Error::Parse(e.to_string()))?;
        if f.name().to_ascii_lowercase().ends_with(&suffix) {
            let mut data = Vec::new();
            f.read_to_end(&mut data).map_err(|e| Error::Parse(e.to_string()))?;
            return Ok(data);
        }
    }
    Err(Error::NotFound(format!("no .{ext} inside the archive")))
}

// --- title matching ---------------------------------------------------------

/// Heuristic: does this filename look like a 128K / +2 / +3 build? Those won't
/// run on the 48K core, so they rank below a plain file of the same format.
fn is_128k(lower_path: &str) -> bool {
    ["128", "+2", "+3", "plus2", "plus3"].iter().any(|m| lower_path.contains(m))
}

const STOPWORDS: [&str; 4] = ["the", "a", "an", "of"];

/// Lowercase alphanumeric tokens, punctuation stripped, stopwords dropped.
/// `"Daley Thompson's Decathlon"` → `["daley", "thompsons", "decathlon"]`.
fn normalize(s: &str) -> Vec<String> {
    s.split_whitespace()
        .map(|w| w.chars().filter(|c| c.is_ascii_alphanumeric()).flat_map(|c| c.to_lowercase()).collect::<String>())
        .filter(|t| !t.is_empty() && !STOPWORDS.contains(&t.as_str()))
        .collect()
}

/// True if `title` plausibly *is* the queried game: every query token appears as
/// a title token, or the title's squashed form contains the query's squashed
/// form (handles `"Skooldaze"` vs `"Skool Daze"`). Deliberately asymmetric — a
/// shorter title never matches a longer query, so `"Treasure Island"` is
/// rejected for `"Treasure Island Dizzy"`.
fn title_matches(title: &str, query_tokens: &[String]) -> bool {
    if query_tokens.is_empty() {
        return true;
    }
    let title_tokens = normalize(title);
    if query_tokens.iter().all(|q| title_tokens.contains(q)) {
        return true;
    }
    let tcat: String = title_tokens.concat();
    let qcat: String = query_tokens.concat();
    tcat.contains(&qcat)
}

/// Percent-encode for a URL, keeping `/` in paths when `slash` is set.
fn pct(s: &str, slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let keep = b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'_' | b'.' | b'~')
            || (slash && b == b'/');
        if keep {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_punctuation_and_stopwords() {
        assert_eq!(normalize("Daley Thompson's Decathlon"), ["daley", "thompsons", "decathlon"]);
        assert_eq!(normalize("The Lords of Midnight"), ["lords", "midnight"]);
        assert_eq!(normalize("Spy vs Spy"), ["spy", "vs", "spy"]);
    }

    #[test]
    fn title_match_is_precise() {
        let ti_dizzy = normalize("Treasure Island Dizzy");
        // The real game matches; the unrelated shorter title does not.
        assert!(title_matches("Treasure Island Dizzy", &ti_dizzy));
        assert!(!title_matches("Treasure Island", &ti_dizzy));

        // One-word vs two-word spelling.
        assert!(title_matches("Skooldaze", &normalize("Skool Daze")));
        assert!(title_matches("Skool Daze", &normalize("Skool Daze")));

        // A different game is rejected.
        assert!(!title_matches("Green Beret", &normalize("Jet Set Willy")));
    }

    #[test]
    fn pct_encodes_paths() {
        assert_eq!(pct("/a b/c(d).tap", true), "/a%20b/c%28d%29.tap");
        assert_eq!(pct("Spy vs Spy", false), "Spy%20vs%20Spy");
    }

    // Network-gated end-to-end fetch. Run with: cargo test -p wos -- --ignored
    #[test]
    #[ignore]
    fn fetch_jet_set_willy() {
        let g = fetch("Jet Set Willy").expect("fetch JSW");
        assert!(g.title.to_lowercase().contains("jet set willy"));
        assert_eq!(g.format, "tap");
        assert!(g.data.len() > 1000);
    }
}
