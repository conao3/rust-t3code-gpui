use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Markdown(String),
    Table(Vec<Vec<String>>),
}

fn cells(line: &str) -> Vec<String> {
    let line = line.trim();
    let line = line.strip_prefix('|').unwrap_or(line);
    let line = line.strip_suffix('|').unwrap_or(line);
    line.split('|').map(|c| c.trim().to_string()).collect()
}

fn is_separator(line: &str) -> bool {
    let row = cells(line);
    !row.is_empty()
        && row.iter().all(|c| {
            let c = c.trim_matches(':');
            !c.is_empty() && c.chars().all(|ch| ch == '-')
        })
}

/// The markdown with each GFM table taken out as its rows (the header row first, without the
/// separator line), so the preview can sort them. Tables inside code fences stay markdown.
pub fn split(markdown: &str) -> Vec<Segment> {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut segments = Vec::new();
    let mut text = String::new();
    let mut in_fence = false;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            in_fence = !in_fence;
        }
        let starts_table = !in_fence
            && line.trim_start().starts_with('|')
            && lines.get(i + 1).is_some_and(|next| is_separator(next));
        if starts_table {
            if !text.is_empty() {
                segments.push(Segment::Markdown(std::mem::take(&mut text)));
            }
            let mut rows = vec![cells(line)];
            i += 2;
            while i < lines.len() && lines[i].trim_start().starts_with('|') {
                rows.push(cells(lines[i]));
                i += 1;
            }
            segments.push(Segment::Table(rows));
            continue;
        }
        text.push_str(line);
        text.push('\n');
        i += 1;
    }
    if !text.is_empty() {
        segments.push(Segment::Markdown(text));
    }
    segments
}

/// The number a cell holds, read past thousands separators, currency signs, units and the
/// minus signs `-`, `−` and `▲` (a Japanese negative mark).
pub fn number(cell: &str) -> Option<f64> {
    let cell = cell.trim();
    let (negative, rest) = match cell.chars().next()? {
        '-' | '−' | '▲' | '△' => (true, &cell[cell.chars().next()?.len_utf8()..]),
        _ => (false, cell),
    };
    let digits: String = rest
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
        .filter(|c| *c != ',')
        .collect();
    let prefix: String = rest.chars().take_while(|c| !c.is_ascii_digit()).collect();
    if digits.is_empty() || prefix.chars().any(char::is_alphanumeric) {
        return None;
    }
    let value: f64 = digits.parse().ok()?;
    Some(if negative { -value } else { value })
}

/// Orders cells for sorting: numbers by value before text by its characters, empty cells last.
pub fn compare(a: &str, b: &str) -> Ordering {
    match (a.trim().is_empty(), b.trim().is_empty()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        _ => {}
    }
    match (number(a), number(b)) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a.cmp(b),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Ascending(usize),
    Descending(usize),
}

impl Sort {
    /// The next state after a click on `column`: ascending, then descending, then the file's order.
    pub fn next(current: Option<Sort>, column: usize) -> Option<Sort> {
        match current {
            Some(Sort::Ascending(c)) if c == column => Some(Sort::Descending(column)),
            Some(Sort::Descending(c)) if c == column => None,
            _ => Some(Sort::Ascending(column)),
        }
    }
}

/// The body rows (after the `header_rows` first rows) in the sort's order. Empty cells stay last
/// either way.
pub fn sorted(rows: &[Vec<String>], header_rows: usize, sort: Option<Sort>) -> Vec<Vec<String>> {
    let mut body: Vec<Vec<String>> = rows.iter().skip(header_rows).cloned().collect();
    let cell = |row: &Vec<String>, column: usize| row.get(column).cloned().unwrap_or_default();
    match sort {
        Some(Sort::Ascending(column)) => {
            body.sort_by(|a, b| compare(&cell(a, column), &cell(b, column)))
        }
        Some(Sort::Descending(column)) => body.sort_by(|a, b| {
            let (x, y) = (cell(a, column), cell(b, column));
            match (x.trim().is_empty(), y.trim().is_empty()) {
                (false, false) => compare(&y, &x),
                _ => compare(&x, &y),
            }
        }),
        None => {}
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_come_out_of_the_markdown_but_not_out_of_fences() {
        let md = "# T\n\n| a | b |\n|---|:--:|\n| 1 | x |\n| 2 | y |\nafter\n```\n| no | table |\n|---|---|\n```\n";
        let segments = split(md);
        assert_eq!(segments[0], Segment::Markdown("# T\n\n".into()));
        assert_eq!(
            segments[1],
            Segment::Table(vec![
                vec!["a".into(), "b".into()],
                vec!["1".into(), "x".into()],
                vec!["2".into(), "y".into()],
            ])
        );
        assert!(matches!(&segments[2], Segment::Markdown(rest) if rest.contains("| no | table |")));
    }

    #[test]
    fn numbers_are_read_past_separators_signs_and_units() {
        assert_eq!(number("1,234"), Some(1234.));
        assert_eq!(number("¥12,000"), Some(12000.));
        assert_eq!(number("▲300"), Some(-300.));
        assert_eq!(number("−2.5 kg"), Some(-2.5));
        assert_eq!(number("abc 12"), None);
        assert_eq!(number(""), None);
    }

    #[test]
    fn sorting_puts_numbers_first_and_empty_cells_last_in_both_directions() {
        let rows: Vec<Vec<String>> = [["h"], ["b"], ["10"], [""], ["2"], ["a"]]
            .iter()
            .map(|r| r.iter().map(|c| c.to_string()).collect())
            .collect();
        let col =
            |rows: Vec<Vec<String>>| rows.into_iter().map(|r| r[0].clone()).collect::<Vec<_>>();
        assert_eq!(
            col(sorted(&rows, 1, Some(Sort::Ascending(0)))),
            ["2", "10", "a", "b", ""]
        );
        assert_eq!(
            col(sorted(&rows, 1, Some(Sort::Descending(0)))),
            ["b", "a", "10", "2", ""]
        );
        assert_eq!(col(sorted(&rows, 1, None)), ["b", "10", "", "2", "a"]);
        assert_eq!(col(sorted(&rows, 2, None)), ["10", "", "2", "a"]);
        assert_eq!(Sort::next(None, 1), Some(Sort::Ascending(1)));
        assert_eq!(
            Sort::next(Some(Sort::Ascending(1)), 1),
            Some(Sort::Descending(1))
        );
        assert_eq!(Sort::next(Some(Sort::Descending(1)), 1), None);
        assert_eq!(
            Sort::next(Some(Sort::Descending(1)), 0),
            Some(Sort::Ascending(0))
        );
    }
}
