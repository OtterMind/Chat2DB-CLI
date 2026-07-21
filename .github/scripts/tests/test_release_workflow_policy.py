from __future__ import annotations

from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[3]
WORKFLOW = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
SECRET_EXPRESSION = re.compile(r"\$\{\{\s*secrets(?:\.|\[)")


def parse_jobs() -> dict[str, str]:
    try:
        jobs_text = WORKFLOW.split("\njobs:\n", 1)[1]
    except IndexError as error:
        raise AssertionError("release workflow has no jobs section") from error
    matches = list(re.finditer(r"(?m)^  ([a-z0-9][a-z0-9-]*):\n", jobs_text))
    jobs: dict[str, str] = {}
    for index, match in enumerate(matches):
        end = matches[index + 1].start() if index + 1 < len(matches) else len(jobs_text)
        jobs[match.group(1)] = jobs_text[match.end() : end]
    return jobs


JOBS = parse_jobs()


def job(name: str) -> str:
    try:
        return JOBS[name]
    except KeyError as error:
        raise AssertionError(f"release job not found: {name}") from error


def job_needs(name: str) -> set[str]:
    block = job(name)
    match = re.search(r"(?m)^    needs:(?:[ ]+([^\n]+))?\n", block)
    if match is None:
        return set()
    if match.group(1):
        return {match.group(1).strip()}

    dependencies: set[str] = set()
    for line in block[match.end() :].splitlines():
        dependency = re.fullmatch(r"      - ([a-z0-9][a-z0-9-]*)", line)
        if dependency:
            dependencies.add(dependency.group(1))
            continue
        if not line.startswith("      "):
            break
    return dependencies


def job_property(name: str, property_name: str) -> str | None:
    match = re.search(
        rf"(?m)^    {re.escape(property_name)}:[ ]*([^\n]*)$",
        job(name),
    )
    return match.group(1).strip() if match else None


def transitively_needs(name: str, required: str, seen: set[str] | None = None) -> bool:
    if name == required:
        return True
    seen = set() if seen is None else seen
    if name in seen:
        return False
    seen.add(name)
    return any(
        transitively_needs(dependency, required, seen)
        for dependency in job_needs(name)
    )


class ReleaseWorkflowPolicyTests(unittest.TestCase):
    def test_release_is_manual_and_main_only(self) -> None:
        trigger = WORKFLOW.split("permissions:", 1)[0]
        self.assertIn("workflow_dispatch:", trigger)
        self.assertNotRegex(trigger, r"(?m)^  push:")
        self.assertNotIn("pull_request_target:", trigger)
        self.assertIn("type: choice", trigger)
        self.assertIn("- cdn-only", trigger)
        self.assertIn("- release", trigger)
        self.assertIn('if [ "$SOURCE_REF" != refs/heads/main ]; then', job("resolve"))

    def test_release_tag_is_cryptographically_bound_to_main(self) -> None:
        validate = job("validate")
        self.assertIn('test "$GITHUB_SHA" = "$(git rev-parse origin/main)"', validate)
        self.assertIn('gh api "repos/$GH_REPO/git/ref/tags/$RELEASE_TAG"', validate)
        self.assertIn('test "$tag_object_type" = tag', validate)
        self.assertIn('gh api "repos/$GH_REPO/git/tags/$tag_object_sha"', validate)
        self.assertIn("python3 .github/scripts/validate-release-tag.py", validate)
        self.assertIn('--source-sha "$GITHUB_SHA"', validate)
        self.assertIn('--local-tag-sha "$tag_object_sha"', validate)
        self.assertNotIn("BEGIN PGP", validate)
        self.assertNotIn("BEGIN SSH", validate)

    def test_stable_release_lookup_fails_closed(self) -> None:
        validate = job("validate")
        self.assertIn('existing_tags_file="$(mktemp)"', validate)
        self.assertIn('> "$existing_tags_file"', validate)
        self.assertIn('mapfile -t existing_tags < "$existing_tags_file"', validate)
        self.assertNotIn("mapfile -t existing_tags < <(", validate)

    def test_every_secret_job_is_validated_and_environment_scoped(self) -> None:
        expected_environments = {
            "notify-start": "release-notifications",
            "notify-summary": "release-notifications",
            "publish-cdn-only": "cdn-production",
            "publish-full-release": "cdn-production",
        }
        secret_jobs = {
            name for name, block in JOBS.items() if SECRET_EXPRESSION.search(block)
        }
        self.assertEqual(secret_jobs, set(expected_environments))
        self.assertEqual(
            sum(len(SECRET_EXPRESSION.findall(block)) for block in JOBS.values()),
            len(SECRET_EXPRESSION.findall(WORKFLOW)),
            "a Secret expression exists outside a parsed job",
        )

        for name, environment in expected_environments.items():
            block = job(name)
            self.assertTrue(
                transitively_needs(name, "validate"),
                f"secret job {name} does not depend on validate",
            )
            self.assertEqual(job_property(name, "environment"), environment)
            self.assertIn("ref: ${{ github.sha }}", block)
        self.assertEqual(
            job_property("notify-summary", "if"),
            "${{ always() && needs.validate.result == 'success' }}",
        )

    def test_publication_modes_are_mutually_exclusive(self) -> None:
        expected_conditions = {
            "draft-release": "if: ${{ needs.resolve.outputs.mode == 'release' }}",
            "publish-cdn-only": "if: ${{ needs.resolve.outputs.mode == 'cdn-only' }}",
            "publish-full-release": "if: ${{ needs.resolve.outputs.mode == 'release' }}",
        }
        for name, condition in expected_conditions.items():
            self.assertEqual(job_property(name, "if"), condition.removeprefix("if: "))
        self.assertIn("contents: read", job("publish-cdn-only"))
        self.assertNotIn("contents: write", job("publish-cdn-only"))
        self.assertIn("contents: write", job("publish-full-release"))


if __name__ == "__main__":
    unittest.main()
