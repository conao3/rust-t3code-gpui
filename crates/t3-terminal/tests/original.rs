use anyhow::{Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use t3_terminal::{Point, Terminal, Theme};

fn point(value: &Value) -> Result<Point> {
    Ok(serde_json::from_value(value.clone())?)
}

fn difference(actual: &Value, expected: &Value, path: String) -> Option<String> {
    match (actual, expected) {
        (Value::Array(left), Value::Array(right)) if left.len() == right.len() => left
            .iter()
            .zip(right)
            .enumerate()
            .find_map(|(index, (left, right))| difference(left, right, format!("{path}[{index}]"))),
        (Value::Object(left), Value::Object(right)) if left.len() == right.len() => {
            right.iter().find_map(|(key, right)| {
                difference(
                    left.get(key).unwrap_or(&Value::Null),
                    right,
                    format!("{path}.{key}"),
                )
            })
        }
        _ if actual == expected => None,
        _ => Some(format!("{path}: actual={actual}, expected={expected}")),
    }
}

#[test]
fn matches_original_terminal_operations() -> Result<()> {
    let oracle: Value = serde_json::from_str(include_str!("fixtures/original-terminal.json"))?;
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("../assets/ghostty-vt.wasm"))
        ),
        oracle["wasmSha256"].as_str().unwrap()
    );
    let version = include_str!("../assets/Ghostty-VERSION").trim().as_bytes();
    assert!(
        include_bytes!("../assets/ghostty-vt.wasm")
            .windows(version.len())
            .any(|window| window == version)
    );
    let theme: Theme = serde_json::from_value(oracle["theme"].clone())?;
    for case in oracle["cases"].as_array().unwrap() {
        let mut term = Terminal::new(
            case["cols"].as_u64().unwrap() as u16,
            case["rows"].as_u64().unwrap() as u16,
            8,
            18,
            &theme,
        )?;
        for (index, op) in case["ops"].as_array().unwrap().iter().enumerate() {
            let actual = if let Some(text) = op.get("write") {
                term.write(text.as_str().unwrap().as_bytes())?;
                Value::Null
            } else if let Some(bytes) = op.get("writeBytes") {
                term.write(
                    &bytes
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|byte| byte.as_u64().unwrap() as u8)
                        .collect::<Vec<_>>(),
                )?;
                Value::Null
            } else if let Some(text) = op.get("writeSync") {
                json!(term.write_with_sync_end(text.as_str().unwrap().as_bytes())?)
            } else if op.get("synchronized").is_some() {
                json!(term.mode(2026)?)
            } else if op.get("snapshot").is_some() {
                serde_json::to_value(term.snapshot()?)?
            } else if let Some(text) = op.get("resetAndWrite") {
                term.reset_and_write(text.as_str().unwrap().as_bytes())?;
                Value::Null
            } else if op.get("replies").is_some() {
                json!(
                    term.take_replies()
                        .iter()
                        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                        .collect::<Vec<_>>()
                )
            } else if let Some(rows) = op.get("scroll") {
                term.scroll(rows.as_i64().unwrap() as i32)?;
                Value::Null
            } else if op.get("scrollToBottom").is_some() {
                term.scroll_to_bottom()?;
                Value::Null
            } else if op.get("selectAll").is_some() {
                serde_json::to_value(term.select_all()?)?
            } else if op.get("selectionText").is_some() {
                json!(term.selection_text()?)
            } else if op.get("selectionRange").is_some() {
                serde_json::to_value(term.selection_range()?)?
            } else if let Some(coords) = op.get("selectWord") {
                serde_json::to_value(term.select_word(Point {
                    x: coords[0].as_u64().unwrap() as u32,
                    y: coords[1].as_u64().unwrap() as u32,
                })?)?
            } else if let Some(coords) = op.get("selectLine") {
                serde_json::to_value(term.select_line(Point {
                    x: coords[0].as_u64().unwrap() as u32,
                    y: coords[1].as_u64().unwrap() as u32,
                })?)?
            } else if let Some(points) = op.get("setSelection") {
                term.set_selection(
                    point(&points[0])?,
                    point(&points[1])?,
                    points[0]["tag"].as_u64().unwrap_or(1) as u32,
                    points[1]["tag"].as_u64().unwrap_or(1) as u32,
                )?;
                Value::Null
            } else if op.get("clearSelection").is_some() {
                term.clear_selection()?;
                Value::Null
            } else if let Some(coords) = op.get("hyperlinkAt") {
                json!(term.hyperlink_at(Point {
                    x: coords[0].as_u64().unwrap() as u32,
                    y: coords[1].as_u64().unwrap() as u32
                })?)
            } else if let Some(size) = op.get("resize") {
                term.resize(
                    size[0].as_u64().unwrap() as u16,
                    size[1].as_u64().unwrap() as u16,
                    8,
                    18,
                )?;
                Value::Null
            } else if op.get("encodeKey").is_some() {
                json!(String::from_utf8_lossy(&term.encode_key(
                    &serde_json::from_value(op["nativeKey"].clone())?
                )?))
            } else if let Some(text) = op.get("encodePaste") {
                json!(String::from_utf8_lossy(
                    &term.encode_paste(text.as_str().unwrap())?
                ))
            } else if let Some(mouse) = op.get("encodeMouse") {
                json!(String::from_utf8_lossy(
                    &term.encode_mouse(&serde_json::from_value(mouse.clone())?)?
                ))
            } else {
                bail!("Unknown original operation {op}");
            };
            if let Some(difference) = difference(&actual, &case["results"][index], String::new()) {
                bail!("{} operation {index} {op}: {difference}", case["name"]);
            }
        }
    }
    Ok(())
}

#[test]
fn invalid_selection_coordinates_release_guest_allocations() -> Result<()> {
    let theme: Theme = serde_json::from_value(
        json!({"foreground":{"r":245,"g":245,"b":245},"background":{"r":0,"g":0,"b":0},"cursor":{"r":180,"g":203,"b":255}}),
    )?;
    let mut terminal = Terminal::new(12, 4, 8, 18, &theme)?;
    terminal.write(b"selected text")?;
    let start = Point { x: 0, y: 0 };
    let outside = Point { x: 0, y: 1_000_000 };
    for _ in 0..100 {
        assert!(terminal.set_selection(start, outside, 1, 1).is_err());
    }
    let memory = terminal.memory_bytes();
    for _ in 0..20_000 {
        assert!(terminal.set_selection(start, outside, 1, 1).is_err());
    }
    assert_eq!(terminal.memory_bytes(), memory);
    terminal.select_all()?;
    assert_eq!(terminal.selection_text()?, "selected text");
    Ok(())
}
