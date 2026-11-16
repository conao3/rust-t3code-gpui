"""Execute the same real-input scenario against Electron and GPUI."""

import argparse
import asyncio
import json
import logging
import time
from pathlib import Path

from cdp import Cdp
from native import Native
from PIL import Image, ImageChops, ImageStat

logger = logging.getLogger(__name__)


async def invoke(app, method, *args):
    result = getattr(app, method)(*args)
    if hasattr(result, "__await__"):
        return await result
    return result


def matches(nodes, selector):
    roles = {"entry": "textbox", "radio menu item": "menuitemradio"}
    return [
        node
        for node in nodes
        if all(
            (roles.get(node["role"], node["role"]) if key == "role" else node.get(key))
            == value
            for key, value in selector.items()
        )
    ]


async def wait_for(app, selector, count=None, value=None, timeout=10):
    deadline = time.monotonic() + timeout
    while True:
        nodes = await invoke(app, "tree")
        found = matches(nodes, selector)
        if (count is None and found or count is not None and len(found) == count) and (
            value is None or len(found) == 1 and found[0].get("value") == value
        ):
            return nodes
        if time.monotonic() >= deadline:
            raise AssertionError(
                f"Timed out: {selector}, count={count}, value={value!r}; found={found}"
            )
        await asyncio.sleep(0.1)


async def execute(app, target, scenario, directory):
    directory.mkdir(parents=True, exist_ok=True)
    results = []
    for index, step in enumerate(scenario["steps"]):
        result = {"index": index, "step": step, "status": "failed"}
        try:
            action = step["action"]
            if action == "click":
                selector = step.get(target, step.get("selector"))
                await wait_for(app, selector)
                role = (
                    "entry"
                    if target == "native" and selector["role"] == "textbox"
                    else selector["role"]
                )
                await invoke(app, "click", role, selector["name"])
            elif action in ["press", "type"]:
                await invoke(app, action, step["value"])
            elif action == "checkpoint":
                nodes = await wait_for(
                    app, step["selector"], step.get("count"), step.get("value")
                )
                await asyncio.sleep(scenario["settle_ms"] / 1000)
                nodes = await wait_for(
                    app, step["selector"], step.get("count"), step.get("value")
                )
                (directory / f"{index:02d}.json").write_text(
                    json.dumps(nodes, ensure_ascii=False, indent=2) + "\n"
                )
                await invoke(app, "screenshot", directory / f"{index:02d}.png")
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
        results.append(result)
        if result["status"] == "failed":
            break
    (directory / "actions.json").write_text(
        json.dumps(results, ensure_ascii=False, indent=2) + "\n"
    )
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
    parser.add_argument("--endpoint", required=True)
    parser.add_argument("--session", type=Path, required=True)
    parser.add_argument("--scenario", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    scenario = json.loads(args.scenario.read_text())
    if args.output.exists():
        raise RuntimeError(
            "Output directory already exists; preserve each run separately"
        )
    args.output.mkdir(parents=True)
    native = Native(args.session)
    width, height = scenario["viewport"]
    async with Cdp(args.endpoint) as reference:
        await reference.call(
            "Emulation.setEmulatedMedia",
            features=[{"name": "prefers-color-scheme", "value": scenario["theme"]}],
        )
        await reference.call(
            "Emulation.setDeviceMetricsOverride",
            width=width,
            height=height,
            deviceScaleFactor=1,
            mobile=False,
        )
        await native.viewport(width, height)
        reports = {
            "reference": await execute(
                reference, "reference", scenario, args.output / "reference"
            )
        }
        reports["native"] = await execute(
            native, "native", scenario, args.output / "native"
        )
    comparisons = []
    for index, step in enumerate(scenario["steps"]):
        if step["action"] != "checkpoint":
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
        "targets": reports,
        "visual": comparisons,
        "status": "passed"
        if all(r["status"] == "passed" for rows in reports.values() for r in rows)
        and comparisons
        and all(r["status"] == "passed" for r in comparisons)
        else "failed",
    }
    (args.output / "report.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    )
    print(json.dumps({"status": report["status"], "output": str(args.output)}))
    raise SystemExit(0 if report["status"] == "passed" else 1)


if __name__ == "__main__":
    asyncio.run(main())
