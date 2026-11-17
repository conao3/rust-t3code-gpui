"""Find native controls through AT-SPI and send real X11 user input."""

import argparse
import asyncio
import json
import os
import subprocess
import time
from itertools import groupby
from pathlib import Path


def stale_object(error):
    return "Unknown object" in str(error) or (
        error.domain == "atspi_error"
        and error.code == 1
        and error.message.startswith("/org/a11y/atspi/accessible/")
    )


class Native:
    def __init__(self, session):
        self.session_path = Path(session)
        self.session = json.loads(self.session_path.read_text())
        os.environ["DBUS_SESSION_BUS_ADDRESS"] = self.session["dbus_address"]
        os.environ["DISPLAY"] = self.session["display"]
        import pyatspi

        self.atspi = pyatspi
        self.desktop = pyatspi.Registry.getDesktop(0)
        from gi.repository import Atspi

        self.desktop.set_cache_mask(Atspi.Cache.NONE)
        deadline = time.monotonic() + 15
        while True:
            try:
                self.window = self.xdo(
                    "search", "--onlyvisible", "--pid", self.session["pid"]
                ).splitlines()[0]
                break
            except subprocess.CalledProcessError:
                if time.monotonic() >= deadline:
                    raise RuntimeError(
                        "Native fixture did not open its window"
                    ) from None
                time.sleep(0.1)

    def renderer_info(self):
        prefix = "T3CODE_GPUI_RENDERER_INFO "
        for line in self.session_path.with_name("native.log").read_text().splitlines():
            if line.startswith(prefix):
                return json.loads(line.removeprefix(prefix))
        return None

    def xdo(self, *args, timeout=10):
        return subprocess.check_output(
            ["xdotool", *map(str, args)], text=True, timeout=timeout
        ).strip()

    def app(self):
        for app in self.desktop:
            if app.get_process_id() == self.session["pid"]:
                from gi.repository import Atspi

                app.set_cache_mask(Atspi.Cache.NONE)
                app.clear_cache()
                return app
        raise RuntimeError(
            f"Native PID {self.session['pid']} has no accessible application"
        )

    def nodes(self):
        queue = [(self.app(), [])]
        result = []
        self.parent_paths = {}
        while queue:
            node, parents = queue.pop(0)
            node.clear_cache()
            self.parent_paths[id(node)] = parents
            result.append(node)
            if len(result) > 5000:
                raise RuntimeError(
                    "Accessible tree exceeds 5000 nodes; refusing an incomplete snapshot"
                )
            queue.extend(
                (child, parents + [id(node)]) for child in node if child is not None
            )
        return result

    def tree(self):
        from gi.repository import GLib

        attempts = 0
        while True:
            attempts += 1
            context = GLib.MainContext.default()
            while context.pending():
                context.iteration(False)
            try:
                return self.snapshot()
            except GLib.Error as error:
                if not stale_object(error) or attempts >= 3:
                    raise
                time.sleep(0.05)

    def snapshot(self):
        result = []
        controls = {}
        identities = {}
        for node in self.nodes():
            role = node.getRoleName()
            name = node.name
            identities[id(node)] = {"role": role, "name": name}
            controls.setdefault((role, name), []).append(node)
            try:
                rect = node.queryComponent().getExtents(self.atspi.DESKTOP_COORDS)
                bounds = {
                    "x": rect.x,
                    "y": rect.y,
                    "width": rect.width,
                    "height": rect.height,
                }
            except NotImplementedError:
                bounds = None
            value = None
            if role in [
                "entry",
                "text",
                "label",
                "document frame",
                "terminal",
                "combo box",
            ]:
                try:
                    content = node.queryText()
                    value = content.getText(0, content.characterCount)
                except NotImplementedError:
                    pass
            state = node.getState()
            result.append(
                {
                    "role": role,
                    "name": name,
                    "value": value,
                    "ancestors": [
                        identities[parent] for parent in self.parent_paths[id(node)]
                    ],
                    "focused": state.contains(self.atspi.STATE_FOCUSED),
                    "disabled": not state.contains(self.atspi.STATE_ENABLED),
                    "checked": state.contains(
                        self.atspi.STATE_PRESSED
                        if role == "toggle button"
                        else self.atspi.STATE_CHECKED
                    )
                    if role
                    in [
                        "radio menu item",
                        "check menu item",
                        "toggle button",
                        "switch",
                        "check box",
                        "radio button",
                    ]
                    else None,
                    "expanded": state.contains(self.atspi.STATE_EXPANDED)
                    if state.contains(self.atspi.STATE_EXPANDABLE)
                    else None,
                    "selected": state.contains(self.atspi.STATE_SELECTED),
                    "bounds": bounds,
                    "states": [str(value) for value in state.getStates()],
                }
            )
        self.snapshot_controls = controls
        return result

    def focus(self):
        self.xdo("windowactivate", "--sync", self.window)
        self.xdo("windowfocus", "--sync", self.window)
        time.sleep(0.1)

    def focus_control(self, role, name, index=0):
        self.focus()
        node = self.reveal_control(role, name, index, require_focus=True)
        if not node.queryComponent().grabFocus():
            raise RuntimeError(f"Focus request failed: {role} {name!r}")

    def reveal_control(self, role, name, index=0, *, require_focus=False):
        matches = [
            node
            for node in self.tree()
            if node["role"] == role and node["name"] == name
        ]
        if not 0 <= index < len(matches):
            raise RuntimeError(
                f"Cannot find {role} {name!r} #{index}; matches={len(matches)}"
            )
        node = self.snapshot_controls[(role, name)][index]
        state = node.getState()
        if not state.contains(self.atspi.STATE_ENABLED):
            raise RuntimeError(f"Target is disabled: {role} {name!r}")
        if require_focus and not state.contains(self.atspi.STATE_FOCUSABLE):
            raise RuntimeError(f"Target cannot take focus: {role} {name!r}")
        node.queryComponent().scrollTo(self.atspi.SCROLL_ANYWHERE)
        time.sleep(0.1)
        return node

    def click(self, role, name, index=0):
        self.focus()
        self.reveal_control(role, name, index)
        matches = [
            node
            for node in self.tree()
            if node["role"] == role and node["name"] == name
        ]
        if not 0 <= index < len(matches):
            raise RuntimeError(
                f"Cannot find {role} {name!r} #{index}; matches={len(matches)}"
            )
        rect = matches[index]["bounds"]
        if matches[index].get("disabled", False):
            raise RuntimeError(f"Target is disabled: {role} {name!r}")
        if rect is None or rect["width"] <= 0 or rect["height"] <= 0:
            raise RuntimeError("Target has no visible bounds")
        self.xdo(
            "mousemove", rect["x"] + rect["width"] // 2, rect["y"] + rect["height"] // 2
        )
        time.sleep(0.1)
        self.xdo("mousedown", 1)
        time.sleep(0.08)
        self.xdo("mouseup", 1)

    def press(self, chord):
        self.focus()
        keys = {
            "backspace": "BackSpace",
            "escape": "Escape",
            "enter": "Return",
            "tab": "Tab",
            "up": "Up",
            "down": "Down",
            "left": "Left",
            "right": "Right",
            "home": "Home",
            "end": "End",
            "pageup": "Prior",
            "pagedown": "Next",
            "delete": "Delete",
            "space": "space",
        }
        parts = chord.lower().split("+")
        parts[-1] = keys.get(parts[-1], parts[-1])
        if getattr(self, "held_modifiers", set()):
            self.xdo("key", "+".join(parts))
        else:
            self.xdo("key", "--clearmodifiers", "+".join(parts))

    def modifier(self, key, held):
        keys = {
            "ctrl": "Control_L",
            "alt": "Alt_L",
            "shift": "Shift_L",
            "super": "Super_L",
        }
        if key not in keys or type(held) is not bool:
            raise ValueError("Expected a modifier key and a boolean held state")
        current = getattr(self, "held_modifiers", set()).copy()
        if (key in current) == held:
            return
        self.focus()
        self.xdo("keydown" if held else "keyup", keys[key])
        if held:
            current.add(key)
        else:
            current.discard(key)
        self.held_modifiers = current

    def release_modifiers(self):
        for key in list(getattr(self, "held_modifiers", set())):
            self.modifier(key, False)

    def pointer(self, kind, x, y, button="left"):
        if kind not in ["move", "down", "up"] or button not in [
            "left",
            "middle",
            "right",
        ]:
            raise ValueError("Unsupported pointer action")
        bounds = self.content_bounds()
        if not 0 <= x < bounds["width"] or not 0 <= y < bounds["height"]:
            raise ValueError("Pointer coordinates must be inside the content viewport")
        self.focus()
        self.xdo("mousemove", bounds["x"] + x, bounds["y"] + y)
        if kind != "move":
            self.xdo(
                "mousedown" if kind == "down" else "mouseup",
                {"left": 1, "middle": 2, "right": 3}[button],
            )

    def type(self, text):
        if any(ord(character) < 32 or ord(character) == 127 for character in text):
            raise ValueError("Use explicit key or paste actions for control characters")
        self.focus()
        for ascii_only, characters in groupby(text, str.isascii):
            segment = "".join(characters)
            delay = 20 if ascii_only else 300
            self.xdo(
                "type",
                "--clearmodifiers",
                "--delay",
                delay,
                segment,
                timeout=max(10, len(segment) * delay / 1000 + 5),
            )

    def content_bounds(self):
        groups = [node for node in self.tree() if node["name"] == "Application content"]
        if len(groups) != 1:
            raise RuntimeError(
                f"Expected one accessible application content group, got {len(groups)}"
            )
        return groups[0]["bounds"]

    def geometry(self):
        return dict(
            line.split("=", 1)
            for line in self.xdo(
                "getwindowgeometry", "--shell", self.window
            ).splitlines()
        )

    async def viewport(self, width, height):
        self.focus()
        self.xdo("windowmove", self.window, 40, 40)
        await asyncio.sleep(0.5)
        previous = None
        stable = 0
        for _ in range(40):
            bounds, geometry = self.content_bounds(), self.geometry()
            current = bounds, geometry
            stable = stable + 1 if current == previous else 0
            previous = current
            if stable >= 2:
                if bounds["width"] == width and bounds["height"] == height:
                    return
                self.xdo(
                    "windowsize",
                    self.window,
                    int(geometry["WIDTH"]) + width - bounds["width"],
                    int(geometry["HEIGHT"]) + height - bounds["height"],
                )
                previous, stable = None, 0
            await asyncio.sleep(0.2)
        raise RuntimeError(
            f"Cannot set native content viewport: {self.content_bounds()}"
        )

    def screenshot(self, path):
        from io import BytesIO

        from PIL import Image, ImageChops

        self.focus()
        time.sleep(0.5)
        bounds, geometry = self.content_bounds(), self.geometry()
        x, y = bounds["x"] - int(geometry["X"]), bounds["y"] - int(geometry["Y"])
        previous = None
        deadline = time.monotonic() + 5
        while True:
            data = subprocess.check_output(
                ["import", "-window", self.window, "png:-"], timeout=8
            )
            with Image.open(BytesIO(data)) as picture:
                current = picture.crop(
                    (x, y, x + bounds["width"], y + bounds["height"])
                ).convert("RGB")
            if (
                previous is not None
                and ImageChops.difference(previous, current).getbbox() is None
            ):
                current.save(path)
                return
            if time.monotonic() >= deadline:
                raise RuntimeError(
                    "Native checkpoint did not reach a stable painted frame"
                )
            previous = current
            time.sleep(0.2)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--session", type=Path, required=True)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("tree")
    for command in ["click", "focus"]:
        p = sub.add_parser(command)
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
    p = sub.add_parser("screenshot")
    p.add_argument("path", type=Path)
    args = parser.parse_args()
    app = Native(args.session)
    if args.command == "tree":
        print(json.dumps(app.tree(), ensure_ascii=False, indent=2))
    elif args.command == "click":
        app.click(args.role, args.name, args.index)
    elif args.command == "focus":
        app.focus_control(args.role, args.name, args.index)
    elif args.command == "press":
        app.press(args.chord)
    elif args.command == "type":
        app.type(args.text)
    elif args.command == "pointer":
        app.pointer(args.kind, args.x, args.y, args.button)
    elif args.command == "screenshot":
        app.screenshot(args.path)


if __name__ == "__main__":
    main()
