#!/usr/bin/env python3
"""Validate a manually selected CI run against an open same-repository PR."""
import json
import os
import re
import sys
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


def validate_target(pr, repository, ref, sha):
    if not isinstance(pr, dict) or pr.get("state") != "open":
        raise ValueError("The requested pull request must be open.")
    base = pr.get("base") or {}
    head = pr.get("head") or {}
    if ((base.get("repo") or {}).get("full_name") != repository
            or (head.get("repo") or {}).get("full_name") != repository):
        raise ValueError("Manual PR verification accepts only same-repository pull requests.")
    if ref != "refs/heads/" + head.get("ref", ""):
        raise ValueError("Select the pull request's head branch when dispatching CI.")
    if sha != head.get("sha"):
        raise ValueError("This run does not target the PR's current head; dispatch a fresh run.")


def main():
    try:
        number = os.environ["PR_NUMBER"]
        if not re.fullmatch(r"[1-9][0-9]*", number):
            raise ValueError("The pull request number must be a positive integer.")
        repository = os.environ["GITHUB_REPOSITORY"]
        request = Request(
            f"https://api.github.com/repos/{repository}/pulls/{number}",
            headers={
                "Authorization": "Bearer " + os.environ["GH_TOKEN"],
                "Accept": "application/vnd.github+json",
                "X-GitHub-Api-Version": "2022-11-28",
            },
        )
        with urlopen(request, timeout=20) as response:
            body = response.read(1024 * 1024 + 1)
        if len(body) > 1024 * 1024:
            raise ValueError("Pull request metadata exceeds the expected size limit.")
        validate_target(json.loads(body), repository, os.environ["GITHUB_REF"], os.environ["GITHUB_SHA"])
        print(f"Verified PR #{number} at {os.environ['GITHUB_SHA']}.")
    except HTTPError as error:
        print(f"Cannot inspect the requested PR (HTTP {error.code}).", file=sys.stderr)
        return 1
    except URLError:
        print("Cannot reach GitHub to verify the requested PR.", file=sys.stderr)
        return 1
    except KeyError:
        print("Manual PR verification is missing its GitHub run context.", file=sys.stderr)
        return 1
    except ValueError as error:
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
