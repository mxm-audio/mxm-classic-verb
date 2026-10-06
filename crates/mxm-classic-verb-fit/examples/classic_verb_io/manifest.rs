//! The factory-space manifest `classic_verb_generate` reads (`crates/mxm-classic-verb-fit/AGENTS.md`,
//! *The factory-space generator*). **It lives outside the repository**: it names response files, and
//! nothing that names them is committed.
//!
//! One space per line, four fields separated by single tabs — a permanent id, a display name, a family
//! and the response file's path. Blank lines and lines starting with `#` are ignored. The order of the
//! lines is the selector's order. With `→` standing for a tab:
//!
//! ```text
//! # id → name → family → response
//! small-room → Small room → room → D:\responses\a response.aif
//! ```

use std::path::PathBuf;

/// The families a factory space may belong to, as a manifest writes them, and the `SpaceFamily`
/// variant the generated module names for each.
pub const FAMILIES: [(&str, &str); 10] = [
    ("room", "Room"),
    ("ambience", "Ambience"),
    ("chamber", "Chamber"),
    ("hall", "Hall"),
    ("large space", "LargeSpace"),
    ("drum room", "DrumRoom"),
    ("plate", "Plate"),
    ("spring", "Spring"),
    ("strange", "Strange"),
    ("experimental", "Experimental"),
];

/// The selector's last position, which no factory space may take the id, name or variant of.
pub const LOADED_ID: &str = "loaded";
pub const LOADED_NAME: &str = "Loaded";

/// One factory space as the manifest lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The manifest line it came from, counting from one.
    pub line: usize,
    pub id: String,
    pub name: String,
    /// The family as the manifest writes it, and the generated module's variant for it.
    pub family: &'static str,
    pub family_variant: &'static str,
    pub response: PathBuf,
}

/// The Rust variant an id names: each hyphen-separated word capitalised, `small-room` → `SmallRoom`.
pub fn variant(id: &str) -> String {
    id.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

fn has_separator(text: &str) -> bool {
    text.contains('/') || text.contains('\\')
}

/// Lowercase ASCII letters and digits in hyphen-separated words, the first starting with a letter.
fn is_id(id: &str) -> bool {
    id.split('-').enumerate().all(|(index, word)| {
        !word.is_empty()
            && word
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            && (index > 0 || word.as_bytes()[0].is_ascii_lowercase())
    })
}

fn id_problem(id: &str) -> Option<String> {
    if id.is_empty() {
        Some("the id is empty".to_owned())
    } else if has_separator(id) {
        Some(format!("the id `{id}` contains a path separator"))
    } else if !is_id(id) {
        Some(format!(
            "the id `{id}` is not lowercase letters and digits in hyphen-separated words, starting with a letter"
        ))
    } else if id == LOADED_ID || variant(id) == LOADED_NAME {
        Some(format!(
            "the id `{id}` is the selector's `Loaded` position's"
        ))
    } else if variant(id) == "Self" {
        Some(format!(
            "the id `{id}` would name the variant `Self`, which is a keyword"
        ))
    } else {
        None
    }
}

fn name_problem(name: &str) -> Option<String> {
    if name.is_empty() {
        Some("the name is empty".to_owned())
    } else if has_separator(name) {
        Some(format!("the name `{name}` contains a path separator"))
    } else if name.chars().any(char::is_control) {
        Some(format!("the name `{name}` contains a control character"))
    } else if name.to_lowercase() == LOADED_NAME.to_lowercase() {
        Some(format!(
            "the name `{name}` is the selector's `Loaded` position's"
        ))
    } else {
        None
    }
}

/// Every row of a manifest, or every problem with it — each named with its line. **Nothing is
/// fitted from a manifest with any problem**, so a mistake costs no run.
pub fn parse(text: &str) -> Result<Vec<Row>, Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows: Vec<Row> = Vec::new();
    let mut problems = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let content = raw.trim_end_matches('\r');
        if content.trim().is_empty() || content.trim_start().starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = content.splitn(4, '\t').map(str::trim).collect();
        if fields.len() != 4 {
            problems.push(format!(
                "line {line}: expected four tab-separated fields (id, name, family, response file), found {}",
                fields.len()
            ));
            continue;
        }
        let (id, name, family, response) = (fields[0], fields[1], fields[2], fields[3]);
        let mut row_problems: Vec<String> = [id_problem(id), name_problem(name)]
            .into_iter()
            .flatten()
            .collect();
        let known = FAMILIES.iter().find(|(word, _)| *word == family);
        if known.is_none() {
            let words: Vec<&str> = FAMILIES.iter().map(|(word, _)| *word).collect();
            row_problems.push(format!(
                "the family `{family}` is not one of: {}",
                words.join(", ")
            ));
        }
        if response.is_empty() {
            row_problems.push("no response file is named".to_owned());
        }
        if !row_problems.is_empty() {
            problems.extend(
                row_problems
                    .into_iter()
                    .map(|problem| format!("line {line}: {problem}")),
            );
            continue;
        }
        let (family, family_variant) = *known.expect("checked above");
        rows.push(Row {
            line,
            id: id.to_owned(),
            name: name.to_owned(),
            family,
            family_variant,
            response: PathBuf::from(response),
        });
    }

    for (index, row) in rows.iter().enumerate() {
        for earlier in &rows[..index] {
            if earlier.id == row.id {
                problems.push(format!(
                    "line {}: the id `{}` repeats line {}",
                    row.line, row.id, earlier.line
                ));
            } else if variant(&earlier.id) == variant(&row.id) {
                problems.push(format!(
                    "line {}: the id `{}` names the variant `{}`, as line {}'s `{}` does",
                    row.line,
                    row.id,
                    variant(&row.id),
                    earlier.line,
                    earlier.id
                ));
            }
            if earlier.name.to_lowercase() == row.name.to_lowercase() {
                problems.push(format!(
                    "line {}: the name `{}` repeats line {}'s `{}`",
                    row.line, row.name, earlier.line, earlier.name
                ));
            }
        }
    }
    if rows.is_empty() && problems.is_empty() {
        problems.push("the manifest names no space".to_owned());
    }
    if problems.is_empty() {
        Ok(rows)
    } else {
        Err(problems)
    }
}
