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

`vendor/gpui-pre-linux/` is `gpui-pre-linux` 0.3.12 from [Zed](https://github.com/zed-industries/zed), used under the Apache License 2.0 (`vendor/gpui-pre-linux/LICENSE-APACHE`). It is patched in through `[patch.crates-io]` with local fixes: X11 windows ask wgpu for the `Immediate` present mode (`src/linux/x11/window.rs`), as Wayland windows already ask for `Mailbox`. With the default `Fifo`, presenting a frame blocks the main thread until the next vblank, so key events wait behind it; on an X server whose vblank is slow (no monitor connected, about 1 Hz), typing through XIM falls seconds behind. GPUI paces frames with its own timer at the screen's refresh rate, so `Immediate` does not render more often. X11 accessibility coordinates use the same full client-window origin as GPUI elements, including client-side decoration padding.

`crates/t3code-gpui/assets/InterVariable.ttf` is Inter 4.1 by the Inter Project Authors, distributed under the SIL Open Font License 1.1. The license is in `crates/t3code-gpui/assets/Inter-LICENSE.txt`. Source: https://github.com/rsms/inter/tree/v4.1.

Provider icon SVGs and the stop glyph derive from T3 Code `apps/web/src/components/Icons.tsx` and `ComposerPrimaryActions.tsx` at commit 322f6544df.

```text
MIT License

Copyright (c) 2026 T3 Tools Inc.

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

`vendor/gpui-pre-wgpu/` is `gpui-pre-wgpu` 0.3.12, distributed under Apache-2.0 (`vendor/gpui-pre-wgpu/LICENSE-APACHE`). The local patch preserves each selected font's effective weight across shaping, metrics, fallback, rasterization and glyph-cache identities. Variable fonts use the requested weight within their supported axis range; static faces retain their actual weight.

`crates/t3code-gpui/tests/fixtures/FiraMono-Medium.ttf` is copied from the `cosmic-text` 0.19.0 test fonts. It is distributed under the SIL Open Font License 1.1, included as `FiraMono-LICENSE` in that directory.

## GPUI accessibility cache

`vendor/gpui-pre/` is `gpui-pre` 0.3.12, distributed under Apache-2.0 (`vendor/gpui-pre/LICENSE-APACHE`). The local patch replays accessible nodes, focus mappings, bounds and action listeners together with cached view prepaint data. It preserves the accessible tree when a pane reuses its rendering cache. Window bounds reach the platform adapter with each accessible update. Text inputs with exposed values gain a synthetic text run so AT-SPI can read their content.

## T3 Code interface

Interface behavior, permission labels and descriptions are based on [T3 Code](https://github.com/pingdotgg/t3code/tree/24e77473ecf41bc1f7142529c7911c1c1aae1d76), including `apps/web/src/components/chat/runtimeModeConfig.ts` and `ComposerFooterControls.tsx`. Editor icon SVGs in `crates/t3code-gpui/assets/editors` derive from `apps/web/src/components/Icons.tsx` and `JetBrainsIcons.tsx` at that revision. T3 Code is distributed under the MIT License:

```text
MIT License

Copyright (c) 2026 T3 Tools Inc.

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

## GPUI component menus

`vendor/gpui-component/` is `gpui-component` 0.9.1, distributed under Apache-2.0 (`vendor/gpui-component/LICENSE-APACHE`). Custom popup menu items expose an explicit accessible label alongside their custom content.

The assistant message metadata and copy behavior in `crates/t3code-gpui/src/{timeline,row_view,markdown_directives,timestamp}.rs` derive from T3 Code `apps/web/src/components/chat/{TimelineAssistantRows,MessageCopyButton}.tsx`, `apps/web/src/timestampFormat.ts`, and `packages/shared/src/{codexMarkdownDirectives,codexFileCitations,codexArtifactTemplates}.ts` at source revision `24e77473ecf41bc1f7142529c7911c1c1aae1d76`, under the T3 Tools Inc. MIT License reproduced above.

ICU4X date/time formatting and its Unicode/CLDR data are distributed under the Unicode License V3, reproduced in `crates/t3code-gpui/assets/licenses/Unicode-3.0.txt` and installed with the application notices.
