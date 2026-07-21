#!/usr/bin/env python3

import re
from pathlib import Path


USES = re.compile(r"^\s*-?\s*uses:\s*([^\s#]+)")
FULL_SHA = re.compile(r"^[0-9a-f]{40}$")


def main() -> None:
    failures = []
    workflow_files = sorted(Path(".github").rglob("*.yml")) + sorted(
        Path(".github").rglob("*.yaml")
    )
    for path in workflow_files:
        for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            match = USES.match(line)
            if not match:
                continue
            action = match.group(1)
            if action.startswith("./"):
                continue
            _, separator, reference = action.rpartition("@")
            if not separator or not FULL_SHA.fullmatch(reference):
                failures.append(f"{path}:{line_number}: external Action is not pinned: {action}")

    if failures:
        raise SystemExit("\n".join(failures))
    print("all external Actions are pinned to full commit SHAs")


if __name__ == "__main__":
    main()
