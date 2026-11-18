#!/usr/bin/env python3
"""Keep development builds out of the public release workflow."""
from pathlib import Path
import re
import sys
import tomllib


def require_release_version(root: Path):
    manifest = root / "manifest.toml"
    if not manifest.is_file():
        raise ValueError("Release manifest is missing")
    version = tomllib.loads(manifest.read_text()).get("version")
    if not isinstance(version, str) or not re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)", version):
        raise ValueError("Public release blocked: manifest version must be a stable version")


if __name__ == "__main__":
    try:
        require_release_version(Path(__file__).resolve().parent.parent)
    except ValueError as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1)
