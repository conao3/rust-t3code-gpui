import argparse
import json
from pathlib import Path

REGIONS = {"Message input", "Thread details", "Stashed prompts", "Run context"}


def texts(nodes):
    result = set()
    for node in nodes:
        role = node["role"]
        ancestors = node.get("ancestors", [])
        if role == "InlineTextBox":
            continue
        scoped = (
            role == "heading"
            and node.get("name", "").startswith(("What should", "Choose a project"))
            or node.get("name") == "or start without a project"
            or role in {"combobox", "combo box"}
            and node.get("name") == "No project"
            or any(
                parent.get("name") in REGIONS
                or parent.get("name", "").endswith(
                    (" queued message", " queued messages")
                )
                or parent.get("role") in {"menu", "listbox", "list box", "dialog"}
                or parent.get("role") == "heading"
                and parent.get("name", "").startswith(
                    ("What should", "Choose a project")
                )
                for parent in ancestors
            )
        )
        if not scoped:
            continue
        for field in ("name", "value"):
            value = node.get(field)
            if value is not None and (value := " ".join(str(value).split())):
                result.add(value)
    return result


def compare(reference, native):
    expected, actual = texts(reference), texts(native)
    return {
        "reference_count": len(expected),
        "native_count": len(actual),
        "missing": sorted(expected - actual),
        "extra": sorted(actual - expected),
        "reference": sorted(expected),
        "native": sorted(actual),
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("run", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    results = {}
    for reference in sorted((args.run / "reference").glob("[0-9]*.json")):
        native = args.run / "native" / reference.name
        results[reference.stem] = (
            compare(json.loads(reference.read_text()), json.loads(native.read_text()))
            if native.exists()
            else {"error": "Native checkpoint missing"}
        )
    if not results:
        raise ValueError("No accessible snapshots found")
    differences = sum(
        len(item.get("missing", [])) + len(item.get("extra", []))
        for item in results.values()
    )
    report = {"differences": differences, "checkpoints": results}
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"differences": differences, "output": str(args.output)}))
    raise SystemExit(
        1 if differences or any("error" in item for item in results.values()) else 0
    )


if __name__ == "__main__":
    main()
