# T3 behavioral and visual comparison

Run inside `nix develop`. The reference is the actual T3 Code Electron 0.1.9 desktop, with upstream source revision `24e77473ecf41bc1f7142529c7911c1c1aae1d76`. Keep reference source and test artifacts in `.claude-dev/`; neither belongs in the application distribution.

`cdp.py` controls an isolated Electron renderer over a loopback Chrome DevTools Protocol endpoint. It locates controls through the accessibility tree and sends mouse/key/text input. `native.py` locates native controls through AT-SPI and sends X11 input. The scenario runner uses these same user input paths and observes the resulting accessible content and images.

Create two independent server fixtures from the same initial data. Each must have its own database, provider state, credentials and writable workspace. Point the original desktop at the first fixture with `T3CODE_HOME`, and point the GPUI configuration at the second. Use an isolated desktop profile (`XDG_CONFIG_HOME`) and test-owned provider homes. Keep fixture configuration files mode 600. Sending scenarios require a deterministic test provider.

Use a fresh session directory and Electron profile for each comparison, and restore server fixtures before scenarios that change server or workspace state. The private desktop launcher owns Xvfb, its window manager, Electron and the native accessibility session:

```sh
python3 tools/parity/desktop_session.py \
  --reference-binary "$(command -v t3code-desktop)" \
  --reference-home .claude-dev/parity/reference-home \
  --reference-profile .claude-dev/parity/run-001/electron-profile \
  --reference-port 38881 --cdp-port 39991 \
  --config .claude-dev/parity/native-environments.json \
  --session .claude-dev/parity/run-001/native-session.json
```

The native fixture server must already be running independently. Startup verifies authentication with the native CLI. The launcher rejects a configuration that points to the reference origin; the runner also compares the actual Electron bootstrap origin and both server identities. Origins and identities are retained in the report, without bearer or bootstrap tokens. The default native binary is `target/debug/t3code-gpui`; build it before launching. The GUI scenario uses a declared content viewport and theme. For a light scenario, initialize the native fixture's client settings with `gpuiAppearance: "light"` as well.

To operate an original desktop on an existing X11 display, start it with its isolated profile and these additional arguments:

```sh
t3code-desktop --remote-debugging-address=127.0.0.1 --remote-debugging-port=39991
```

Launch the native test application on a separate accessibility bus:

```sh
dbus-run-session -- python3 tools/parity/private_session.py \
  --config .claude-dev/parity/native-environments.json \
  --session .claude-dev/parity/native-session.json
```

The session file identifies the native process and private bus. The launcher owns and cleans up that application and its accessibility services. It enables accessibility only on its private bus.

Inspect or operate the running applications:

```sh
python3 tools/parity/cdp.py --endpoint http://127.0.0.1:39991 tree
python3 tools/parity/cdp.py --endpoint http://127.0.0.1:39991 click button 'New thread'
python3 tools/parity/native.py --session .claude-dev/parity/native-session.json tree
python3 tools/parity/native.py --session .claude-dev/parity/native-session.json click button 'New thread'
```

With both fixtures at a new-thread-ready state, run the shared input scenario:

```sh
python3 tools/parity/run.py \
  --endpoint http://127.0.0.1:39991 \
  --session .claude-dev/parity/native-session.json \
  --scenario tools/parity/draft.json \
  --output .claude-dev/parity/run-001
```

The shared draft scenario exercises text retention, sidebar toggling, permission selection and keyboard versus mouse focus. Checkpoints assert exact input values, checked state and focus, not just control presence. Each checkpoint saves the accessible tree and a screenshot after the scenario’s explicit `settle_ms` delay. The native capture waits for a stable painted frame after activating its window; an unstable checkpoint fails. This static-image delay is separate from input/streaming latency measurements. Input values are compared exactly, including newlines. `type` accepts printable text; control characters require explicit key or paste actions. The runner sizes both content areas to the declared viewport and excludes native window shadows/borders using accessible content bounds. Output includes action results, side-by-side images, pixel differences and `report.json`. Every action and visual checkpoint must pass for exit status zero. Missing controls, unreadable input values and missing screenshots fail. Large visual differences remain failures even when typing works. Inspect the actual images as well as numeric differences; the pixel metric detects differences and does not certify feature coverage.

Inventory the reference commands separately:

```sh
python3 tools/parity/audit_commands.py \
  --reference-root .claude-dev/parity/reference \
  --output .claude-dev/parity/commands.json
```

The inventory distinguishes missing dispatch from untested behavioral equivalence. Registration alone never certifies equivalence, so an unverified inventory returns a nonzero status. Dynamic `script.*.run` commands and custom patch commands require additional scenarios beyond the upstream static inventory.

The developer checks include driver regression tests. GUI comparison requires an X11 desktop, a window manager, the original installed desktop and both running fixtures. Performance measurements must record the rendering backend and use the same display and provider output for both applications.
