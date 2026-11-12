# AGENTS.md

## Conventions

- Rust edition 2024, `style_edition = "2024"`. Format with `nix fmt`.
- `t3-client` keeps protocol entities as `serde_json::Value` and reads only the fields it uses, so unknown fields and event types from newer servers pass through.
- UI state changes reach the window through `t3_client::session::Change`; the window coalesces them and redraws at most every `REDRAW_INTERVAL`.
- `gpui-kit` is pinned with `=` and updated by hand (Dependabot ignores `gpui-*`).

## Verification

- `nix develop --command make check` runs the formatter check, clippy with `-D warnings` and the unit tests. CI runs the same command and `nix build`.
- `make integration` starts a temporary `t3 serve` with `--base-dir` in a temp directory and exercises the shell stream, `thread.create`, the thread stream and `thread.settle`. Run it against the same `t3` version the servers use before upgrading T3.
