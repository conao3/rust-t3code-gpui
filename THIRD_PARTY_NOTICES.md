# Third-party notices

`crates/t3-client/src/{error,http,projection,rpc}.rs` and `crates/t3-client/src/rpc/` derive from [t3term](https://github.com/krishhgg/t3term) (commit d1d33e8), used under the MIT License:

```
MIT License

Copyright (c) 2026 Krish Garg

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

`vendor/gpui-pre-linux/` is `gpui-pre-linux` 0.3.12 from [Zed](https://github.com/zed-industries/zed), used under the Apache License 2.0 (`vendor/gpui-pre-linux/LICENSE-APACHE`). It is patched in through `[patch.crates-io]` with one change: X11 windows ask wgpu for the `Immediate` present mode (`src/linux/x11/window.rs`), as Wayland windows already ask for `Mailbox`. With the default `Fifo`, presenting a frame blocks the main thread until the next vblank, so key events wait behind it; on an X server whose vblank is slow (no monitor connected, about 1 Hz), typing through XIM falls seconds behind. GPUI paces frames with its own timer at the screen's refresh rate, so `Immediate` does not render more often.
