import argparse
import json
from collections import Counter
from pathlib import Path


def texts(nodes):
    result = Counter()
    for node in nodes:
        if node["role"] == "InlineTextBox":
            continue
        if not any(
            ancestor.get("role") == "dialog" and ancestor.get("name") == "Settings"
            for ancestor in node.get("ancestors", [])
        ):
            continue
        for field in ["name", "value"]:
            value = node.get(field)
            if value is not None:
                value = " ".join(str(value).split())
                if value:
                    result[value] = 1
    return result


def compare(reference, native):
    expected, actual = texts(reference), texts(native)
    return {
        "reference_count": sum(expected.values()),
        "native_count": sum(actual.values()),
        "missing": list((expected - actual).elements()),
        "extra": list((actual - expected).elements()),
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("run", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    results = {}
    for reference in sorted((args.run / "reference").glob("[0-9]*.json")):
        native = args.run / "native" / reference.name
        if not native.exists():
            results[reference.stem] = {"error": "Native checkpoint missing"}
            continue
        results[reference.stem] = compare(
            json.loads(reference.read_text()), json.loads(native.read_text())
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
