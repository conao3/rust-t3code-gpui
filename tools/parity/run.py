"""Execute the same real-input scenario against Electron and GPUI."""

import argparse
import asyncio
import json
import logging
import sys
import time
from collections import Counter
from pathlib import Path

from cdp import Cdp
from fixture_validation import distinct, environment_descriptor, origin
from native import Native
from PIL import Image, ImageChops, ImageStat
from prepare import prepare
from workspace_fixture import independent, observe

logger = logging.getLogger(__name__)


def assert_visible_regions(path, regions):
    with Image.open(path).convert("RGB") as image:
        for region in regions:
            bounds = region["bounds"]
            if (
                len(bounds) != 4
                or any(type(value) is not int for value in bounds)
                or not 0 <= bounds[0] < bounds[2] <= image.width
                or not 0 <= bounds[1] < bounds[3] <= image.height
            ):
                raise ValueError(f"Invalid visible image region: {bounds}")
            minimum = region["minimum_pixels"]
            if type(minimum) is not int or minimum <= 0:
                raise ValueError(
                    "A visible image assertion needs a positive pixel count"
                )
            pixels = list(image.crop(bounds).get_flattened_data())
            background = Counter(pixels).most_common(1)[0][0]
            contrasting = sum(
                max(abs(a - b) for a, b in zip(pixel, background)) >= 64
                for pixel in pixels
            )
            if contrasting < minimum:
                raise AssertionError(
                    f"Image region {bounds} has {contrasting} contrasting pixels; expected at least {region['minimum_pixels']}"
                )


async def invoke(app, method, *args):
    result = getattr(app, method)(*args)
    if hasattr(result, "__await__"):
        return await result
    return result


def matches(nodes, selector):
    roles = {
        "entry": "textbox",
        "radio menu item": "menuitemradio",
        "combo box": "combobox",
        "landmark": "region",
    }
    index = selector.get("index")
    if index is not None and (type(index) is not int or index < 0):
        raise ValueError("Selector index must be a nonnegative integer")
    found = [
        node
        for node in nodes
        if all(
            (roles.get(node["role"], node["role"]) if key == "role" else node.get(key))
            == value
            for key, value in selector.items()
            if key not in ["index", "ancestor"]
        )
        and (
            "ancestor" not in selector
            or matches(node.get("ancestors", []), selector["ancestor"])
        )
    ]
    return found if index is None else found[index : index + 1]


async def wait_for(
    app, selector, count=None, value=None, timeout=10, *, text_contains=None
):
    deadline = time.monotonic() + timeout
    while True:
        nodes = await invoke(app, "tree")
        found = matches(nodes, selector)
        if (
            (count is None and found or count is not None and len(found) == count)
            and (value is None or len(found) == 1 and found[0].get("value") == value)
            and (
                text_contains is None
                or any(
                    text_contains in (node.get("value") or node.get("name", ""))
                    for node in found
                )
            )
        ):
            return nodes
        if time.monotonic() >= deadline:
            raise AssertionError(
                f"Timed out: {selector}, count={count}, value={value!r}, text_contains={text_contains!r}; found={found}"
            )
        await asyncio.sleep(0.1)


async def execute(app, target, scenario, directory):
    directory.mkdir(parents=True, exist_ok=True)
    results = []
    for index, step in enumerate(scenario["steps"]):
        result = {"index": index, "step": step, "status": "failed"}
        started = time.monotonic()
        try:
            action = step["action"]
            if action in ["click", "focus"]:
                selector = step.get(target, step.get("selector"))
                nodes = await wait_for(app, selector)
                chosen = matches(nodes, selector)[0]
                index = [
                    node
                    for node in nodes
                    if node["role"] == chosen["role"] and node["name"] == chosen["name"]
                ].index(chosen)
                role = selector["role"]
                if target == "native":
                    role = chosen["role"]
                await invoke(
                    app,
                    ("focus_snapshot" if action == "focus" else "click_snapshot")
                    if target == "native"
                    else ("focus_control" if action == "focus" else "click"),
                    role,
                    selector["name"],
                    index,
                )
            elif action in ["press", "type"]:
                await invoke(app, action, step["value"])
            elif action == "modifier":
                await invoke(app, "modifier", step["key"], step["held"])
            elif action == "pointer":
                await invoke(
                    app,
                    "pointer",
                    step["kind"],
                    step["x"],
                    step["y"],
                    step.get("button", "left"),
                )
            elif action == "checkpoint":
                selector = step.get(target, step["selector"])
                nodes = await wait_for(
                    app,
                    selector,
                    step.get("count"),
                    step.get("value"),
                    text_contains=step.get("text_contains"),
                )
                await asyncio.sleep(scenario["settle_ms"] / 1000)
                nodes = await wait_for(
                    app,
                    selector,
                    step.get("count"),
                    step.get("value"),
                    text_contains=step.get("text_contains"),
                )
                (directory / f"{index:02d}.json").write_text(
                    json.dumps(nodes, ensure_ascii=False, indent=2) + "\n"
                )
                if step.get("screenshot", True) or step.get("visible_regions"):
                    await invoke(app, "screenshot", directory / f"{index:02d}.png")
                if regions := step.get("visible_regions"):
                    assert_visible_regions(directory / f"{index:02d}.png", regions)
            else:
                raise ValueError(f"Unknown action: {action}")
            result["status"] = "passed"
        except Exception as error:
            logger.exception("Scenario action failed")
            result["error"] = str(error)
            try:
                await invoke(app, "screenshot", directory / f"{index:02d}-failed.png")
            except Exception as capture_error:
                logger.exception("Failure screenshot failed")
                result["capture_error"] = str(capture_error)
        result["seconds"] = time.monotonic() - started
        results.append(result)
        if result["status"] == "failed":
            break
    (directory / "actions.json").write_text(
        json.dumps(results, ensure_ascii=False, indent=2) + "\n"
    )
    await invoke(app, "release_modifiers")
    return results


def compare(reference, native, output):
    with (
        Image.open(reference).convert("RGB") as left,
        Image.open(native).convert("RGB") as right,
    ):
        if left.size != right.size:
            return {
                "status": "failed",
                "error": f"Viewport mismatch: {left.size} != {right.size}",
            }
        difference = ImageChops.difference(left, right)
        difference.save(output.with_suffix(".diff.png"))
        pair = Image.new("RGB", (left.width * 2, left.height))
        pair.paste(left, (0, 0))
        pair.paste(right, (left.width, 0))
        pair.save(output.with_suffix(".pair.png"))
        mean = sum(ImageStat.Stat(difference).mean) / 3
        changed = difference.convert("L").point(lambda p: 255 if p > 16 else 0)
        changed_fraction = ImageStat.Stat(changed).mean[0] / 255
        return {
            "status": "passed" if changed_fraction <= 0.005 and mean <= 1 else "failed",
            "mean_channel_difference": mean,
            "changed_pixel_fraction": changed_fraction,
            "width": left.width,
            "height": left.height,
        }


async def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--endpoint")
    parser.add_argument("--session", type=Path)
    parser.add_argument("--scenario", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--reference-workspace", type=Path)
    parser.add_argument("--native-workspace", type=Path)
    args = parser.parse_args()
    manifest = None
    if args.manifest:
        if args.endpoint or args.session:
            parser.error("Use --manifest or --endpoint/--session")
        manifest = json.loads(args.manifest.read_text())
        args.endpoint = manifest["endpoint"]
        args.session = Path(manifest["session"])
    elif not args.endpoint or not args.session:
        parser.error("Provide --manifest or both --endpoint and --session")
    scenario = json.loads(args.scenario.read_text())
    if args.output.exists():
        raise RuntimeError(
            "Output directory already exists; preserve each run separately"
        )
    args.output.mkdir(parents=True)
    if manifest:
        (args.output / "environment.json").write_text(
            json.dumps(
                {key: manifest[key] for key in ["slot", "generation", "session"]},
                indent=2,
            )
            + "\n"
        )
    if scenario.get("prepare"):
        if manifest is None:
            raise ValueError("Scenario preparation requires --manifest")
        await prepare(manifest, scenario["prepare"], args.output / "preparation")
    targets = scenario.get("targets", ["reference", "native"])
    if targets not in [["native"], ["reference", "native"]]:
        raise ValueError("Scenario targets must be native or reference and native")
    if manifest and "workspace" in scenario:
        args.reference_workspace = Path(manifest["workspaces"]["reference"])
        args.native_workspace = Path(manifest["workspaces"]["native"])
    workspaces = None
    workspace_before = None
    if "workspace" in scenario:
        if args.reference_workspace is None or args.native_workspace is None:
            raise ValueError("This scenario requires both independent workspace paths")
        workspaces = {
            "reference": args.reference_workspace,
            "native": args.native_workspace,
        }
        workspace_before = independent(*workspaces.values())
        for target, state in workspace_before.items():
            for key, value in scenario.get("workspace_initial", {}).items():
                if state.get(key) != value:
                    raise ValueError(f"{target}: initial {key} must be {value!r}")
    for index, declaration in enumerate(scenario.get("prepare", [])):
        helper = declaration["helper"]
        if helper == "actions":
            for target in targets:
                directory = args.output / "preparation" / target / str(index)
                directory.mkdir(parents=True)
                with (directory / "setup.log").open("w") as log:
                    process = await asyncio.create_subprocess_exec(
                        sys.executable,
                        str(Path(__file__).with_name("prepare.py")),
                        "--manifest",
                        str(args.manifest),
                        "--scenario",
                        str(args.scenario),
                        "--target",
                        target,
                        "--index",
                        str(index),
                        "--output",
                        str(directory),
                        stdout=log,
                        stderr=log,
                    )
                    if await process.wait():
                        raise RuntimeError(
                            f"{target} GUI preparation failed; see {directory}"
                        )
        elif helper == "reload_clients":
            async with Cdp(args.endpoint) as reference:
                await reference.call("Page.reload")
                await wait_for(reference, {"role": "button", "name": "Settings"})
            process = await asyncio.create_subprocess_exec(
                sys.executable,
                str(Path(__file__).with_name("env.py")),
                "restart",
                "--slot",
                str(manifest["slot"]),
                "--target",
                "native",
            )
            if await process.wait():
                raise RuntimeError("Fixture client restart failed")
    native = Native(args.session)
    width, height = scenario["viewport"]
    async with Cdp(args.endpoint) as reference:
        reference_origin = origin(await reference.server_origin())
        descriptor = await asyncio.to_thread(environment_descriptor, reference_origin)
        distinct(
            reference_origin,
            native.session.get("environments", []),
            descriptor["environmentId"],
        )
        await reference.call(
            "Emulation.setEmulatedMedia",
            features=[{"name": "prefers-color-scheme", "value": scenario["theme"]}],
        )
        reference_appearance = await reference.appearance()
        if reference_appearance != scenario["theme"]:
            raise ValueError(
                f"Reference appearance is {reference_appearance!r}, expected {scenario['theme']!r}; "
                "set its appearance through Settings before comparing"
            )
        await reference.viewport(width, height, native.session["display"])
        await native.viewport(width, height)
        await reference.call("Page.bringToFront")
        reports = {}
        for target in targets:
            app = reference if target == "reference" else native
            reports[target] = await execute(app, target, scenario, args.output / target)
    comparisons = []
    for index, step in enumerate(scenario["steps"]):
        if (
            "reference" not in targets
            or step["action"] != "checkpoint"
            or not (step.get("screenshot", True) or step.get("visible_regions"))
        ):
            continue
        left, right = [
            args.output / target / f"{index:02d}.png"
            for target in ["reference", "native"]
        ]
        comparisons.append(
            {
                "index": index,
                **(
                    compare(left, right, args.output / f"{index:02d}")
                    if left.exists() and right.exists()
                    else {"status": "failed", "error": "Missing checkpoint"}
                ),
            }
        )
    report = {
        "scenario": scenario,
        "fixtures": {
            "reference": {
                "origin": reference_origin,
                "environment_id": descriptor["environmentId"],
                "appearance": reference_appearance,
            },
            "native": native.session["environments"],
        },
        "targets": reports,
        "visual": comparisons,
        "status": "passed"
        if all(r["status"] == "passed" for rows in reports.values() for r in rows)
        and any(step["action"] == "checkpoint" for step in scenario["steps"])
        and all(r["status"] == "passed" for r in comparisons)
        else "failed",
    }
    if workspaces is not None:
        report["workspace_before"] = workspace_before
        started = time.monotonic()
        while True:
            observed = await asyncio.to_thread(
                observe, workspaces, scenario["workspace"]
            )
            if observed["status"] == "passed" or time.monotonic() - started >= 10:
                break
            await asyncio.sleep(0.2)
        report["workspace_after"] = observed
        report["workspace_after"]["wait_seconds"] = time.monotonic() - started
        if report["workspace_after"]["status"] == "failed":
            report["status"] = "failed"
    if "git_result" in scenario:
        if not args.manifest:
            parser.error("Git fixture assertions require --manifest")
        from panel_fixture import verify_git

        report["git_result"] = await verify_git(manifest, scenario["git_result"])
        if report["git_result"]["status"] != "passed":
            report["status"] = "failed"
    if "automation_result" in scenario:
        if not args.manifest:
            parser.error("Automation fixture assertions require --manifest")
        from panel_fixture import verify_automation

        report["automation_result"] = await verify_automation(
            manifest, scenario["automation_result"]
        )
        if report["automation_result"]["status"] != "passed":
            report["status"] = "failed"
    (args.output / "report.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    )
    print(json.dumps({"status": report["status"], "output": str(args.output)}))
    raise SystemExit(0 if report["status"] == "passed" else 1)


if __name__ == "__main__":
    asyncio.run(main())
