use anyhow::{Context, Result};
use serde_json::json;
use std::time::Instant;
use t3_terminal::{Color, Terminal, Theme};

fn ticks() -> Result<u64> {
    let value = std::fs::read_to_string("/proc/self/stat")?;
    let fields: Vec<_> = value
        .rsplit_once(") ")
        .context("process stat")?
        .1
        .split_whitespace()
        .collect();
    Ok(fields[11].parse::<u64>()? + fields[12].parse::<u64>()?)
}

fn main() -> Result<()> {
    let hz = std::process::Command::new("getconf")
        .arg("CLK_TCK")
        .output()?;
    let hz: f64 = std::str::from_utf8(&hz.stdout)?.trim().parse()?;
    let theme = Theme {
        foreground: Color {
            r: 245,
            g: 245,
            b: 245,
        },
        background: Color::default(),
        cursor: Color {
            r: 180,
            g: 203,
            b: 255,
        },
        palette: None,
    };
    let started = Instant::now();
    let mut terminal = Terminal::new(120, 40, 8, 18, &theme)?;
    let cold = started.elapsed().as_secs_f64() * 1000.;
    let started = Instant::now();
    let second = Terminal::new(120, 40, 8, 18, &theme)?;
    let warm = started.elapsed().as_secs_f64() * 1000.;
    drop(second);
    let bytes = "hello 日本語 ".repeat(4000).into_bytes();
    terminal.write(&bytes)?;
    let before = ticks()?;
    let started = Instant::now();
    for _ in 0..2000 {
        terminal.write(&bytes)?;
    }
    let parse_seconds = started.elapsed().as_secs_f64();
    let parse_cpu_seconds = (ticks()? - before) as f64 / hz;
    let frames: Vec<_> = ['x', 'y']
        .map(|c| format!("\x1b[H{}", c.to_string().repeat(4800)))
        .into_iter()
        .collect();
    terminal.snapshot()?;
    let before = ticks()?;
    let started = Instant::now();
    let mut snapshot_seconds = 0.;
    for index in 0..200 {
        terminal.write(frames[index % 2].as_bytes())?;
        let started = Instant::now();
        let snapshot = terminal.snapshot()?;
        snapshot_seconds += started.elapsed().as_secs_f64();
        assert_eq!(snapshot.row_data.len(), 40);
        assert_eq!(snapshot.dirty_rows.len(), 40);
    }
    let frame_seconds = started.elapsed().as_secs_f64();
    let frame_cpu_seconds = (ticks()? - before) as f64 / hz;
    println!(
        "{}",
        json!({"cold_initialization_ms":cold,"warm_initialization_ms":warm,"parse_bytes":bytes.len()*2000,"parse_seconds":parse_seconds,"parse_cpu_seconds":parse_cpu_seconds,"parse_mib_per_second":bytes.len() as f64*2000./1048576./parse_seconds,"frames":200,"frame_seconds":frame_seconds,"frame_cpu_seconds":frame_cpu_seconds,"snapshot_mean_ms":snapshot_seconds/200.*1000.,"memory_bytes":terminal.memory_bytes()})
    );
    Ok(())
}
