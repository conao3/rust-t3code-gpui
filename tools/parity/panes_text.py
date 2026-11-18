import argparse
import json
from pathlib import Path

DIAGRAM = {
    "Show code",
    "Show diagram",
    "Copy code",
    "Copied",
    "Expand diagram",
    "Disable line wrap",
    "Wrap lines",
    "mermaid",
    "Mermaid diagram",
    "Mermaid diagram, zoomable image",
    "Expanded image preview",
    "Close image preview",
}


def texts(nodes, scope):
    result = set()
    for node in nodes:
        if node["role"] == "InlineTextBox":
            continue
        ancestors = node.get("ancestors", [])
        names = [item.get("name", "") for item in [node, *ancestors]]
        selected = {
            "sidechat": any(
                name.startswith("Workspace panel: Side chat") for name in names
            )
            or node.get("name") in {"Add panel surface", "Close Side chat"},
            "search": "Search project contents" in names,
            "diagram": node.get("name") in DIAGRAM or node.get("value") in DIAGRAM,
            "timeline": "Conversation" in names and "Message input" not in names,
        }[scope]
        if not selected:
            continue
        if node["role"] in ["StaticText", "label"] and any(
            parent["role"] in ["button", "combobox", "combo box", "textbox", "entry"]
            for parent in ancestors
        ):
            continue
        for field in ["name", "value"]:
            value = node.get(field)
            if value is not None and (value := " ".join(str(value).split())):
                result.add(value)
    return result


def compare(reference, native, scope):
    expected, actual = texts(reference, scope), texts(native, scope)
    return {
        "reference": sorted(expected),
        "native": sorted(actual),
        "missing": sorted(expected - actual),
        "extra": sorted(actual - expected),
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("run", type=Path)
    parser.add_argument(
        "--scope", choices=["sidechat", "search", "diagram", "timeline"], required=True
    )
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    results = {}
    for reference in sorted((args.run / "reference").glob("[0-9]*.json")):
        native = args.run / "native" / reference.name
        results[reference.stem] = (
            compare(
                json.loads(reference.read_text()),
                json.loads(native.read_text()),
                args.scope,
            )
            if native.exists()
            else {"error": "Native checkpoint missing"}
        )
    if not results:
        raise ValueError("No accessible snapshots found")
    differences = sum(
        len(item.get("missing", [])) + len(item.get("extra", []))
        for item in results.values()
    )
    report = {"scope": args.scope, "differences": differences, "checkpoints": results}
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"differences": differences, "output": str(args.output)}))
    raise SystemExit(
        1 if differences or any("error" in item for item in results.values()) else 0
    )


if __name__ == "__main__":
    main()
