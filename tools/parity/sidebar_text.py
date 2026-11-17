import argparse
import json
from pathlib import Path

TITLEBAR_PANEL_CONTROLS = {
    "Toggle thread details panel",
    "Toggle terminal drawer",
    "Toggle right panel",
    "Changes",
}
WINDOW_CONTROLS = {
    "Minimize window",
    "Maximize or restore window",
    "Close window",
}


def texts(nodes, target):
    content = next(
        (node.get("bounds") for node in nodes if node["name"] == "Application content"),
        None,
    )
    sidebar = next(
        (node.get("bounds") for node in nodes if node["name"] == "Threads"),
        None,
    )
    result = set()
    for node in nodes:
        if node["role"] == "InlineTextBox":
            continue
        ancestors = node.get("ancestors", [])
        names = {ancestor.get("name") for ancestor in ancestors}
        popup = any(
            item["role"] in {"menu", "popup menu", "listbox", "list box"}
            or item["role"] == "dialog"
            and item.get("name") == "Command palette"
            for item in [node, *ancestors]
        )
        if "T3 Code home" in names:
            continue
        if target == "reference":
            in_scope = bool(names & {"Threads", "Thread breadcrumb"}) or any(
                ancestor["role"] == "sectionheader" for ancestor in ancestors
            )
            in_scope |= node["name"] in {
                "Threads",
                "Thread breadcrumb",
                "Resize Sidebar",
                "Toggle main sidebar",
            }
            titlebar = "Threads" not in names or node["name"] in TITLEBAR_PANEL_CONTROLS
        else:
            if content is None or sidebar is None:
                raise ValueError("Native application/sidebar bounds missing")
            bounds = node.get("bounds")
            if not popup and (not bounds or "Application content" not in names):
                continue
            titlebar = bool(
                bounds
                and content["y"] <= bounds["y"] < content["y"] + 40
                and "Conversation" not in names
            )
            in_scope = titlebar or bool(
                bounds and bounds["x"] < sidebar["x"] + sidebar["width"]
            )
            in_scope |= node["name"] == "Resize Sidebar"
        in_scope |= popup
        if not in_scope or node["name"] in WINDOW_CONTROLS:
            continue
        if titlebar and node["name"] in TITLEBAR_PANEL_CONTROLS:
            continue
        for field in ["name", "value"]:
            value = node.get(field)
            if value is not None and (value := " ".join(str(value).split())):
                result.add(value)
    return result


def compare(reference, native):
    expected = texts(reference, "reference")
    actual = texts(native, "native")
    return {
        "reference_count": len(expected),
        "native_count": len(actual),
        "missing": sorted(expected - actual),
        "extra": sorted(actual - expected),
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
    report = {
        "differences": differences,
        "excluded": {
            "branding": "Descendants of T3 Code home; the link name is compared",
            "native_window_controls": sorted(WINDOW_CONTROLS),
            "titlebar_panel_controls_owned_by_another_domain": sorted(
                TITLEBAR_PANEL_CONTROLS
            ),
        },
        "checkpoints": results,
    }
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"differences": differences, "output": str(args.output)}))
    raise SystemExit(
        1 if differences or any("error" in item for item in results.values()) else 0
    )


if __name__ == "__main__":
    main()
