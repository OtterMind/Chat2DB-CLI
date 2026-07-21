#!/usr/bin/env python3

import re
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
NOTICE = ROOT / "THIRD_PARTY_LICENSES.txt"
TARGETS = (
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
)


def fail(message: str) -> None:
    raise SystemExit(f"error: {message}")


PACKAGE_LINE = re.compile(r"^([^ ]+) v([^ ]+)")


def runtime_packages(target: str) -> set[tuple[str, str]]:
    result = subprocess.run(
        [
            "cargo",
            "tree",
            "--locked",
            "--all-features",
            "--edges",
            "normal,build",
            "--prefix",
            "none",
            "--format",
            "{p}",
            "--target",
            target,
        ],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    packages = {
        (match.group(1), match.group(2))
        for line in result.stdout.splitlines()
        if (match := PACKAGE_LINE.match(line))
    }
    packages = {package for package in packages if package[0] != "chat2db"}
    if not packages:
        fail(f"cargo tree returned no runtime dependencies for {target}")
    return packages


def main() -> None:
    if not NOTICE.is_file():
        fail(f"missing {NOTICE.name}")
    text = NOTICE.read_text(encoding="utf-8")
    if re.search(r"^- chat2db ", text, re.MULTILINE):
        fail("third-party notices must not list the Chat2DB CLI package itself")
    expected = set().union(*(runtime_packages(target) for target in TARGETS))
    missing = sorted(
        f"{name} {version}"
        for name, version in expected
        if f"- {name} {version}\n" not in text
    )
    if missing:
        fail("third-party notices are stale; missing: " + ", ".join(missing))
    print(f"third-party license notices cover {len(expected)} locked packages")


if __name__ == "__main__":
    main()
