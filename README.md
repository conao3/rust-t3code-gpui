# rust-t3code-gpui

A native desktop frontend for [T3 Code](https://github.com/pingdotgg/t3code), built with GPUI through [gpui-kit](https://github.com/longbridge/gpui-kit). It talks to unmodified `t3 serve` servers over T3's WebSocket protocol (orchestration protocol 2) and renders the sidebar, a thread's timeline and a composer.

Redraws are batched: stream updates land in the client state as they arrive, and the window repaints at most every 250 ms. A streaming assistant message is split into blocks at blank lines, so only its last block is re-laid out.

## Layout

- `crates/t3-client`: the RPC connection, the shell and thread projections, the live subscriptions with reconnect and `afterSequence` resume, and the command helpers. No UI dependency.
- `crates/t3code-gpui`: the window and the CLI.

## Setup

Each server needs a bearer token. On the server's host, with the server running:

```sh
t3 auth session issue --ttl 365d --label t3code-gpui --token-only
```

Put the servers in `~/.config/t3code-gpui/environments.json` (mode 600):

```json
{
  "environments": [
    { "label": "eos", "httpBaseUrl": "http://127.0.0.1:3774", "token": "..." },
    { "label": "helios", "httpBaseUrl": "https://helios.example.ts.net:8443", "token": "..." }
  ]
}
```

## Usage

```sh
nix run                                   # open the window
t3code-gpui environment list-environments
t3code-gpui environment check-environment --label eos
```

| Key | Action |
|---|---|
| Ctrl+Enter | Send the composer text (queued while the thread is working) |
| Alt+O | Open the next thread that is Done or waiting for input |

"Open in browser" opens the same thread in the server's web UI.

## Development

```sh
nix develop
make check        # nix fmt --ci, clippy, unit tests
make integration  # runs t3-client against a temporary `t3 serve` (needs `t3` on PATH)
```
