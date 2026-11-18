# T3 behavioral and visual comparison

## Independent slots

Run `env.py` inside `nix develop` on Linux. Build the native binary in the checkout that will run the comparison:

```sh
nix develop --command cargo build -j 4
nix develop --command python3 tools/parity/env.py up --slot 1
nix develop --command python3 tools/parity/env.py status --slot 1
nix develop --command python3 tools/parity/env.py theme --slot 1 --theme light
nix develop --command python3 tools/parity/env.py reset --slot 1
nix develop --command python3 tools/parity/env.py restart --slot 1
nix develop --command python3 tools/parity/env.py restart --slot 1 --target native
nix develop --command python3 tools/parity/env.py down --slot 1
```

Slots 1–100 reserve ports `42000 + 10*N` (reference server), `42001 + 10*N` (native server), `42002 + 10*N` (CDP), and Xvfb display `:(100+N)`. Occupied ports or displays cause startup to fail. Every checkout of the repository shares the slot namespace under the main worktree's `.claude-dev/parity/slots/N/`; the slot control lock prevents concurrent changes to the same slot. `status` remains available during startup and shutdown. Different slots can start and run scenarios concurrently.

`up` creates a fresh generation with separate server homes, databases, provider homes, Git workspaces, Electron profile, native client settings, private D-Bus sessions and Xvfb/xfwm4. Both server homes and workspaces are copied from one deterministic seed: an empty SQLite database and a Git repository with `main` and `feature/parity`. Each server initializes that database independently, then receives the same `UI parity` project (`11111111-1111-4111-8111-111111111111`) and mock model selection through the public API. The Electron desktop owns its reference server; the native server runs through `t3 serve`. Startup checks actual Electron origin, distinct environment IDs, independent credentials, authentication on both servers and native accessibility readiness.

`--native-binary PATH` selects the executable (default: this checkout's `target/debug/t3code-gpui`). `--t3-binary` and `--reference-binary` default to the installed 0.1.9 Nix paths on helios. `--reference-source PATH` selects the pinned original source (`24e77473ecf41bc1f7142529c7911c1c1aae1d76`); its default is the main worktree's `.claude-dev/scratch/parity/reference`. Only its deterministic Codex transcript fixture is read. Each generation copies that transcript before launch. The local `replay_codex.py` provider uses it to implement the protocol and emits the deterministic `Fixture output 000:` through `199:` response. `--stream-ms` controls its duration (default 15000). Every slot has its own wrapper and provider state. Mock threads have distinct provider thread IDs, and interrupting a turn leaves other concurrent turns running. The provider performs no paid requests.

The final `up` result includes the path of `manifest.json`. The manifest contains the CDP endpoint, native session path, origins/IDs and workspace/configuration paths. Pass it directly to the runner:

```sh
nix develop --command python3 tools/parity/run.py \
  --manifest /absolute/main-worktree/.claude-dev/parity/slots/1/manifest.json \
  --scenario tools/parity/draft.json \
  --output /absolute/main-worktree/.claude-dev/parity/slots/1/draft-dark-001
```

`draft.json` needs only the initial project and mock configuration. Use a scenario declaring the selected theme: `theme --theme light` changes both actual client settings through their Settings controls; it does not rewrite the scenario. `up --theme light` starts in light mode. `reset` stops the current generation and starts a new one from the initial seed, preserving launch options and the selected theme. It clears drafts, server changes and workspace changes by selecting new homes. Previous generations and reports remain available for inspection; use a fresh output directory for every scenario run.

`down` signals only process groups carrying this generation's ownership marker, checking process start times before signalling. It also handles descendants whose original launcher has exited, waits for termination, escalates to SIGKILL if needed, and checks that the reserved ports/display are released. `status` reports the live supervisor and owned processes, including orphaned groups after a supervisor crash. `up` refuses a slot with surviving processes; use `down` to recover it. Configuration and manifest files are mode 600, directories are private, and tokens stay in configuration files. Startup failures report the stage and error class; subprocess stdout/stderr is discarded, and native diagnostics live in the generation's `desktop/` directory.

`fixture_rpc.FixtureRpc` accepts one environment object read from the manifest's `configurations` paths. Scenario-specific fixture helpers can call the public RPC using that selected configuration. The working-transition seed/cleanup scripts remain scenario-specific: the generic slot seed provides an empty project, and `reset` restores that complete baseline. Token renewal, launch and restart preparation are handled by fresh slot generations. The scratch provider-theme style dump remains an investigation artifact; `theme` handles the reusable appearance change.

The PR, Git and right-panel scenarios prepare their own data with a `fixture` field through `panel_fixture.py`. Run `right-panel.json`, `diff-controls.json`, `git-commit.json`, `git-commit-selection.json`, `pull-requests-page.json`, `pull-requests-layout.json` and `pull-request-filters.json` after a fresh `up` or `reset`; no manual thread creation or workspace edits are required. Each fixture creates `Panel fixture` and two untracked text files independently on each server. It configures the isolated Git repositories with the fixture author, disabled commit signing and disabled hooks. Commit scenarios wait for the expected subject, remaining files and identical committed tree; differing wall-clock commit timestamps are not compared. Use a fresh generation for every scenario because commit scenarios change Git state.

`pull-request-review.json` requires `up --pr-rpc-fixture tools/parity/pr-rpc-fixture.json`. The synthetic PR timestamps are fixed in the future so relative labels remain `just now`; this scenario does not test timestamp aging. It enables review guides through the server settings and exercises the populated PR list, Summary, Guide navigation, review editing and submission against the local RPC fixture. This option intercepts PR requests on both owned servers and records them in each server home's `pr-rpc.jsonl`; it never submits a review to GitHub. Use `down` followed by ordinary `up` to return to the empty PR scenarios.

`project-script.json` creates and saves an action with a keybinding, executes it twice and checks that each client still has one terminal. `device-panel.json` checks initial setup, Cancel and reopening. `side-chat-panel.json` checks the surface frame; its native conversation body is an integration point for the conversation pane implementation. `thread-services.json` creates a paused automation and local HTTP servers at each slot server port plus 3, checks pause/resume, opens the existing automation editor, saves a renamed task and verifies its persisted title, schedule, thread and enabled state. These processes belong to the slot and are stopped by `down`.

`turn-diff.json` uses the mock provider to create a completed turn with a file change. Its helper waits for the provider turn to start after baseline capture before writing the file and for the run checkpoint before continuing, then records both `getTurnDiff` results in `fixture.json`. The scenario opens the individual file immediately after selecting Latest turn and then changes the comparison base. All these scenarios use `--manifest` alone after a fresh `up`.

The four `conversation-*-native.json` files now use the same Side chat launcher input on both applications; their filenames remain stable. Start each from a fresh generation with two empty mock threads named `Pane Alpha` and `Pane Beta` in `UI parity`. `conversation-panes-native.json` checks independent drafts and focus, empty-side-chat deletion and its exclusion from reopen. `conversation-workspace-native.json` checks that sidebar navigation changes the main conversation while preserving its side chat draft. `conversation-stream-native.json` opens both composers for the streaming measurement. `conversation-composer-native.json` uses a mock stream duration of 600000 ms, stashes and restores the main draft, verifies that Ctrl+S leaves the Side chat draft in place, sends both drafts, queues one follow-up per conversation and edits both queued messages. Check both server projections for their separate prompts and queue edits. An empty side chat is deleted on close, including when it has a local stash; reopening a nonempty side chat requires a completed or active run in that side chat.

Composer state, asynchronous completions and workspace selectors belong to the originating pane. Saved prompt stashes and drafts are keyed by pane ID; a reopened pane keeps its ID. File and skill tokens are restored through the same input-content conversion as ordinary draft loading. Input completeness is checked through the accessible editor value after 80 ASCII characters at a 10 ms XTest interval. To check streaming isolation, enable `T3CODE_GPUI_TRACE_CONVERSATIONS=1` for the isolated launch, record per-pane render counts over six seconds, stop one of two streaming threads, and require zero renders for its quiet pane during the next settled six-second interval. Functional acceptance uses action results, accessible content and server projections; image differences remain separate evidence. Streaming checkpoints omit screenshots because their painted content continues to change. Native focus actions wait for the requested editor to report focus before subsequent typing, and accessible traversal clears cached children before reading the current tree.

`composer-history-wrapped.json` needs an open, stopped `Composer queue fixture` with two user messages: `Prior prompt`, followed by twelve space-separated copies of `Wrapped prompt boundary`. It recalls the wrapped prompt from an empty composer, keeps it when Up moves within its visual lines, recalls the older prompt only from the first visual line, and leaves history with Down from the last visual line. Its final selectors explicitly preserve the empty Lexical newline versus the native empty value; they do not assert that the stopped-thread action buttons match. `composer-new-thread.json` checks the project heading and the projectless link through the scratch RPC. `composer-queue-collapse.json` needs a stopped `Composer queue fixture` with five queued messages on each independent server; it checks two visible rows, expansion to five and collapse back to two without sending or removing a message. `python3 tools/parity/composer_text.py RUN --output REPORT.json` compares every accessible name/value under the composer, new-thread heading, details popover and open menus in matching checkpoints. It normalizes whitespace and omits duplicate browser `InlineTextBox` nodes, retains both text sets and reports missing/extra strings; nonzero differences fail the audit. A passing interaction scenario does not imply zero text differences.

Selectors can include `ancestor: {role, name}` to select a control inside a specific accessible container. Both drivers retain ancestry, and the runner resolves the selected control's global occurrence before sending real input. This scopes repeated row actions such as Settle thread to the intended thread instead of relying on sidebar position.

`node tools/parity/snooze_oracle.mjs ORIGINAL_ROOT crates/t3code-gpui/tests/fixtures/snooze-oracle.json` executes the original Snooze state, scheduling, selection and custom-input functions. Its 407 states, 140 schedules with nested selections, and 2,100 custom inputs cover five IANA zones, spring/fall daylight-saving changes and the skipped Apia date. Preset labels use an explicit HH:mm adapter with en-US weekday labels. This calculation oracle does not certify native dialog controls or images.

`node tools/parity/sidebar_status_oracle.mjs ORIGINAL_ROOT crates/t3code-gpui/tests/fixtures/sidebar-status-oracle.json` executes the original status, receding, unread and work-duration functions: 864 status combinations, 144 receding conditions, 30 unread cases, 108 start-time cases and 11 elapsed durations. Explicit-null activity start times prevent fallback, and offset dates and invalid visit timestamps are included.

`sidebar-snooze-dialog.json` opens the first row's Snooze menu through keyboard focus and Enter, then checks Custom snooze and its Duration input. `sidebar-snooze-custom.json` adds Up/Down and pointer stepping, zero-duration validation, a two-hour Snooze, the next thread's focused composer and the Snoozed shelf count. Prepare two empty user-created threads in the `UI parity` project, in this creation order: `Next interaction fixture`, then `Snooze interaction fixture`, newer than the other active rows. Restore the Snooze row to an unsnoozed, unsettled state before replay. Observe `snoozedAt` and `snoozedUntil` through the server projection after the scenario to verify the stored duration. Platform selectors retain AT-SPI menu-item, toggle-button and notification roles; text entries use the runner's canonical `textbox` role. Screenshot comparisons retain the same strict thresholds as the other scenarios.

`sidebar-wake-settle.json` continues from the custom-Snooze result: it wakes the background row, settles the current row and checks the next composer's focus. `sidebar-snooze-expiry.json` starts from the same unsnoozed, unsettled fixture and enters a fractional duration of 0.001 hours, waits for Woke, then dismisses it. Verify afterwards that the stored last-visited time equals that wake's timestamp. These scenarios exercise actual GUI actions; they do not dispatch the operation under test through fixture setup.

`sidebar-shell.json` requires an empty user-created thread named `Model defaults fixture` in `UI parity`, with the stored model selection `{ "instanceId": "codex", "model": "gpt-5.5" }` and the fixture provider advertising only its default `gpt-6-luna` (`6 Luna`). It checks both the existing conversation and a new draft, along with the common sidebar controls. The installed web client resolves the unavailable stored slug to the advertised default. This fixture does not run a provider.

`python3 tools/parity/sidebar_text.py RUN --output REPORT.json` compares the sidebar and title bar's accessible name/value strings, following `settings_text.py`'s whitespace normalization, unique-string comparison and exclusion of duplicate `InlineTextBox` nodes. Reference scope comes from Threads/Thread breadcrumb ancestry; native scope comes from the application/sidebar bounds. It retains the home link name but excludes its branding children, the native window controls, and the title bar panel controls owned by the panel comparison. Each exclusion is listed in the report. This is a text audit of the captured states, not a claim about menus absent from those snapshots.

`sidebar-working-settle.json` requires an active mock run named `Working transition fixture` in `UI parity`, with Working collapsed and no settled threads. Use a stream duration longer than the comparison (for example 3600000 ms). The scenario expands Working, opens its row and activates the row's Settle button with Enter. The installed reference sends `orchestration.dispatchCommand` with `type: "thread.settle"`; the server rejects it with `Thread <id> is still running. Stop it before settling.` (`Orchestrator.ts`, the running-thread settlement guard). The current thread, Working count and empty Settled shelf must remain unchanged. Afterwards query `orchestration.getThreadProjection` on each independent server: `settledOverride` must remain null and the same run must remain running. Streaming checkpoints omit screenshots. A successful input event alone does not prove a successful settlement.

`sidebar-snooze-unit.json` opens the actual Unit select with Space, selects Minutes with Up and Enter, and commits the Snooze. Verify a two-minute duration in each independent server projection. The native select exposes named list items where Electron exposes options.

`sidebar-snooze-options.json` follows Edit snooze options, waits for the settings screen, adds a sixth option and changes its duration to 45 minutes. Start with the default five presets. Verify the six saved settings, including 45 minutes, on both servers. `sidebar-snooze-options-restore.json` removes that option, closes settings and checks the original composer. Compare the saved settings after removal as well as the GUI checkpoints.

`sidebar-state.json` opens the latest completed `Fixture response`, checks its composer and final `Fixture output 199:` line, then reopens it through keyboard focus and Enter. This is a live regression for the raw-shell conversion. Prepare the same completed replay fixture on both independent servers; select the requested theme through Settings before running the dark scenario or its light variant.

`node tools/parity/sidebar_state_oracle.mjs ORIGINAL_ROOT crates/t3code-gpui/tests/fixtures/sidebar-state-oracle.json` executes the original `presentThreadShell`, runtime/background, provider-stack and activity helpers. The 211 cases compare the normalized fields and the actual sidebar entries produced from raw server shells. Passing raw shell fields straight to the display helpers is a negative control: it disagrees in 157 stack cases and 46 activity cases. Dates use a UTC JS Date adapter; PR lists and subagent titles are outside the asserted fields and use explicit identity adapters. The fixture records these limits and source hashes.

`sidebar-providers.json` selects three non-running fixtures with different provider icons and checks the active composer's title after moving the pointer away from the sidebar. Create `Provider fixture codex_hub`, `Provider fixture kiro` and `Provider fixture codex` in each isolated `UI parity` project using the same public `thread.create` commands. Configure `codex_hub` as a disabled Codex instance named `Codex Hub` with accent `#1b4ed8`. Keep the ordinary Codex instance enabled to exercise the disabled instance's badge without badging the single enabled instance. No provider run is required. Generate display and timestamp expectations with `node tools/parity/provider_display_oracle.mjs ORIGINAL_ROOT crates/t3code-gpui/tests/fixtures/provider-display-oracle.json`; generate the original provider SVG variants with `node tools/parity/provider_assets.mjs ORIGINAL_ROOT crates/t3code-gpui/assets/providers`. The fixture records the exact source revision and hashes. Visual checks and performance measurements remain separate from these source oracles.

`sidebar-shelves-empty.json` checks the empty Settled control. `sidebar-shelves.json` opens the completed fixture response and checks pointer, Enter and Space activation, expanded state and child visibility for populated Working, Snoozed and Settled shelves. Prepare one private fixture in each shelf and collapse all three before running it. `sidebar-shelves-restored.json` checks the expanded shelves after restarting both clients with their saved profiles.

For slot comparisons, `env.py restart --slot N` restarts the two clients with the same profiles, homes, workspaces and server databases. Electron also restarts its owned reference server; the independent native server stays running. `--target native` restarts only GPUI, including after rebuilding its executable. The supervisor verifies process identity and slot ownership before signaling either client. Close reference dialogs before a two-client restart readiness check.

For comparisons using `desktop_session.py` directly, pass the same `--native-home` and `--native-state-home`, and explicitly use `--reuse-reference-profile` with the same reference profile. Session output directories must remain fresh. The isolated Electron profile uses `--password-store=basic` to avoid waiting for the desktop keyring service. Native focus and click preparation requests AT-SPI scrolling and rereads the target coordinates before sending pointer input.

`sidebar-add-project.json` checks source selection, folder browsing, Git URL entry and the clone destination step. It stops before cloning. `sidebar-working-settle.json` checks the running-thread rejection notification, its original error detail, copy feedback and dismissal, and the unchanged Working and Settled shelves.

`sidebar-history.json` checks draft text through Back and Forward, the home link, and repeated New thread actions. Prepare the `Model defaults fixture` thread before running it.

Run `node tools/parity/model_selection_oracle.mjs ORIGINAL_ROOT crates/t3code-gpui/tests/fixtures/model-selection-oracle.json` with Node 24 to regenerate the model-selection table. It executes the pinned original TypeScript for project provider enablement, hidden and custom model resolution, dynamic unavailable selections, aliases and draft carry overrides. The fixture records source hashes and the adapters for Effect collections, branded IDs and unrelated entry metadata; Rust compares all 1,460 cases. Provider usage-limit fallback and the complete composer provider picker are outside this table.

A checkpoint can specify `visible_regions`, with pixel `bounds` and `minimum_pixels`. This rejects blank captures by counting pixels that contrast with the region's most common color. It is independent of the full image comparison, whose thresholds remain unchanged.

Run inside `nix develop`. The reference is the actual T3 Code Electron 0.1.9 desktop, with upstream source revision `24e77473ecf41bc1f7142529c7911c1c1aae1d76`. Keep reference source and test artifacts in `.claude-dev/`; neither belongs in the application distribution.

`cdp.py` controls an isolated Electron renderer over a loopback Chrome DevTools Protocol endpoint. It locates controls through the accessibility tree and sends mouse/key/text input. `native.py` locates native controls through AT-SPI and sends X11 input. The scenario runner uses these same user input paths and observes the resulting accessible content and images.

`lineage-panel.json` compares the parent link in the actual details panel, the disabled merge control on an empty fork, and pointer/Enter navigation. `lineage.json` also checks the timeline divider using pointer, Enter and Space. Enter and Space send browser character events in addition to key down/up so the original browser performs its normal button activation.

The relationship graph oracle is generated from the pinned original TypeScript, retaining its source hash. Run `node tools/parity/lineage_oracle.mjs PATH/TO/threadRelationships.ts crates/t3code-gpui/tests/fixtures/lineage-oracle.json` with Node 24. Its only date adapter converts decoded UTC shell dates to JavaScript Date for `isDateTime`/`toEpochMillis`. Rust tests compare all 137 graph cases, including omitted parents, authoritative live shells, rolled-back subagents, status precedence, transfers and ordering. GUI controls, server effects and images are checked separately.

The same oracle also runs the original agent-name formatter and prompt truncation for 27 Unicode and path cases. `lineage-stop.json` needs a running `Lineage stop fixture` conversation with a running app-owned child titled `Subagent: /root/parity_stop` in both independent servers, and both Working shelves expanded. It checks the focused Stop button through Enter activation. Verify the server projections afterwards: both the subagent and child run must be interrupted while the parent run remains running. Recreate this fixture before another stopping scenario; an already stopped child invalidates the comparison.

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

The native fixture server must already be running independently. Startup verifies authentication with the native CLI. The launcher rejects a configuration that points to the reference origin; the runner also compares the actual Electron bootstrap origin and both server identities. Origins and identities are retained in the report, without bearer or bootstrap tokens. The default native binary is `target/debug/t3code-gpui`; build it before launching. The GUI scenario uses a declared content viewport and theme. Set the reference appearance through Settings and initialize the native fixture's client settings with the matching `gpuiAppearance`. Emulated system appearance does not override a saved explicit theme. The runner verifies the reference's applied color scheme before sending scenario input and records it in the report.

Pass `--display :0` to the desktop launcher to use an existing X11 desktop for physical GPU measurements. Select the measured release or packaged binary with `--native-binary`. Its application process groups belong to that comparison and are cleaned up with the launcher.

The isolated native session enables `T3CODE_GPUI_RENDERER_INFO` and captures the selected GPU, driver and software-emulation flag in `native.log`. `Native.renderer_info()` reads that startup record. Record the original browser's `SystemInfo.getInfo` data alongside it when comparing performance; opening a GPU device during adapter enumeration alone does not identify the selected renderer.

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

`workspace.json` opens and hides thread details, searches for `feature/parity`, checks out that branch and checks composer focus. Prepare both Git fixtures with identical commits, refs and files, including a `feature/parity` branch. Pass their paths with `--reference-workspace` and `--native-workspace`. The runner rejects shared Git stores or different initial snapshots, then saves and compares the final branch, HEAD, refs, status and file hashes. Each platform's native accessibility roles remain in the saved trees; selectors can explicitly identify equivalent native controls with `reference` and `native` fields on clicks and checkpoints.

`editor.json` selects Files and Cursor from thread details, verifies the preferred-editor button label and checks that selecting a menu item returns focus to its trigger. Use Linux fixtures advertising both editors. Selection updates the preference; the primary button opens the selected editor. Checkpoints require ordinary menu items and preserve the native role names in the result.

`messages.json` opens the deterministic fixture thread named `Fixture response` in `UI parity`, checks the rendered assistant text including Japanese, opens a new thread and returns to the fixture. Prepare the same completed response on both servers. Message documents expose the text of the parsed Markdown through AT-SPI; snapshots cover the currently rendered timeline chunks. `text_contains` checks actual text content within the selected nodes, while `value` continues to require an exact input value. The saved images independently check message layout.

`lineage.json` opens the first `Fixture response fork` in `UI parity`, checks its `Open source conversation` control, follows that control to the source composer, and returns to the fork. Prepare the fork independently on both servers through `Fork from this response`. The timeline uses the server's visible item order, including inherited history; source links must navigate rather than only display a divider.

`terminal.json` opens that completed fixture, toggles its terminal, splits horizontally and vertically, hides and reopens the drawer, and checks that all three panes and the original response remain present. Use a fresh profile with no saved terminals. Terminal checkpoints count the original input textareas and native terminal surfaces independently; a toolbar alone does not prove that panes exist. The scenario does not certify terminal contents, mouse reporting, IME or rendering parity.

The native text driver keeps ASCII input at 20 ms and gives Unicode keymap substitutions 300 ms. Fast consecutive XTest Unicode substitutions can lose characters in both Electron and GPUI; input completeness must be checked in the application. This cadence belongs to functional input tests and must not be presented as application latency or used as the typing performance workload.

Both drivers also accept `pointer move|down|up X Y --button left|middle|right`, with coordinates relative to the content viewport. A shared scenario can use `{"action":"pointer","kind":"down","x":300,"y":650}` and matching move/up steps for a drag. The original driver preserves pressed-button state within a connection; use a scenario or one Python connection for a complete gesture. Verify terminal mouse protocols using a raw PTY input recorder as well as the resulting screen.

`focus ROLE NAME` and the scenario action `focus` request focus through DOM/CDP or the native OS accessibility interface. They do not inject a pointer click, select text or change a value. Use a subsequent focused checkpoint to confirm the result. This is useful when preparing a typing workload on a shared X11 desktop, where a pointer click can arrive after a later keyboard event. Record this setup explicitly; it does not certify physical click timing. The `click` and `pointer` actions remain separate tests of pointer input.

Use `modifier("ctrl", True)` and `modifier("ctrl", False)` on either Python driver to hold and release a modifier around checkpoints or other input. Scenarios accept `{"action":"modifier","key":"ctrl","held":true}`. Supported keys are `ctrl`, `alt`, `shift` and `super`. A `press("1")` while Control is held sends Ctrl+1; it preserves the held state. Scenario completion and failure release tracked modifiers. Inspect images for shortcut badges: the original marks their text as hidden from accessibility.

Inventory the reference commands separately:

```sh
python3 tools/parity/audit_commands.py \
  --reference-root .claude-dev/parity/reference \
  --output .claude-dev/parity/commands.json
```

The inventory distinguishes missing dispatch from untested behavioral equivalence. Registration alone never certifies equivalence, so an unverified inventory returns a nonzero status. Dynamic `script.*.run` commands and custom patch commands require additional scenarios beyond the upstream static inventory.

The developer checks include driver regression tests. GUI comparison requires an X11 desktop, a window manager, the original installed desktop and both running fixtures. Performance measurements must record the rendering backend and use the same display and provider output for both applications.

`settings-text-pages.json` visits every visible Settings page in the pinned desktop reference. Run `python3 tools/parity/settings_text.py RUN_DIRECTORY --output REPORT.json` after capturing both applications. The audit collects accessible names and text values inside the Settings dialog, normalizes whitespace, and compares unique strings per checkpoint; repeated wrappers do not inflate counts. Missing native checkpoints fail. The report lists every missing and extra string, including numeric values and selected options. It does not certify roles, behavior, hidden controls, or layout. `settings-advanced-search.json` also opens Advanced, changes the skills preference, searches for it and exercises Select Home/End. Inspect its paired screenshots and persisted settings separately.

`usage_chart_oracle.mjs ORIGINAL_ROOT OUTPUT` extracts the pinned reference's scale and monotone tangent functions and records their results for the native chart tests. The chart retains independent provider areas rather than stacking provider values.

`env.py up --slot N --pr-rpc-fixture tools/parity/pr-rpc-fixture.json` fixes PR RPC responses in both isolated servers. The Node preload intercepts server WebSocket requests before dispatch; every `pullRequests.*` method is either answered by the JSON fixture or rejected. It never passes an unconfigured PR method to a real host. Each server records PR method names and inputs in its own `home/pr-rpc.jsonl`, including review submissions. The frontend bundle and input paths remain unchanged. The fixture is read on each request, so separate empty, running, failed, stale and completed guide states can be selected without changing application state. Enable `reviewGuidesEnabled` through the isolated server settings for Guide scenarios. The fixture certifies client behavior and payloads, not a real forge's review processing.

### Multiple hosts inside one slot

`host_fixture.py` adds one independent mock server per application on the slot's
base port + 3 and + 4. Each application then has two environments and two copies
of the `Multi host fixture` project, grouped by their identical repository
origin. Environment identities, tokens, provider homes and working directories
remain independent. The helper rejects an unowned manifest, a changed generation
or an allocation that does not belong to that slot.

The Electron connection catalog requires secure storage. Start a fresh slot with
`keyring_reference.py` as its reference executable. This requires
`gnome-keyring-daemon` on PATH and creates an empty-password fixture keyring on
Electron's private D-Bus, with data and control files in the slot generation.
Its processes carry the slot owner marker and are stopped by `env.py down`.

```sh
nix develop --command python3 tools/parity/env.py up --slot 5 \
  --native-binary target/debug/t3code-gpui \
  --reference-binary tools/parity/keyring_reference.py
nix develop --command python3 tools/parity/host_fixture.py \
  --manifest /path/to/main/.claude-dev/parity/slots/5/manifest.json \
  --primary-label helios
```

Use your allocated slot number and main checkout path. `--primary-label` is the
primary machine name displayed by Electron; the helper uses that name for the
native connection too. `--label` chooses the secondary name (default
`parity-secondary`). The helper registers the Electron connection catalog,
updates the native configuration, and restarts only the native client. It binds
`composer.cycleHost` to `Ctrl+Alt+H` on all four fixture servers. Host switching,
draft editing and submission must then be tested through normal UI input.

The helper writes `multi-host/fixture.json` with the environment identities,
workspace paths and shortcut. The adjacent `configurations.json` contains fixture
tokens and is private. Run the helper once per fresh generation; `env.py reset`
creates a new generation without the additional hosts.

Select `Multi host fixture` in both clients before running `composer-host.json`.
It uses the project's `New thread in Multi host fixture` button, switches hosts
with the selector and `Ctrl+Alt+H`, checks that the draft text follows the chosen
host, and submits on the secondary host. The global `New thread` project palette
is a separate behavior: do not count it as verified by this scenario. Compare the
secondary servers' message/run state as well as the primary servers' state.
