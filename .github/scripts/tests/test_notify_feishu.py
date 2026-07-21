#!/usr/bin/env python3

import importlib.util
import json
import unittest
from pathlib import Path
from unittest import mock


MODULE_PATH = Path(__file__).resolve().parents[1] / "notify-feishu.py"
SPEC = importlib.util.spec_from_file_location("notify_feishu", MODULE_PATH)
assert SPEC and SPEC.loader
NOTIFY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(NOTIFY)
FEISHU_ORIGIN = "https://open.feishu.cn"
WEBHOOK_PATH = "/open-apis/bot/v2/hook/"
VALID_WEBHOOK = FEISHU_ORIGIN + WEBHOOK_PATH + "00000000-0000-0000-0000-000000000000"


class NotifyFeishuTest(unittest.TestCase):
    class Response:
        def __init__(self, value: object) -> None:
            self.body = json.dumps(value).encode()

        def __enter__(self):
            return self

        def __exit__(self, *_args) -> None:
            return None

        def read(self) -> bytes:
            return self.body

    def test_accepts_only_official_feishu_bot_webhook(self) -> None:
        NOTIFY.validate_webhook(VALID_WEBHOOK)
        for url in (
            "http://open.feishu.cn" + WEBHOOK_PATH + "00000000-0000-0000-0000-000000000000",
            "https://example.com" + WEBHOOK_PATH + "00000000-0000-0000-0000-000000000000",
            FEISHU_ORIGIN + WEBHOOK_PATH + "not-a-token",
            VALID_WEBHOOK + "?x=1",
        ):
            with self.subTest(url=url), self.assertRaises(ValueError):
                NOTIFY.validate_webhook(url)

    def test_start_message_contains_release_identity(self) -> None:
        message = NOTIFY.start_message(
            {
                "RELEASE_MODE": "cdn-only",
                "RELEASE_VERSION": "0.2.0",
                "RELEASE_ACTOR": "maintainer",
                "RELEASE_SHA": "abc123",
                "RELEASE_RUN_URL": "https://github.com/example/actions/runs/1",
            }
        )
        self.assertIn("cdn-only", message)
        self.assertIn("0.2.0", message)
        self.assertIn("abc123", message)

    def test_cdn_only_requires_its_terminal_job(self) -> None:
        self.assertEqual(
            "success",
            NOTIFY.summary_state(
                "cdn-only", False, {"publish-cdn-only": "success", "publish-full-release": "skipped"}
            ),
        )
        self.assertEqual(
            "failure",
            NOTIFY.summary_state("cdn-only", False, {"assemble-release": "success"}),
        )

    def test_release_requires_its_terminal_job(self) -> None:
        self.assertEqual(
            "success",
            NOTIFY.summary_state(
                "release", False, {"publish-cdn-only": "skipped", "publish-full-release": "success"}
            ),
        )
        self.assertEqual(
            "failure",
            NOTIFY.summary_state("release", False, {"publish-full-release": "failure"}),
        )

    def test_cancelled_result_wins(self) -> None:
        self.assertEqual(
            "cancelled",
            NOTIFY.summary_state(
                "release", False, {"publish-full-release": "success", "quality": "cancelled"}
            ),
        )
        self.assertEqual("cancelled", NOTIFY.summary_state("cdn-only", True, {}))

    def test_cdn_only_summary_states_non_public_boundaries(self) -> None:
        message = NOTIFY.summary_message(
            {
                "RELEASE_MODE": "cdn-only",
                "RELEASE_VERSION": "0.2.0",
                "RESULT_PUBLISH_CDN_ONLY": "success",
            }
        )
        self.assertIn("GitHub Release: not created", message)
        self.assertIn("latest aliases: unchanged", message)

    def test_send_requires_an_explicit_success_code(self) -> None:
        responses = [self.Response({}), self.Response([]), self.Response(None)]
        with mock.patch.object(NOTIFY.urllib.request, "urlopen", side_effect=responses) as open_url:
            with mock.patch.object(NOTIFY.time, "sleep") as sleep:
                with self.assertRaises(RuntimeError):
                    NOTIFY.send(
                        VALID_WEBHOOK,
                        "test",
                    )
        self.assertEqual(3, open_url.call_count)
        self.assertEqual(2, sleep.call_count)

    def test_send_accepts_explicit_zero_code_without_retry(self) -> None:
        with mock.patch.object(
            NOTIFY.urllib.request, "urlopen", return_value=self.Response({"code": 0})
        ) as open_url:
            with mock.patch.object(NOTIFY.time, "sleep") as sleep:
                NOTIFY.send(
                    VALID_WEBHOOK,
                    "test",
                )
        open_url.assert_called_once()
        sleep.assert_not_called()


if __name__ == "__main__":
    unittest.main()
