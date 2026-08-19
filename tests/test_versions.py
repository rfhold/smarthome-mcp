"""Shared package-version invariants."""

import json
import re
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ROOT_SEMVER = re.compile(r"(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)")


def test_shared_versions_are_root_semver() -> None:
    """Cargo, Python, and the embedded integration use one root SemVer."""
    with (ROOT / "Cargo.toml").open("rb") as stream:
        cargo_version = tomllib.load(stream)["package"]["version"]
    with (ROOT / "pyproject.toml").open("rb") as stream:
        python_version = tomllib.load(stream)["project"]["version"]
    manifest = json.loads(
        (ROOT / "custom_components/smarthome_mcp/manifest.json").read_text(
            encoding="utf-8"
        )
    )

    versions = {cargo_version, python_version, manifest["version"]}
    assert len(versions) == 1
    assert ROOT_SEMVER.fullmatch(versions.pop()) is not None
