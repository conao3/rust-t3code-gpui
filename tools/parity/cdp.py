"""Control the real T3 Electron renderer through its loopback CDP endpoint."""

import argparse
import asyncio
import base64
import json
import time
from pathlib import Path
from urllib.error import URLError
from urllib.request import urlopen

import websockets

MODIFIERS = {
    "ctrl": ("Control", "ControlLeft", 17, 2),
    "alt": ("Alt", "AltLeft", 18, 1),
    "shift": ("Shift", "ShiftLeft", 16, 8),
    "super": ("Meta", "MetaLeft", 91, 4),
}


class Cdp:
    def __init__(self, endpoint):
        self.endpoint = endpoint.rstrip("/")
        self.next_id = 0
        self.socket = None

    async def __aenter__(self):
        def list_pages():
            with urlopen(self.endpoint + "/json/list", timeout=5) as response:
                return json.load(response)

        deadline = time.monotonic() + 120
        while True:
            try:
                pages = await asyncio.to_thread(list_pages)
                pages = [p for p in pages if p.get("type") == "page"]
                ready = len(pages) == 1 and pages[0].get("url", "") not in [
                    "",
                    "about:blank",
                ]
                if len(pages) > 1 or ready or time.monotonic() >= deadline:
                    break
            except (URLError, TimeoutError) as error:
                reason = error.reason if isinstance(error, URLError) else error
                if (
                    not isinstance(reason, (ConnectionRefusedError, TimeoutError))
                    or time.monotonic() >= deadline
                ):
                    raise
            await asyncio.sleep(0.1)
        if len(pages) != 1:
            raise RuntimeError(f"Expected one isolated renderer, got {len(pages)}")
        if not ready:
            raise RuntimeError(
                "The isolated renderer did not navigate to its application"
            )
        self.page_url = pages[0]["url"]
        self.socket = await websockets.connect(
            pages[0]["webSocketDebuggerUrl"],
            max_size=32 * 1024 * 1024,
            open_timeout=60,
        )
        try:
            try:
                await self.wait_document()
            except TimeoutError:
                await self.socket.close()
                self.socket = await websockets.connect(
                    pages[0]["webSocketDebuggerUrl"],
                    max_size=32 * 1024 * 1024,
                    open_timeout=60,
                )
                await self.wait_document()
        except BaseException:
            await self.socket.close()
            raise
        return self

    async def wait_document(self):
        deadline = time.monotonic() + 60
        while not await self.evaluate(
            "document.readyState === 'complete' && !!document.querySelector('button')"
        ):
            if time.monotonic() >= deadline:
                raise TimeoutError("The application document did not finish loading")
            await asyncio.sleep(0.1)

    async def __aexit__(self, *_):
        try:
            await self.release_modifiers()
        finally:
            await self.socket.close()

    async def call(self, method, **params):
        self.next_id += 1
        message_id = self.next_id
        await self.socket.send(
            json.dumps({"id": message_id, "method": method, "params": params})
        )
        async with asyncio.timeout(15):
            while True:
                reply = json.loads(await self.socket.recv())
                if reply.get("id") != message_id:
                    continue
                if "error" in reply:
                    raise RuntimeError(f"{method}: {reply['error']}")
                return reply.get("result", {})

    async def evaluate(self, expression):
        reply = await self.call(
            "Runtime.evaluate",
            expression=expression,
            returnByValue=True,
            awaitPromise=True,
        )
        if "exceptionDetails" in reply:
            raise RuntimeError(str(reply["exceptionDetails"]))
        return reply.get("result", {}).get("value")

    async def server_origin(self):
        if self.page_url.startswith(("http://", "https://")):
            return self.page_url
        result = await self.evaluate(
            "window.desktopBridge?.getLocalEnvironmentBootstraps().find(e => e.id === 'primary')?.httpBaseUrl"
        )
        if not isinstance(result, str):
            raise TypeError(
                "The reference desktop did not expose its local server origin"
            )
        return result

    async def appearance(self):
        return await self.evaluate(
            "getComputedStyle(document.documentElement).colorScheme"
        )

    async def tree(self):
        result = await self.call("Accessibility.getFullAXTree")
        return [
            {
                "role": n.get("role", {}).get("value"),
                "name": n.get("name", {}).get("value", ""),
                "value": n.get("value", {}).get("value"),
                "backend_id": n.get("backendDOMNodeId"),
                "disabled": any(
                    p["value"].get("value") is True
                    for p in n.get("properties", [])
                    if p["name"] == "disabled"
                ),
                "focused": any(
                    p["value"].get("value") is True
                    for p in n.get("properties", [])
                    if p["name"] == "focused"
                ),
                "checked": next(
                    (
                        p["value"].get("value") in [True, "true"]
                        for p in n.get("properties", [])
                        if p["name"] == "checked"
                    ),
                    None,
                ),
            }
            for n in result["nodes"]
            if not n.get("ignored")
        ]

    async def click(self, role, name, index=0):
        matches = [
            n for n in await self.tree() if n["role"] == role and n["name"] == name
        ]
        if not 0 <= index < len(matches):
            raise RuntimeError(
                f"Cannot find {role} {name!r} #{index}; matches={len(matches)}"
            )
        node = matches[index]["backend_id"]
        if matches[index]["disabled"]:
            raise RuntimeError(f"Target is disabled: {role} {name!r}")
        await self.call("DOM.scrollIntoViewIfNeeded", backendNodeId=node)
        box = (await self.call("DOM.getBoxModel", backendNodeId=node))["model"][
            "content"
        ]
        x, y = sum(box[::2]) / 4, sum(box[1::2]) / 4
        await self.call(
            "Input.dispatchMouseEvent",
            type="mouseMoved",
            x=x,
            y=y,
            modifiers=getattr(self, "held_modifiers", 0),
        )
        await asyncio.sleep(0.1)
        for kind in ["mousePressed", "mouseReleased"]:
            await self.call(
                "Input.dispatchMouseEvent",
                type=kind,
                x=x,
                y=y,
                button="left",
                clickCount=1,
                modifiers=getattr(self, "held_modifiers", 0),
            )
            await asyncio.sleep(0.08)

    async def press(self, chord):
        parts = chord.lower().split("+")
        aliases = MODIFIERS
        key = parts[-1]
        named = {
            "enter": ("Enter", "Enter", 13),
            "escape": ("Escape", "Escape", 27),
            "tab": ("Tab", "Tab", 9),
            "backspace": ("Backspace", "Backspace", 8),
            "up": ("ArrowUp", "ArrowUp", 38),
            "down": ("ArrowDown", "ArrowDown", 40),
            "left": ("ArrowLeft", "ArrowLeft", 37),
            "right": ("ArrowRight", "ArrowRight", 39),
            "delete": ("Delete", "Delete", 46),
            "home": ("Home", "Home", 36),
            "end": ("End", "End", 35),
            "pageup": ("PageUp", "PageUp", 33),
            "pagedown": ("PageDown", "PageDown", 34),
            "space": (" ", "Space", 32),
            "[": ("[", "BracketLeft", 219),
            "]": ("]", "BracketRight", 221),
        }
        if any(part not in aliases for part in parts[:-1]):
            raise ValueError(f"Unsupported modifier: {chord}")
        if key.startswith("f") and key[1:].isdigit() and 1 <= int(key[1:]) <= 24:
            key, code, vk = key.upper(), key.upper(), 111 + int(key[1:])
        elif key in named:
            key, code, vk = named[key]
        elif len(key) == 1 and key.isascii() and key.isalnum():
            code, vk = (
                ("Key" if key.isalpha() else "Digit") + key.upper(),
                ord(key.upper()),
            )
        else:
            raise ValueError(f"Unsupported key: {key}")
        modifiers = getattr(self, "held_modifiers", 0)
        held = []
        try:
            for part in parts[:-1]:
                modifier_key, modifier_code, modifier_vk, bit = aliases[part]
                if modifiers & bit:
                    continue
                modifiers |= bit
                held.append((modifier_key, modifier_code, modifier_vk, bit))
                await self.call(
                    "Input.dispatchKeyEvent",
                    type="rawKeyDown",
                    key=modifier_key,
                    code=modifier_code,
                    windowsVirtualKeyCode=modifier_vk,
                    modifiers=modifiers,
                )
            if modifiers & 8 and len(key) == 1:
                key = key.upper()
            for kind in ["rawKeyDown", "keyUp"]:
                await self.call(
                    "Input.dispatchKeyEvent",
                    type=kind,
                    key=key,
                    code=code,
                    windowsVirtualKeyCode=vk,
                    modifiers=modifiers,
                )
        finally:
            for modifier_key, modifier_code, modifier_vk, bit in reversed(held):
                modifiers &= ~bit
                await self.call(
                    "Input.dispatchKeyEvent",
                    type="keyUp",
                    key=modifier_key,
                    code=modifier_code,
                    windowsVirtualKeyCode=modifier_vk,
                    modifiers=modifiers,
                )

    async def modifier(self, key, held):
        if key not in MODIFIERS or type(held) is not bool:
            raise ValueError("Expected a modifier key and a boolean held state")
        name, code, vk, bit = MODIFIERS[key]
        current = getattr(self, "held_modifiers", 0)
        if bool(current & bit) == held:
            return
        modifiers = current | bit if held else current & ~bit
        await self.call(
            "Input.dispatchKeyEvent",
            type="rawKeyDown" if held else "keyUp",
            key=name,
            code=code,
            windowsVirtualKeyCode=vk,
            modifiers=modifiers,
        )
        self.held_modifiers = modifiers

    async def release_modifiers(self):
        for key, (_, _, _, bit) in MODIFIERS.items():
            if getattr(self, "held_modifiers", 0) & bit:
                await self.modifier(key, False)

    async def pointer(self, kind, x, y, button="left"):
        kinds = {"move": "mouseMoved", "down": "mousePressed", "up": "mouseReleased"}
        if kind not in kinds or button not in ["left", "middle", "right"]:
            raise ValueError("Unsupported pointer action")
        viewport = await self.evaluate("({width: innerWidth, height: innerHeight})")
        if not 0 <= x < viewport["width"] or not 0 <= y < viewport["height"]:
            raise ValueError("Pointer coordinates must be inside the content viewport")
        held = getattr(self, "pointer_button", None)
        if kind == "down":
            held = button
        elif kind == "up":
            held = None
        bits = {"left": 1, "middle": 4, "right": 2, None: 0}
        await self.call(
            "Input.dispatchMouseEvent",
            type=kinds[kind],
            x=x,
            y=y,
            button=button if kind != "move" else held or "none",
            buttons=bits[held],
            modifiers=getattr(self, "held_modifiers", 0),
            clickCount=1 if kind != "move" else 0,
        )
        self.pointer_button = held

    async def type(self, text):
        if any(ord(character) < 32 or ord(character) == 127 for character in text):
            raise ValueError("Use explicit key or paste actions for control characters")
        for character in text:
            await self.call("Input.insertText", text=character)
            await asyncio.sleep(0.02)

    async def screenshot(self, path):
        result = await self.call(
            "Page.captureScreenshot", format="png", captureBeyondViewport=False
        )
        Path(path).write_bytes(base64.b64decode(result["data"]))


async def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--endpoint", default="http://127.0.0.1:39991")
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("tree")
    p = sub.add_parser("click")
    p.add_argument("role")
    p.add_argument("name")
    p.add_argument("--index", type=int, default=0)
    p = sub.add_parser("press")
    p.add_argument("chord")
    p = sub.add_parser("type")
    p.add_argument("text")
    p = sub.add_parser("pointer")
    p.add_argument("kind", choices=["move", "down", "up"])
    p.add_argument("x", type=int)
    p.add_argument("y", type=int)
    p.add_argument("--button", choices=["left", "middle", "right"], default="left")
    p = sub.add_parser("evaluate")
    p.add_argument("expression")
    p = sub.add_parser("screenshot")
    p.add_argument("path")
    args = parser.parse_args()
    async with Cdp(args.endpoint) as page:
        if args.command == "tree":
            print(json.dumps(await page.tree(), ensure_ascii=False, indent=2))
        elif args.command == "click":
            await page.click(args.role, args.name, args.index)
        elif args.command == "press":
            await page.press(args.chord)
        elif args.command == "type":
            await page.type(args.text)
        elif args.command == "pointer":
            await page.pointer(args.kind, args.x, args.y, args.button)
        elif args.command == "evaluate":
            print(json.dumps(await page.evaluate(args.expression), ensure_ascii=False))
        elif args.command == "screenshot":
            await page.screenshot(args.path)


if __name__ == "__main__":
    asyncio.run(main())
