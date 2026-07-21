import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "validate-release-tag.py"
SPEC = importlib.util.spec_from_file_location("validate_release_tag", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


TAG = "v0.2.0"
SOURCE_SHA = "1" * 40
TAG_SHA = "2" * 40


def valid_ref() -> dict:
    return {
        "ref": f"refs/tags/{TAG}",
        "object": {"type": "tag", "sha": TAG_SHA},
    }


def valid_tag() -> dict:
    return {
        "sha": TAG_SHA,
        "tag": TAG,
        "message": "Chat2DB CLI\n-----BEGIN PGP SIGNATURE-----\nfake text",
        "object": {"type": "commit", "sha": SOURCE_SHA},
        "verification": {"verified": True, "reason": "valid"},
    }


class ReleaseTagValidationTests(unittest.TestCase):
    def test_accepts_github_verified_tag_at_exact_commit(self) -> None:
        MODULE.validate_release_tag(TAG, SOURCE_SHA, TAG_SHA, valid_ref(), valid_tag())

    def test_signature_looking_message_is_not_verification(self) -> None:
        tag = valid_tag()
        tag["verification"] = {"verified": False, "reason": "unsigned"}
        with self.assertRaisesRegex(ValueError, "did not verify"):
            MODULE.validate_release_tag(TAG, SOURCE_SHA, TAG_SHA, valid_ref(), tag)

    def test_rejects_lightweight_or_mismatched_tag_ref(self) -> None:
        for mutation in (
            {"ref": f"refs/tags/{TAG}", "object": {"type": "commit", "sha": TAG_SHA}},
            {"ref": "refs/tags/v0.2.1", "object": {"type": "tag", "sha": TAG_SHA}},
            {"ref": f"refs/tags/{TAG}", "object": {"type": "tag", "sha": "3" * 40}},
        ):
            with self.assertRaises(ValueError):
                MODULE.validate_release_tag(TAG, SOURCE_SHA, TAG_SHA, mutation, valid_tag())

    def test_rejects_tag_object_not_bound_to_source(self) -> None:
        for field, value in (
            ("sha", "3" * 40),
            ("tag", "v0.2.1"),
            ("object", {"type": "commit", "sha": "3" * 40}),
            ("object", {"type": "tag", "sha": SOURCE_SHA}),
        ):
            tag = valid_tag()
            tag[field] = value
            with self.assertRaises(ValueError):
                MODULE.validate_release_tag(TAG, SOURCE_SHA, TAG_SHA, valid_ref(), tag)


if __name__ == "__main__":
    unittest.main()
