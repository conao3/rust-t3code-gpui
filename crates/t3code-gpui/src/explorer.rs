use std::collections::{HashMap, HashSet};

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub directory: bool,
}

impl Entry {
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

fn parent(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

/// The entries directly in `directory` from a `projects.listEntries` result, folders first and
/// then by name, as upstream's explorer keeps only the direct children.
pub fn children(result: &Value, directory: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = result
        .get("entries")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|e| {
            let path = e.get("path")?.as_str()?;
            (parent(path) == directory).then(|| Entry {
                path: path.to_string(),
                directory: e.get("kind").and_then(Value::as_str) == Some("directory"),
            })
        })
        .collect();
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.name().to_lowercase().cmp(&b.name().to_lowercase()))
    });
    entries
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub entry: Entry,
    pub depth: usize,
    pub expanded: bool,
    pub loading: bool,
}

/// The tree as the lines to draw: each loaded directory's children under it while it is open.
pub fn lines(directories: &HashMap<String, Vec<Entry>>, expanded: &HashSet<String>) -> Vec<Line> {
    fn walk(
        directory: &str,
        depth: usize,
        directories: &HashMap<String, Vec<Entry>>,
        expanded: &HashSet<String>,
        out: &mut Vec<Line>,
    ) {
        for entry in directories
            .get(directory)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let open = entry.directory && expanded.contains(&entry.path);
            out.push(Line {
                entry: entry.clone(),
                depth,
                expanded: open,
                loading: open && !directories.contains_key(&entry.path),
            });
            if open {
                walk(&entry.path, depth + 1, directories, expanded, out);
            }
        }
    }
    let mut out = Vec::new();
    walk("", 0, directories, expanded, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn children_keep_the_direct_entries_folders_first() {
        let result = json!({"entries": [
            {"path": "src/main.rs", "kind": "file"},
            {"path": "README.md", "kind": "file"},
            {"path": "src", "kind": "directory"},
            {"path": "Cargo.toml", "kind": "file"},
        ], "truncated": false});
        let names: Vec<_> = children(&result, "").into_iter().map(|e| e.path).collect();
        assert_eq!(names, ["src", "Cargo.toml", "README.md"]);
        assert_eq!(children(&result, "src")[0].name(), "main.rs");
    }

    #[test]
    fn open_folders_show_their_children_indented() {
        let file = |p: &str| Entry {
            path: p.into(),
            directory: false,
        };
        let dir = |p: &str| Entry {
            path: p.into(),
            directory: true,
        };
        let directories = HashMap::from([
            ("".to_string(), vec![dir("src"), dir("docs"), file("a.md")]),
            ("src".to_string(), vec![file("src/main.rs")]),
        ]);
        let shown = |open: &[&str]| {
            let expanded = open.iter().map(|s| s.to_string()).collect();
            lines(&directories, &expanded)
                .into_iter()
                .map(|l| (l.entry.path, l.depth, l.loading))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            shown(&[]),
            [
                ("src".into(), 0, false),
                ("docs".into(), 0, false),
                ("a.md".into(), 0, false)
            ]
        );
        assert_eq!(
            shown(&["src", "docs"]),
            [
                ("src".into(), 0, false),
                ("src/main.rs".into(), 1, false),
                ("docs".into(), 0, true),
                ("a.md".into(), 0, false),
            ]
        );
    }
}
