#!/usr/bin/env python3

import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
READMES = [
    ROOT / "README.md",
    ROOT / "README_FR.md",
    ROOT / "README_JA.md",
    ROOT / "README_ES.md",
    ROOT / "README_KO.md",
]
LINK = re.compile(r"\[[^\]]*\]\(([^)]+)\)")
HAN = re.compile(r"[\u3400-\u4dbf\u4e00-\u9fff]")
JAPANESE_README = ROOT / "README_JA.md"
KOREAN_JAPANESE_NAVIGATION = "[\u65e5\u672c\u8a9e](README_JA.md)"


def fail(message: str) -> None:
    raise SystemExit(f"error: {message}")


def command_blocks(path: Path) -> str:
    lines = path.read_text(encoding="utf-8").splitlines()
    commands: list[str] = []
    active = False
    for line in lines:
        if line in ("```bash", "```powershell"):
            active = True
            continue
        if line == "```":
            active = False
            continue
        if active:
            commands.append(line)
    return "\n".join(commands)


def markdown_files() -> list[Path]:
    roots = [
        *READMES,
        ROOT / "CONTRIBUTING.md",
        ROOT / "SECURITY.md",
        ROOT / "CODE_OF_CONDUCT.md",
        *sorted((ROOT / "docs").rglob("*.md")),
        *sorted((ROOT / "skill").rglob("*.md")),
    ]
    return list(dict.fromkeys(path for path in roots if path.is_file()))


def validate_repository_language() -> None:
    if (ROOT / "README_CN.md").exists():
        fail("Chinese README is not part of the public repository language set")
    ignored_directories = {".git", "target", "dist", "release-assets", "__pycache__"}
    for path in ROOT.rglob("*"):
        if not path.is_file() or any(part in ignored_directories for part in path.parts):
            continue
        if path == JAPANESE_README:
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        if path == ROOT / "README_KO.md":
            text = text.replace(KOREAN_JAPANESE_NAVIGATION, "")
        if HAN.search(text):
            fail(f"Chinese text is not allowed in {path.relative_to(ROOT)}")


def validate_relative_links(path: Path, text: str) -> None:
    for raw_target in LINK.findall(text):
        target = raw_target.strip().strip("<>")
        if target.startswith(("https://", "http://", "mailto:", "#")):
            continue
        relative = target.split("#", 1)[0]
        if not relative:
            continue
        resolved = (path.parent / relative).resolve()
        try:
            resolved.relative_to(ROOT)
        except ValueError:
            fail(f"link escapes repository in {path.relative_to(ROOT)}: {raw_target}")
        if not resolved.exists():
            fail(f"broken link in {path.relative_to(ROOT)}: {raw_target}")


def main() -> None:
    missing = [path.name for path in READMES if not path.is_file()]
    if missing:
        fail(f"missing localized README files: {', '.join(missing)}")

    validate_repository_language()

    expected_commands = command_blocks(READMES[0])
    for path in READMES[1:]:
        if command_blocks(path) != expected_commands:
            fail(f"command blocks differ from README.md: {path.name}")

    compatibility_markers = (
        "macOS 11",
        "Windows 10",
        "glibc 2.35",
        "musl",
        "Alpine",
    )
    for path in [*READMES, ROOT / "docs" / "RELEASING.md"]:
        text = path.read_text(encoding="utf-8")
        missing_markers = [marker for marker in compatibility_markers if marker not in text]
        if missing_markers:
            fail(
                f"missing compatibility markers in {path.relative_to(ROOT)}: "
                + ", ".join(missing_markers)
            )

    for path in READMES:
        text = path.read_text(encoding="utf-8")
        if "CHAT2DB_NO_UPDATE_CHECK=1" not in text or "24" not in text:
            fail(f"missing interactive update-check policy in {path.name}")

    for path in markdown_files():
        text = path.read_text(encoding="utf-8")
        if "chat2db status --edition" in text:
            fail(f"status does not accept --edition: {path.relative_to(ROOT)}")
        if "Chat2DB-CLI-Skill" in text:
            fail(f"legacy standalone Skill repository name remains: {path.relative_to(ROOT)}")
        if "https://cdn.chat2db-ai.com/install/" in text:
            fail(f"executable installer must come from GitHub Releases: {path.relative_to(ROOT)}")
        if re.search(r"\bchat2db completions (?:zsh|bash|fish)\b", text):
            fail(f"completions shell must use --shell: {path.relative_to(ROOT)}")
        validate_relative_links(path, text)

    print("documentation checks passed")


if __name__ == "__main__":
    main()
