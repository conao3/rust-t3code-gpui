import argparse
import asyncio
import hashlib
import json
import os
import signal
import subprocess
import sys
from datetime import UTC, datetime
from pathlib import Path

from env import TOOLS, main_tree


def scenarios(directory):
    return sorted(
        path
        for path in directory.glob("*.json")
        if "steps" in json.loads(path.read_text())
    )


def summarize(path, output, slot, error=None):
    scenario = json.loads(path.read_text())
    report_path = output / "report.json"
    report = json.loads(report_path.read_text()) if report_path.exists() else {}
    targets = scenario.get("targets", ["reference", "native"])
    counts = {}
    failures = []
    for target in targets:
        rows = report.get("targets", {}).get(target, [])
        counts[target] = {
            "passed": sum(row["status"] == "passed" for row in rows),
            "failed": sum(row["status"] == "failed" for row in rows),
            "not_run": len(scenario["steps"]) - len(rows),
            "total": len(scenario["steps"]),
        }
        failures.extend(
            {"target": target, "step": row["index"] + 1, "error": row["error"]}
            for row in rows
            if row["status"] == "failed"
        )
    preparation = output / "preparation"
    if not report and preparation.exists():
        rpc = preparation / "rpc.json"
        if rpc.exists():
            failures.extend(
                {"target": target, "step": "preparation", "error": result["error"]}
                for target, result in json.loads(rpc.read_text()).items()
                if result["status"] != "passed"
            )
        for target in targets:
            for actions in sorted((preparation / target).glob("*/actions.json")):
                failures.extend(
                    {
                        "target": target,
                        "step": f"preparation/{row['index'] + 1}",
                        "error": row["error"],
                    }
                    for row in json.loads(actions.read_text())
                    if row["status"] != "passed"
                )
    reference_failed = "reference" in counts and (
        counts["reference"]["passed"] != counts["reference"]["total"]
    )
    workspace = report.get("workspace_after")
    server_results = {
        key: report[key] for key in ["git_result", "automation_result"] if key in report
    }
    category = (
        "scenario_or_fixture_error"
        if any(failure["target"] == "reference" for failure in failures)
        else "preparation_error"
        if error and not report
        else "scenario_or_fixture_error"
        if reference_failed
        else "native_failure"
        if failures
        else "workspace_failure"
        if workspace and workspace["status"] != "passed"
        else "server_state_failure"
        if any(result["status"] != "passed" for result in server_results.values())
        else "visual_difference"
        if any(item["status"] != "passed" for item in report.get("visual", []))
        else "comparison_failure"
        if report.get("status") != "passed"
        else "passed"
    )
    return {
        "scenario": path.name,
        "kind": "native" if targets == ["native"] else "comparison",
        "slot": slot,
        "category": category,
        "counts": counts,
        "failures": failures,
        "error": error if not report else None,
        "visual": report.get("visual", []),
        "workspace": workspace,
        "server_results": server_results,
        "status": report.get("status"),
        "report": str(report_path) if report_path.exists() else None,
        "log": str(output / "run.log"),
    }


def markdown(rows):
    lines = [
        "| Scenario | Run | Slot | Kind | Reference pass/fail/not run | Native pass/fail/not run | Result | Failed step / error | Report |",
        "|---|---:|---:|---|---:|---:|---|---|---|",
    ]
    for row in rows:
        counts = [
            "/".join(
                str(row["counts"][target][key])
                for key in ["passed", "failed", "not_run"]
            )
            if target in row["counts"]
            else "—"
            for target in ["reference", "native"]
        ]
        failure = "; ".join(
            f"{item['target']} {item['step']}: {item['error'].split('; found=', 1)[0]}"
            for item in row["failures"]
        ) or (row["error"] or "")
        failure = failure.replace("|", "\\|").replace("\n", " ")
        report = row["report"] or row["log"]
        lines.append(
            f"| {row['scenario']} | {row.get('iteration', 1)} | {row['slot']} | {row['kind']} | {' | '.join(counts)} | {row['category']} | {failure} | [report]({report}) |"
        )
    return "\n".join(lines) + "\n"


async def command(arguments, log):
    with log.open("ab") as stream:
        process = await asyncio.create_subprocess_exec(
            sys.executable,
            *map(str, arguments),
            stdout=stream,
            stderr=stream,
            start_new_session=True,
        )
        try:
            return await process.wait()
        except asyncio.CancelledError:
            os.killpg(process.pid, signal.SIGTERM)
            await process.wait()
            raise


async def run(args):
    available = scenarios(args.scenarios)
    unknown = set(args.scenario or []) - {path.stem for path in available}
    if unknown:
        raise ValueError(f"Unknown scenarios: {sorted(unknown)}")
    args.output.mkdir(parents=True, exist_ok=False)
    with args.native_binary.open("rb") as binary:
        digest = hashlib.file_digest(binary, "sha256").hexdigest()
    (args.output / "run.json").write_text(
        json.dumps(
            {
                "started": datetime.now(UTC).isoformat(),
                "commit": (
                    await asyncio.to_thread(
                        subprocess.check_output,
                        ["git", "-C", str(TOOLS), "rev-parse", "HEAD"],
                        text=True,
                    )
                ).strip(),
                "native_binary": str(args.native_binary),
                "native_sha256": digest,
                "slots": args.slots,
                "repeat": args.repeat,
            },
            indent=2,
        )
        + "\n"
    )
    queue = asyncio.Queue()
    for scenario in available:
        if not args.scenario or scenario.stem in args.scenario:
            for iteration in range(1, args.repeat + 1):
                queue.put_nowait((scenario, iteration))
    rows = []

    def save():
        ordered = sorted(rows, key=lambda row: (row["scenario"], row["iteration"]))
        (args.output / "results.json").write_text(json.dumps(ordered, indent=2) + "\n")
        (args.output / "results.md").write_text(markdown(ordered))

    async def worker(slot):
        slot_path = main_tree() / ".claude-dev/parity/slots" / str(slot)
        while not queue.empty():
            path, iteration = queue.get_nowait()
            scenario = json.loads(path.read_text())
            output = args.output / (
                f"{path.stem}-{iteration:02d}" if args.repeat > 1 else path.stem
            )
            output.mkdir()
            log = output / "run.log"
            code = await command([TOOLS / "env.py", "down", "--slot", slot], log)
            if code:
                rows.append(
                    summarize(path, output, slot, "Slot shutdown failed")
                    | {"iteration": iteration}
                )
                save()
                continue
            launch = [
                TOOLS / "env.py",
                "up",
                "--slot",
                slot,
                "--native-binary",
                args.native_binary,
                "--theme",
                scenario["theme"],
                "--stream-ms",
                scenario.get("environment", {}).get("stream_ms", 15000),
            ]
            if fixture := scenario.get("environment", {}).get("pr_rpc_fixture"):
                launch.extend(["--pr-rpc-fixture", TOOLS / fixture])
            code = await command(launch, log)
            if code:
                rows.append(
                    summarize(path, output, slot, "Slot startup failed")
                    | {"iteration": iteration}
                )
                save()
                continue
            run_output = output / "comparison"
            code = await command(
                [
                    TOOLS / "run.py",
                    "--manifest",
                    slot_path / "manifest.json",
                    "--scenario",
                    path,
                    "--output",
                    run_output,
                ],
                log,
            )
            row = summarize(
                path, run_output, slot, f"Runner exited {code}" if code else None
            )
            row["iteration"] = iteration
            row["log"] = str(log)
            rows.append(row)
            save()
            print(json.dumps(row), flush=True)

    try:
        async with asyncio.TaskGroup() as tasks:
            for slot in args.slots:
                tasks.create_task(worker(slot))
    finally:
        for slot in args.slots:
            await command(
                [TOOLS / "env.py", "down", "--slot", slot],
                args.output / f"slot-{slot}.log",
            )
    return 0 if rows and all(row["category"] == "passed" for row in rows) else 1


def main():
    parser = argparse.ArgumentParser(
        description="Run each parity scenario in a fresh isolated slot"
    )
    parser.add_argument("--slots", type=int, nargs="+", required=True)
    parser.add_argument("--native-binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--scenarios", type=Path, default=TOOLS)
    parser.add_argument("--scenario", action="append")
    parser.add_argument("--repeat", type=int, default=1)
    args = parser.parse_args()
    if args.repeat < 1:
        parser.error("Repeat must be positive")
    if len(set(args.slots)) != len(args.slots) or any(
        not 1 <= slot <= 100 for slot in args.slots
    ):
        parser.error("Slots must be unique numbers from 1 to 100")
    args.native_binary = args.native_binary.resolve(strict=True)
    args.output = args.output.resolve()
    raise SystemExit(asyncio.run(run(args)))


if __name__ == "__main__":
    main()
