"""Read and compare independent Git workspace fixtures without changing them."""

import hashlib
import subprocess
from pathlib import Path


def git(directory, *args):
    return subprocess.check_output(["git", "-C", str(directory), *args], timeout=10)


def snapshot(directory):
    directory = directory.resolve(strict=True)
    files = {}
    for name in git(
        directory, "ls-files", "-z", "-c", "-o", "--exclude-standard"
    ).split(b"\0"):
        if not name:
            continue
        relative = name.decode()
        path = directory / relative
        if path.is_symlink():
            content = str(path.readlink()).encode()
            kind = "symlink"
        elif path.is_file():
            if path.stat().st_size > 64 << 20:
                raise ValueError("Workspace fixture file exceeds 64 MiB")
            content = path.read_bytes()
            kind = "file"
        elif not path.exists():
            files[relative] = {"kind": "deleted"}
            continue
        else:
            raise ValueError(f"Unsupported workspace fixture entry: {relative}")
        files[relative] = {"kind": kind, "sha256": hashlib.sha256(content).hexdigest()}
    return {
        "branch": git(directory, "branch", "--show-current").decode().strip(),
        "head": git(directory, "rev-parse", "HEAD").decode().strip(),
        "refs": git(
            directory, "for-each-ref", "--format=%(refname) %(objectname)"
        ).decode(),
        "status": git(directory, "status", "--porcelain=v1", "-z").decode(),
        "files": files,
    }


def independent(reference, native):
    reference = reference.resolve(strict=True)
    native = native.resolve(strict=True)
    roots = [
        Path(
            git(directory, "rev-parse", "--path-format=absolute", "--git-common-dir")
            .decode()
            .strip()
        ).resolve()
        for directory in [reference, native]
    ]
    if (
        roots[0] == roots[1]
        or reference.is_relative_to(native)
        or native.is_relative_to(reference)
    ):
        raise ValueError(
            "Comparison workspaces must have independent directories and Git stores"
        )
    before = {"reference": snapshot(reference), "native": snapshot(native)}
    if before["reference"] != before["native"]:
        raise ValueError(
            "Workspace fixtures have different initial Git state or file contents"
        )
    return before


def observe(directories, expected):
    result = {target: snapshot(path) for target, path in directories.items()}
    failures = []
    for target, state in result.items():
        for key, value in expected.items():
            if state.get(key) != value:
                failures.append(
                    f"{target}: expected {key}={value!r}, got {state.get(key)!r}"
                )
    if result["reference"] != result["native"]:
        failures.append("The final workspace states differ")
    return {
        "status": "failed" if failures else "passed",
        "targets": result,
        "errors": failures,
    }
