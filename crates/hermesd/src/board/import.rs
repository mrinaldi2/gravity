//! The one-shot import of the markdown backlog (H-020 §3 point 3, B6):
//! `artifacts/backlog.md` read into board entries. Parsing is pure; storing
//! is `db::board_import`. Rows keep their `H-nnn` ids; any other single id
//! (R1-I2, B1, UX-001) gets a new one and keeps the old as a `was:` label.
//! A row this can't map without guessing is skipped and reported, never
//! approximated.

use super::model::{ColumnCategory, ItemType, Platform, Size};

pub mod cli;
mod fields;
pub mod service;
#[cfg(test)]
mod tests;

use fields::{artifacts, hex_ids, item_type, platforms, size, state};

/// The backlog's id prefix: its rows keep their ids under it (H-017 §5.3).
pub const SOURCE_KEY: &str = "H";

/// The label that marks a renamed release-1/2 row (H-020 §3 point 3).
pub const RENAME_LABEL: &str = "rename";

/// The history note every imported item is created with; a re-run knows
/// its own items by it.
pub const IMPORT_NOTE: &str = "imported from backlog.md";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    /// An `H-nnn` row: the item keeps the id, with this sequence number.
    Kept(u32),
    /// Any other id: a new item id, the old one kept as the label `was:<id>`.
    Legacy(String),
}

/// One backlog entry as a board item.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub line: usize,
    pub source_id: String,
    pub key: Key,
    pub title: String,
    pub item_type: ItemType,
    pub category: ColumnCategory,
    pub platforms: Vec<Platform>,
    pub size: Option<Size>,
    pub labels: Vec<String>,
    pub description: String,
    /// The owner column's name, assigned when it is a bot of the project.
    pub owner: Option<String>,
    pub decisions: Vec<String>,
    pub tasks: Vec<String>,
    pub artifacts: Vec<String>,
    /// Fields that couldn't be mapped and were left unset.
    pub warnings: Vec<String>,
}

/// An entry left out, and why.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Skipped {
    pub line: usize,
    pub id: String,
    pub reason: String,
}

#[derive(Debug, Default)]
pub struct Parsed {
    pub entries: Vec<Entry>,
    pub skipped: Vec<Skipped>,
}

/// The table's columns, by header name.
struct Header {
    id: usize,
    title: usize,
    why: Option<usize>,
    platform: Option<usize>,
    size: Option<usize>,
    state: usize,
    owner: Option<usize>,
    decision: Option<usize>,
}

impl Header {
    fn read(cells: &[String]) -> Option<Self> {
        let at = |name: &str| {
            cells
                .iter()
                .position(|c| c.to_lowercase().starts_with(name))
        };
        Some(Header {
            id: at("id")?,
            title: at("title")?,
            why: at("why"),
            platform: at("platform"),
            size: at("size"),
            state: at("state")?,
            owner: at("owner"),
            decision: at("decision"),
        })
    }
}

fn cells(line: &str) -> Vec<String> {
    let inner = line.trim().trim_start_matches('|').trim_end_matches('|');
    inner.split('|').map(|c| c.trim().to_string()).collect()
}

fn is_rule(cells: &[String]) -> bool {
    cells
        .iter()
        .all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')))
}

/// The key a single id maps to, or why it can't be one item.
fn key_of(id: &str) -> Result<Key, String> {
    let single = !id.is_empty()
        && id.split('-').all(|part| {
            !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
    if !single {
        return Err(format!(
            "'{id}' isn't one plain item id (a range, a group or an odd character); add its items one by one"
        ));
    }
    if let Some(n) = id
        .strip_prefix(SOURCE_KEY)
        .and_then(|r| r.strip_prefix('-'))
    {
        if let Ok(seq) = n.parse::<u32>() {
            return Ok(Key::Kept(seq));
        }
    }
    Ok(Key::Legacy(id.to_string()))
}

/// Read every table row and `### <id> — <title>` section of the backlog.
pub fn parse(markdown: &str) -> Parsed {
    let mut out = Parsed::default();
    let mut header: Option<Header> = None;
    let lines: Vec<&str> = markdown.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let n = i + 1;
        i += 1;
        if line.trim_start().starts_with('|') {
            let row = cells(line);
            if is_rule(&row) {
                continue;
            }
            match &header {
                None => header = Header::read(&row),
                Some(h) => out.add(row_entry(h, &row, n)),
            }
            continue;
        }
        header = None;
        if let Some(heading) = line.strip_prefix("### ") {
            let start = i;
            while i < lines.len() && !lines[i].starts_with('#') && !lines[i].starts_with('|') {
                i += 1;
            }
            out.add(section_entry(heading, &lines[start..i], n));
        }
    }
    out
}

impl Parsed {
    fn add(&mut self, entry: Result<Entry, Skipped>) {
        match entry {
            Ok(e) => self.entries.push(e),
            Err(s) => self.skipped.push(s),
        }
    }
}

fn skipped(line: usize, id: &str, reason: impl Into<String>) -> Skipped {
    Skipped {
        line,
        id: id.to_string(),
        reason: reason.into(),
    }
}

/// An entry with what every source shares: key, labels, type and links.
fn base(line: usize, id: &str, title: &str, category: ColumnCategory) -> Result<Entry, Skipped> {
    let key = key_of(id).map_err(|reason| skipped(line, id, reason))?;
    if title.is_empty() {
        return Err(skipped(line, id, "it has no title"));
    }
    let labels = match &key {
        Key::Kept(_) => Vec::new(),
        Key::Legacy(old) if old.starts_with("R1-") || old.starts_with("R2-") => {
            vec![RENAME_LABEL.to_string(), format!("was:{old}")]
        }
        Key::Legacy(old) => vec![format!("was:{old}")],
    };
    Ok(Entry {
        line,
        source_id: id.to_string(),
        key,
        title: title.to_string(),
        item_type: item_type(title),
        category,
        platforms: Vec::new(),
        size: None,
        labels,
        description: String::new(),
        owner: None,
        decisions: Vec::new(),
        tasks: Vec::new(),
        artifacts: artifacts(title),
        warnings: Vec::new(),
    })
}

fn row_entry(h: &Header, row: &[String], line: usize) -> Result<Entry, Skipped> {
    let cell = |i: Option<usize>| i.and_then(|i| row.get(i)).map_or("", String::as_str);
    let id = cell(Some(h.id));
    let state_text = cell(Some(h.state));
    let category = state(state_text).ok_or_else(|| {
        skipped(
            line,
            id,
            format!("its state '{state_text}' names no board column"),
        )
    })?;
    let mut e = base(line, id, cell(Some(h.title)), category)?;
    let (platform_text, size_text) = (cell(h.platform), cell(h.size));
    let (list, unknown) = platforms(platform_text);
    e.platforms = list;
    if !unknown.is_empty() {
        e.warnings.push(format!(
            "platform '{platform_text}': no platform for {unknown:?}"
        ));
    }
    match size(size_text) {
        Ok(s) => e.size = s,
        Err(()) => e
            .warnings
            .push(format!("size '{size_text}' isn't S, M or L")),
    }
    let (why, owner) = (cell(h.why), cell(h.owner));
    e.decisions = hex_ids(cell(h.decision));
    e.tasks = hex_ids(owner);
    e.owner = owner
        .split(['·', '/'])
        .next()
        .map(str::trim)
        .filter(|o| !o.is_empty() && !matches!(*o, "—" | "-"))
        .map(str::to_string);
    for text in [why, state_text, owner] {
        for a in artifacts(text) {
            if !e.artifacts.contains(&a) {
                e.artifacts.push(a);
            }
        }
    }
    let mut d = String::new();
    if !why.is_empty() {
        d.push_str(&format!("## Why\n\n{why}\n\n"));
    }
    d.push_str(&format!(
        "## From backlog.md\n\n- Id: {id}\n- State: {state_text}\n"
    ));
    for (name, value) in [
        ("Platform", platform_text),
        ("Size", size_text),
        ("Owner / task", owner),
    ] {
        if !value.is_empty() {
            d.push_str(&format!("- {name}: {value}\n"));
        }
    }
    e.description = d;
    Ok(e)
}

/// `### H-040 — title` and its bullets. It names no state, so it is new
/// work: the inbox, as every new item starts (H-017 §3).
fn section_entry(heading: &str, body: &[&str], line: usize) -> Result<Entry, Skipped> {
    let (id, title) = heading
        .split_once(" — ")
        .or_else(|| heading.split_once(" - "))
        .map_or((heading.trim(), ""), |(a, b)| (a.trim(), b.trim()));
    let mut e = base(line, id, title, ColumnCategory::Inbox)?;
    let text = body.join("\n").trim().to_string();
    for a in artifacts(&text) {
        if !e.artifacts.contains(&a) {
            e.artifacts.push(a);
        }
    }
    e.description = format!("{text}\n\n## From backlog.md\n\n- Id: {id}\n- State: none given\n");
    e.warnings
        .push("no state given; placed in the inbox".to_string());
    Ok(e)
}
