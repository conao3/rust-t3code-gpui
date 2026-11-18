use gpui_kit::component::diff::{DiffFile, DiffParseOptions};

pub(super) struct Excerpt {
    pub files: Vec<DiffFile>,
    pub shown: usize,
    pub total: usize,
}

pub(super) fn extract(
    patch: &str,
    path: &str,
    old: bool,
    start: usize,
    end: usize,
) -> Option<Excerpt> {
    let header = regex::Regex::new(r"^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@").ok()?;
    for section in patch.strip_prefix("diff --git ")?.split("\ndiff --git ") {
        let section = format!("diff --git {section}");
        let files =
            DiffFile::parse_with(&section, DiffParseOptions::new().truncated_hunks(true)).ok()?;
        if !files.iter().any(|file| file.path().as_ref() == path) {
            continue;
        }
        let mut previous = String::new();
        let mut modified = String::new();
        let mut rows = Vec::new();
        let mut old_line = 0;
        let mut new_line = 0;
        let mut hunk = 0;
        for line in section.lines() {
            if line.starts_with("--- ") && hunk == 0 {
                previous = line.to_string();
            } else if line.starts_with("+++ ") && hunk == 0 {
                modified = line.to_string();
            } else if let Some(capture) = header.captures(line) {
                old_line = capture[1].parse().ok()?;
                new_line = capture[2].parse().ok()?;
                hunk += 1;
            } else if hunk > 0 {
                let (has_old, has_new) = match line.as_bytes().first() {
                    Some(b' ') => (true, true),
                    Some(b'-') => (true, false),
                    Some(b'+') => (false, true),
                    _ => continue,
                };
                rows.push((hunk, old_line, new_line, has_old, has_new, line));
                old_line += usize::from(has_old);
                new_line += usize::from(has_new);
            }
        }
        let selected = rows
            .iter()
            .enumerate()
            .filter(|(_, (_, a, b, has_old, has_new, _))| {
                if old {
                    *has_old && (start..=end).contains(a)
                } else {
                    *has_new && (start..=end).contains(b)
                }
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let mut first = *selected.first()?;
        let mut last = *selected.last()?;
        if !old && !rows[first].3 {
            while first > 0
                && rows[first - 1].3
                && !rows[first - 1].4
                && rows[first - 1].0 == rows[first].0
            {
                first -= 1;
            }
        }
        if old && !rows[last].4 {
            while last + 1 < rows.len()
                && !rows[last + 1].3
                && rows[last + 1].4
                && rows[last + 1].0 == rows[last].0
            {
                last += 1;
            }
        }
        let shown = &rows[first..=last];
        let mut output = format!("diff --git a/{path} b/{path}\n{previous}\n{modified}\n");
        let mut offset = 0;
        while offset < shown.len() {
            let current = shown[offset].0;
            let length = shown[offset..]
                .iter()
                .take_while(|row| row.0 == current)
                .count();
            let group = &shown[offset..offset + length];
            let old_count = group.iter().filter(|row| row.3).count();
            let new_count = group.iter().filter(|row| row.4).count();
            output.push_str(&format!(
                "@@ -{},{} +{},{} @@\n",
                if old_count == 0 {
                    group[0].1.saturating_sub(1)
                } else {
                    group[0].1
                },
                old_count,
                if new_count == 0 {
                    group[0].2.saturating_sub(1)
                } else {
                    group[0].2
                },
                new_count
            ));
            for row in group {
                output.push_str(row.5);
                output.push('\n');
            }
            offset += length;
        }
        return Some(Excerpt {
            files: DiffFile::parse_with(&output, DiffParseOptions::new().truncated_hunks(true))
                .ok()?,
            shown: shown.len(),
            total: rows.len(),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = "diff --git a/example.rs b/example.rs\n--- a/example.rs\n+++ b/example.rs\n@@ -10,3 +10,3 @@\n same\n-old\n+new\n end\n@@ -30,1 +30,1 @@\n-before\n+after\n";

    #[test]
    fn excerpts_keep_the_selected_side_and_original_line_numbers() {
        let removed = extract(PATCH, "example.rs", true, 11, 11).unwrap();
        assert_eq!((removed.shown, removed.total), (2, 6));
        assert_eq!(removed.files[0].deletions(), 1);
        assert_eq!(removed.files[0].additions(), 1);
        let added = extract(PATCH, "example.rs", false, 11, 11).unwrap();
        assert_eq!((added.shown, added.total), (2, 6));
        assert_eq!(added.files[0].additions(), 1);
        assert_eq!(added.files[0].deletions(), 1);
        let across = extract(PATCH, "example.rs", false, 11, 30).unwrap();
        assert_eq!((across.shown, across.total), (5, 6));
        assert!(extract(PATCH, "example.rs", false, 15, 20).is_none());
        assert!(extract(PATCH, "absent.rs", false, 10, 30).is_none());
    }
}
