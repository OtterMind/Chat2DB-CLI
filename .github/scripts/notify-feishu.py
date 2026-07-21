#!/usr/bin/env python3

from __future__ import annotations

import json
import os
import re
import time
import urllib.parse
import urllib.request


WEBHOOK_PATH = re.compile(
    r"^/open-apis/bot/v2/hook/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
)
RESULT_KEYS = (
    ("resolve", "RESULT_RESOLVE"),
    ("quality", "RESULT_QUALITY"),
    ("validate", "RESULT_VALIDATE"),
    ("package-cli", "RESULT_PACKAGE_CLI"),
    ("package-skill", "RESULT_PACKAGE_SKILL"),
    ("sbom", "RESULT_SBOM"),
    ("assemble-release", "RESULT_ASSEMBLE_RELEASE"),
    ("draft-release", "RESULT_DRAFT_RELEASE"),
    ("publish-cdn-only", "RESULT_PUBLISH_CDN_ONLY"),
    ("publish-full-release", "RESULT_PUBLISH_FULL_RELEASE"),
)


def validate_webhook(url: str) -> None:
    parsed = urllib.parse.urlsplit(url)
    if (
        parsed.scheme != "https"
        or parsed.netloc != "open.feishu.cn"
        or parsed.query
        or parsed.fragment
        or parsed.username is not None
        or parsed.password is not None
        or not WEBHOOK_PATH.fullmatch(parsed.path)
    ):
        raise ValueError("FEISHU_RELEASE_NOTIFY_WEBHOOK is not a valid Feishu bot URL")


def result_map(environment: dict[str, str]) -> dict[str, str]:
    return {
        name: environment.get(variable, "")
        for name, variable in RESULT_KEYS
        if environment.get(variable, "")
    }


def summary_state(mode: str, cancelled: bool, results: dict[str, str]) -> str:
    if cancelled or "cancelled" in results.values():
        return "cancelled"
    terminal = "publish-cdn-only" if mode == "cdn-only" else "publish-full-release"
    if mode in ("cdn-only", "release") and results.get(terminal) == "success":
        return "success"
    return "failure"


def common_lines(environment: dict[str, str]) -> list[str]:
    return [
        f"Mode: {environment.get('RELEASE_MODE') or 'unknown'}",
        f"Version: {environment.get('RELEASE_VERSION') or 'unknown'}",
        f"Actor: {environment.get('RELEASE_ACTOR') or 'unknown'}",
        f"Ref: {environment.get('RELEASE_REF') or 'unknown'}",
        f"Commit: {environment.get('RELEASE_SHA') or 'unknown'}",
        f"Run: {environment.get('RELEASE_RUN_URL') or 'unknown'}",
    ]


def start_message(environment: dict[str, str]) -> str:
    return "\n".join(["Chat2DB CLI publication started", *common_lines(environment)])


def summary_message(environment: dict[str, str]) -> str:
    mode = environment.get("RELEASE_MODE", "")
    results = result_map(environment)
    state = summary_state(mode, False, results)
    title = {
        "success": "Chat2DB CLI CDN upload completed"
        if mode == "cdn-only"
        else "Chat2DB CLI release completed",
        "failure": "Chat2DB CLI publication failed",
        "cancelled": "Chat2DB CLI publication cancelled",
    }[state]
    lines = [title, *common_lines(environment), "Stage results:"]
    lines.extend(
        f"- {name}: {result}"
        for name, result in results.items()
        if result != "skipped"
    )
    version = environment.get("RELEASE_VERSION", "")
    repository = environment.get("RELEASE_REPOSITORY", "")
    if state == "success" and mode == "cdn-only":
        lines.extend(
            (
                "GitHub Release: not created",
                "latest aliases: unchanged",
                f"CLI CDN: https://cdn.chat2db-ai.com/download/chat2db-cli/{version}/",
                f"Skill CDN: https://cdn.chat2db-ai.com/download/chat2db-skill/{version}/",
            )
        )
    elif state == "success" and mode == "release" and repository:
        lines.append(f"Release: https://github.com/{repository}/releases/tag/v{version}")
    return "\n".join(lines)


def send(webhook: str, message: str) -> None:
    payload = json.dumps(
        {"msg_type": "text", "content": {"text": message}}, ensure_ascii=False
    ).encode("utf-8")
    last_error: Exception | None = None
    for attempt in range(1, 4):
        request = urllib.request.Request(
            webhook,
            data=payload,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        try:
            with urllib.request.urlopen(request, timeout=15) as response:
                result = json.loads(response.read().decode("utf-8"))
            if isinstance(result, dict):
                status = result.get("code")
                if status is None:
                    status = result.get("StatusCode")
                if isinstance(status, int) and not isinstance(status, bool) and status == 0:
                    return
            last_error = RuntimeError("Feishu rejected the notification")
        except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
            last_error = error
        if attempt < 3:
            time.sleep(attempt)
    raise RuntimeError("Feishu notification failed after three attempts") from last_error


def main() -> None:
    webhook = os.environ.get("FEISHU_WEBHOOK_URL", "")
    if not webhook:
        print("::warning::FEISHU_RELEASE_NOTIFY_WEBHOOK is not configured")
        return
    validate_webhook(webhook)
    event = os.environ.get("NOTIFICATION_EVENT", "")
    if event == "start":
        message = start_message(os.environ)
    elif event == "summary":
        message = summary_message(os.environ)
    else:
        raise ValueError("NOTIFICATION_EVENT must be start or summary")
    send(webhook, message)
    print(f"Feishu {event} notification sent")


if __name__ == "__main__":
    main()
