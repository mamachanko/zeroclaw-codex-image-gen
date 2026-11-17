import copy
import io
import json
import os
import unittest
from contextlib import redirect_stderr, redirect_stdout
from unittest.mock import patch
from urllib.error import HTTPError

from verify_ci_target import main, validate_target

REPOSITORY = "mamachanko/zeroclaw-codex-image-gen"
SHA = "a" * 40
BRANCH = "renovate/rust-dependencies"
PR = {
    "state": "open",
    "base": {"repo": {"full_name": REPOSITORY}},
    "head": {"repo": {"full_name": REPOSITORY}, "ref": BRANCH, "sha": SHA},
}


class TargetTests(unittest.TestCase):
    def test_accepts_the_current_open_pr_head(self):
        validate_target(PR, REPOSITORY, "refs/heads/" + BRANCH, SHA)

    def test_rejects_wrong_branch_and_stale_head(self):
        for ref, sha in [("refs/heads/main", SHA), ("refs/heads/" + BRANCH, "b" * 40)]:
            with self.subTest(ref=ref, sha=sha), self.assertRaises(ValueError):
                validate_target(PR, REPOSITORY, ref, sha)

    def test_rejects_closed_prs_forks_and_deleted_heads(self):
        for change in ["closed", "fork", "deleted", "other-base"]:
            data = copy.deepcopy(PR)
            if change == "closed":
                data["state"] = "closed"
            elif change == "fork":
                data["head"]["repo"]["full_name"] = "other/fork"
            elif change == "deleted":
                data["head"]["repo"] = None
            else:
                data["base"]["repo"]["full_name"] = "other/base"
            with self.subTest(change=change), self.assertRaises(ValueError):
                validate_target(data, REPOSITORY, "refs/heads/" + BRANCH, SHA)

    def run_main(self, number="7", response=None, error=None):
        env = {"PR_NUMBER": number, "GITHUB_REPOSITORY": REPOSITORY,
               "GH_TOKEN": "fixture-secret", "GITHUB_REF": "refs/heads/" + BRANCH, "GITHUB_SHA": SHA}
        output, errors = io.StringIO(), io.StringIO()
        with patch.dict(os.environ, env, clear=True), redirect_stdout(output), redirect_stderr(errors):
            with patch("verify_ci_target.urlopen", side_effect=error) as request:
                request.return_value.__enter__.return_value.read.return_value = json.dumps(response or PR).encode()
                status = main()
        self.assertNotIn("fixture-secret", output.getvalue() + errors.getvalue())
        return status, request, output.getvalue(), errors.getvalue()

    def test_cli_fetches_only_the_requested_pr_and_reports_success(self):
        status, request, output, _ = self.run_main()
        self.assertEqual(status, 0)
        self.assertEqual(request.call_args.args[0].full_url, f"https://api.github.com/repos/{REPOSITORY}/pulls/7")
        self.assertIn("Verified PR #7", output)

    def test_rejects_invalid_numbers_before_api_access(self):
        for number in ["", "0", "-1", "../7", "7?state=all", "one"]:
            with self.subTest(number=number):
                status, request, _, _ = self.run_main(number=number)
                self.assertEqual(status, 1)
                request.assert_not_called()

    def test_cli_fails_for_api_denial_and_stale_pr(self):
        status, _, _, message = self.run_main(error=HTTPError("https://api.github.com", 403, "Forbidden", {}, None))
        self.assertEqual(status, 1)
        self.assertIn("HTTP 403", message)
        data = copy.deepcopy(PR)
        data["head"]["sha"] = "b" * 40
        status, _, _, _ = self.run_main(response=data)
        self.assertEqual(status, 1)


if __name__ == "__main__":
    unittest.main()
