#!/usr/bin/env python3

import argparse
import json
import re
from pathlib import Path
from typing import Any


STABLE_TAG = re.compile(r"^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
OBJECT_ID = re.compile(r"^[0-9a-f]{40}(?:[0-9a-f]{24})?$")


def fail(message: str) -> None:
    raise ValueError(message)


def object_field(data: dict[str, Any], field: str, context: str) -> Any:
    value: Any = data
    for part in field.split("."):
        if not isinstance(value, dict) or part not in value:
            fail(f"{context} is missing {field}")
        value = value[part]
    return value


def validate_release_tag(
    tag: str,
    source_sha: str,
    local_tag_sha: str,
    ref_data: dict[str, Any],
    tag_data: dict[str, Any],
) -> None:
    if STABLE_TAG.fullmatch(tag) is None:
        fail(f"release tag must be stable SemVer: {tag}")
    for name, value in (("source SHA", source_sha), ("Tag object SHA", local_tag_sha)):
        if OBJECT_ID.fullmatch(value) is None:
            fail(f"{name} is not a canonical Git object id")

    if object_field(ref_data, "ref", "Tag ref") != f"refs/tags/{tag}":
        fail("GitHub Tag ref does not match the requested release Tag")
    if object_field(ref_data, "object.type", "Tag ref") != "tag":
        fail("release Tag must be an annotated Tag object")
    if object_field(ref_data, "object.sha", "Tag ref") != local_tag_sha:
        fail("GitHub and checkout disagree on the Tag object")

    if object_field(tag_data, "sha", "Tag object") != local_tag_sha:
        fail("GitHub returned a different Tag object")
    if object_field(tag_data, "tag", "Tag object") != tag:
        fail("annotated Tag name does not match the requested release Tag")
    if object_field(tag_data, "object.type", "Tag object") != "commit":
        fail("annotated release Tag must point directly to a commit")
    if object_field(tag_data, "object.sha", "Tag object") != source_sha:
        fail("release Tag does not point to the validated main commit")
    if object_field(tag_data, "verification.verified", "Tag object") is not True:
        reason = object_field(tag_data, "verification.reason", "Tag object")
        fail(f"GitHub did not verify the release Tag signature: {reason}")


def load_json(path: Path, context: str) -> dict[str, Any]:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"cannot read {context}: {error}")
    if not isinstance(data, dict):
        fail(f"{context} must be a JSON object")
    return data


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag", required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--local-tag-sha", required=True)
    parser.add_argument("--ref-json", required=True, type=Path)
    parser.add_argument("--tag-json", required=True, type=Path)
    args = parser.parse_args()
    try:
        validate_release_tag(
            args.tag,
            args.source_sha,
            args.local_tag_sha,
            load_json(args.ref_json, "GitHub Tag ref response"),
            load_json(args.tag_json, "GitHub Tag object response"),
        )
    except ValueError as error:
        raise SystemExit(f"error: {error}") from error
    print(f"verified signed release Tag {args.tag} at {args.source_sha}")


if __name__ == "__main__":
    main()
