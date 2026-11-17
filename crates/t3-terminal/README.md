# Terminal state and input encoding

This crate runs the same pinned Ghostty VT module as the reference T3 Code desktop. GPUI owns the terminal view; this crate owns terminal state, render snapshots, selection, keyboard/mouse encoding and terminal-generated PTY replies.

The module is vendored in `assets/ghostty-vt.wasm` (SHA-256 `51b016a6aa3c29ead71c7c8acf8c01d43b064bae19957a9c9f9e2bf469267629`). Its embedded Ghostty revision is `9f62873bf195e4d8a762d768a1405a5f2f7b1697`. The unchanged upstream MIT license and version are beside it. The reference artifact and browser adapter come from T3 Code source revision `24e77473ecf41bc1f7142529c7911c1c1aae1d76`, under the MIT notice in the repository's `THIRD_PARTY_NOTICES.md`. The Rust adapter follows `apps/web/src/terminal/ghostty/{core,runtime}.ts`. Updating the module requires regenerating the reference artifact with that project's `apps/web/scripts/build-libghostty-wasm.sh`, checking its native Ghostty pin, and renewing the oracle fixtures.

A process caches the compiled Wasmtime module. Initial compilation uses at most four workers with Cranelift speed optimization. Each terminal owns its instance and memory, so dropping it releases all handles and allocations together. Initialize and operate it on the terminal worker. Rendering consumes immutable snapshots with shared unchanged rows. Each store limits linear memory to 256 MiB. Scrollback is 10,000 rows, matching the reference.

`reset_and_write` replays server history while suppressing generated PTY replies. Live writes expose those replies through `take_replies`. `write_with_sync_end` detects when an existing synchronized-output group ends, including a close and reopen within one chunk. The transport owns reconnect, sequence numbers, and output scheduling.

`cargo test -p t3-terminal` runs operations and full snapshot comparisons obtained from the pinned original browser adapter. The fixture includes ANSI styling, Unicode/wide cells, combining sequences, partial UTF-8/CSI/OSC, scrollback and alternate screens, selection, resize, PTY queries/history replay, Kitty keyboard actions, bracketed paste, mouse encoding and synchronized-output boundaries. It also verifies the vendored module's hash and embedded revision.

On Linux, `cargo run --release -p t3-terminal --example measure` reports cold/warm initialization, VT throughput and full-grid snapshot cost. These measurements cover the engine. Application comparisons also need GPUI painting, input, transport, frame scheduling and the reference Electron processes.
