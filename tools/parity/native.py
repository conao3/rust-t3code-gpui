"""Find native controls through AT-SPI and send real X11 user input."""

import argparse
import asyncio
import json
import os
import subprocess
import time
from pathlib import Path


class Native:
    def __init__(self, session):
        self.session = json.loads(Path(session).read_text())
        os.environ["DBUS_SESSION_BUS_ADDRESS"] = self.session["dbus_address"]
        os.environ["DISPLAY"] = self.session["display"]
        import pyatspi

        self.atspi = pyatspi
        self.desktop = pyatspi.Registry.getDesktop(0)
        from gi.repository import Atspi

        self.desktop.set_cache_mask(Atspi.Cache.NONE)
        self.window = self.xdo(
            "search", "--onlyvisible", "--pid", self.session["pid"]
        ).splitlines()[0]

    def xdo(self, *args):
        return subprocess.check_output(["xdotool", *map(str, args)], text=True).strip()

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
        queue = [self.app()]
        result = []
        while queue:
            node = queue.pop(0)
            result.append(node)
            if len(result) > 5000:
                raise RuntimeError(
                    "Accessible tree exceeds 5000 nodes; refusing an incomplete snapshot"
                )
            queue.extend(child for child in node if child is not None)
        return result

    def tree(self):
        from gi.repository import GLib

        deadline = time.monotonic() + 5
        while True:
            context = GLib.MainContext.default()
            while context.pending():
                context.iteration(False)
            try:
                return self.snapshot()
            except GLib.Error as error:
                if "Unknown object" not in str(error) or time.monotonic() >= deadline:
                    raise
                time.sleep(0.05)

    def snapshot(self):
        result = []
        for node in self.nodes():
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
            if node.getRoleName() in ["entry", "text"]:
                try:
                    content = node.queryText()
                    value = content.getText(0, content.characterCount)
                except NotImplementedError:
                    pass
            result.append(
                {
                    "role": node.getRoleName(),
                    "name": node.name,
                    "value": value,
                    "bounds": bounds,
                    "states": [str(state) for state in node.getState().getStates()],
                }
            )
        return result

    def focus(self):
        self.xdo("windowactivate", "--sync", self.window)
        self.xdo("windowfocus", "--sync", self.window)
        time.sleep(0.1)

    def click(self, role, name, index=0):
        self.focus()
        matches = [
            node
            for node in self.nodes()
            if node.getRoleName() == role and node.name == name
        ]
        if not 0 <= index < len(matches):
            raise RuntimeError(
                f"Cannot find {role} {name!r} #{index}; matches={len(matches)}"
            )
        rect = matches[index].queryComponent().getExtents(self.atspi.DESKTOP_COORDS)
        if rect.width <= 0 or rect.height <= 0:
            raise RuntimeError("Target has no visible bounds")
        self.xdo("mousemove", rect.x + rect.width // 2, rect.y + rect.height // 2)
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
        }
        parts = chord.lower().split("+")
        parts[-1] = keys.get(parts[-1], parts[-1])
        self.xdo("key", "--clearmodifiers", "+".join(parts))

    def type(self, text):
        self.focus()
        self.xdo("type", "--clearmodifiers", "--delay", 20, text)

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
        for _ in range(3):
            bounds, geometry = self.content_bounds(), self.geometry()
            if bounds["width"] == width and bounds["height"] == height:
                return
            self.xdo(
                "windowsize",
                self.window,
                int(geometry["WIDTH"]) + width - bounds["width"],
                int(geometry["HEIGHT"]) + height - bounds["height"],
            )
            await asyncio.sleep(0.5)
        raise RuntimeError(
            f"Cannot set native content viewport: {self.content_bounds()}"
        )

    def screenshot(self, path):
        from io import BytesIO

        from PIL import Image

        self.focus()
        bounds, geometry = self.content_bounds(), self.geometry()
        data = subprocess.check_output(["import", "-window", self.window, "png:-"])
        x, y = bounds["x"] - int(geometry["X"]), bounds["y"] - int(geometry["Y"])
        with Image.open(BytesIO(data)) as picture:
            picture.crop((x, y, x + bounds["width"], y + bounds["height"])).save(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--session", type=Path, required=True)
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
    p = sub.add_parser("screenshot")
    p.add_argument("path", type=Path)
    args = parser.parse_args()
    app = Native(args.session)
    if args.command == "tree":
        print(json.dumps(app.tree(), ensure_ascii=False, indent=2))
    elif args.command == "click":
        app.click(args.role, args.name, args.index)
    elif args.command == "press":
        app.press(args.chord)
    elif args.command == "type":
        app.type(args.text)
    elif args.command == "screenshot":
        app.screenshot(args.path)


if __name__ == "__main__":
    main()
