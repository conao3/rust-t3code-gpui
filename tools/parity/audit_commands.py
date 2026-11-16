"""Audit upstream keybinding commands without treating an alias as semantic parity."""

import argparse
import json
import re
from pathlib import Path


def reference_commands(source):
    arrays = dict(
        re.findall(
            r"(?:export\s+)?const\s+(\w+)\s*=\s*\[(.*?)\]\s*as const", source, re.DOTALL
        )
    )

    def expand(name):
        body = arrays[name]
        commands = re.findall(r'"([a-z][\w.]+)"', body)
        for other in re.findall(r"\.\.\.(\w+)", body):
            commands.extend(expand(other))
        return commands

    return sorted(set(expand("STATIC_KEYBINDING_COMMANDS")))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    source = (args.reference_root / "packages/contracts/src/keybindings.ts").read_text()
    commands = reference_commands(source)
    native = Path("crates/t3code-gpui/src/app.rs").read_text()
    dispatch = native.split("fn action_for(", 1)[1].split("\n}\n", 1)[0]
    registered = set(re.findall(r'"([a-z][\w.]+)"', dispatch))
    dynamic = {f"thread.jump.{i}" for i in range(1, 10)} | {
        f"modelPicker.jump.{i}" for i in range(1, 10)
    }
    dynamic |= {"modelPicker.previousProvider", "modelPicker.nextProvider"}
    rows = [
        {
            "command": command,
            "dispatch": "registered"
            if command in registered
            else "dynamic"
            if command in dynamic
            else "missing",
            "equivalence": "not-tested",
        }
        for command in commands
    ]
    result = {
        "total": len(rows),
        "missing_dispatch": sum(r["dispatch"] == "missing" for r in rows),
        "commands": rows,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(
        json.dumps(
            {"total": result["total"], "missing_dispatch": result["missing_dispatch"]}
        )
    )
    raise SystemExit(1 if any(r["equivalence"] != "passed" for r in rows) else 0)


if __name__ == "__main__":
    main()
