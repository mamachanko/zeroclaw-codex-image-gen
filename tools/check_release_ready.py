#!/usr/bin/env python3
"""Reject public releases while the active guest imports proposed host APIs."""
from pathlib import Path
import re
import sys


def require_supported_contract(root: Path):
    contract = root / "wit/direct/services.wit"
    if not contract.is_file():
        raise ValueError("Active host contract is missing; release readiness cannot be verified.")
    source = contract.read_text()
    if re.search(r"\bpackage\s+zeroclaw:proposed-", source):
        raise ValueError(
            "Public release blocked: the active WASM component requires proposed "
            "Codex auth/artifact services that stock ZeroClaw does not implement."
        )
    if not re.search(r"\bpackage\s+zeroclaw:[\w-]+@", source):
        raise ValueError("Active host contract has no recognized package declaration.")


def main():
    try:
        require_supported_contract(Path(__file__).resolve().parent.parent)
    except ValueError as error:
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
