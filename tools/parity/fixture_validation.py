"""Reject comparisons that share a backend or start without authenticated fixtures."""

import json
import subprocess
from urllib.parse import urlsplit
from urllib.request import urlopen


def origin(url):
    parsed = urlsplit(url)
    if parsed.scheme not in ["http", "https"] or not parsed.hostname:
        raise ValueError("A fixture must have an HTTP origin")
    host = parsed.hostname.lower()
    if host in ["localhost", "::1"]:
        host = "127.0.0.1"
    if ":" in host:
        host = "[" + host + "]"
    port = parsed.port or (443 if parsed.scheme == "https" else 80)
    return f"{parsed.scheme}://{host}:{port}"


def distinct(reference_origin, environments, reference_id=None):
    if not environments:
        raise ValueError("Native fixture origins are missing from the session manifest")
    for environment in environments:
        if origin(reference_origin) == origin(environment["origin"]):
            raise ValueError("Both applications point at the same server origin")
        if reference_id and environment.get("environment_id") == reference_id:
            raise ValueError("Both applications report the same server identity")


def inspect_environments(binary, config):
    settings = json.loads(config.read_text())
    environments = []
    for environment in settings["environments"]:
        result = subprocess.run(
            [
                str(binary.resolve()),
                "--config",
                str(config.resolve()),
                "environment",
                "check-environment",
                "--label",
                environment["label"],
            ],
            capture_output=True,
            text=True,
            timeout=30,
            check=True,
        )
        checked = json.loads(result.stdout)
        if checked.get("Authenticated") is not True:
            raise RuntimeError(
                "A native fixture did not authenticate; comparison startup rejected"
            )
        environments.append(
            {
                "origin": origin(environment["httpBaseUrl"]),
                "environment_id": checked["EnvironmentId"],
            }
        )
    return environments


def environment_descriptor(server_origin):
    with urlopen(server_origin + "/.well-known/t3/environment", timeout=10) as response:
        return json.load(response)
