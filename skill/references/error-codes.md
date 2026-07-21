# Error Codes

On failure, parse `error.code` first. Use `error.next_action` when present and acceptable. Do not retry unchanged commands when `error.retryable` is `false`.

## Hard Stops

Do not bypass or auto-retry:

| Code | Action |
| --- | --- |
| `checksum_mismatch` | Surface expected/actual hashes. |
| `unofficial_artifact_url` | Surface the URL. |
| `bundle_id_mismatch` | Surface expected/actual bundle ids. |
| `unsafe_replace` | Do not overwrite the path. |
| `unsafe_uninstall` | Do not delete the path. |
| `unsafe_open` | Do not open the app. |
| `app_process_still_running` | Stop the reported edition before changing its installation. |
| `unsafe_zip_path` | Surface the zip path. |
| `invalid_cli_target` | Surface the target path. |
| `skill_target_managed_externally` | Manage with git or remove manually. |

## Common Recovery

Community login/logout return a successful `not_applicable` status; they are
not authentication errors and must not trigger a Pro or Local retry.

| Code | Recovery |
| --- | --- |
| `unsupported_platform` | Desktop app operations require macOS or Windows. Use only non-app commands on this host. |
| `unsupported_arch` | Stop and surface the architecture. |
| `http_request_error` | Retry once; then ask the user to check network/proxy/VPN. |
| `http_status_error` | Retry once for 5xx. Do not retry 4xx. |
| `manifest_unavailable` | Stop; the manifest is not available. |
| `invalid_release_manifest` | Stop; a release manifest or the compiled desktop artifact registry failed version, target, path, size, or checksum validation. |
| `command_io_error` | Surface the missing or failing program. |
| `command_failed` | Surface stderr. |
| `manifest_json_error` | Stop; manifest is invalid. |
| `missing_manifest_field` | Stop; manifest is missing a required field. |
| `unsupported_artifact` | Stop; artifact type is invalid. |
| `invalid_sha256` | Stop; manifest checksum is malformed. |
| `invalid_dmg` | Stop; app artifact is invalid. |
| `invalid_app_zip` | Stop; app zip is invalid. |
| `unexpected_zip_entry` | Stop; artifact layout is invalid. |
| `missing_info_plist` | Stop; app bundle is invalid. |
| `app_already_exists` | Ask before replacing; use `--force --yes` only after confirmation. |
| `app_not_installed` | Install the requested edition first. |
| `app_process_still_running` | Run the reported `chat2db stop --edition ...` next action, verify it stopped, then retry. |
| `confirmation_required` | Use `--dry-run --json` when available, or ask before adding `--yes`. Datasource deletion uses `chat2db db datasource-delete --data-source-id ID --yes --json`. |
| `login_timed_out` | Pro/Local only. Do not auto-retry; the user must complete GUI login. |
| `invalid_cookie_file` | Pro/Local only. Remove the cookie file or ask the user to log in again. |
| `invalid_cookie_shape` | Pro/Local only. Remove the cookie file or ask the user to log in again. |
| `background_install_already_running` | Tail the existing install log; do not start another install. |
| `log_not_found` | Start a background install first; `logs install` only reads background install logs. |
| `completions_shell_unknown` | Re-run with `--shell zsh`, `--shell bash`, or `--shell fish`. |
| `completions_shell_unsupported` | Tell the user only zsh, bash, and fish are supported. |
| `io_error` | Surface the filesystem error and path. |
| `zip_error` | Retry once only if the artifact can be re-downloaded. |
| `plist_error` | Reinstall the app. |
| `url_error` | Stop; URL is invalid. |

## Skill

| Code | Recovery |
| --- | --- |
| `skill_already_installed` | Prefer `chat2db skill update`; use `install --force --yes` only after confirmation. |
| `skill_not_installed` | Run `chat2db skill install`. |
| `skill_version_incompatible` | Update the CLI or use a compatible skill bundle. |
| `skill_target_foreign` | Ask before replacing; `uninstall` always refuses foreign targets. |
| `skill_target_managed_externally` | Manage with git or remove manually. |
| `skill_bundle_corrupt` | Stop; bundle artifact is invalid. |

## Runtime, DB, SQL

| Code | Recovery |
| --- | --- |
| `runtime_cfg_invalid` | Reinstall the selected app edition. |
| `runtime_endpoint_not_ready` | Check `runtime status`; stop stale runtime and retry once after state is clear. |
| `runtime_state_invalid_json` | Run `runtime stop` or remove the reported state file. |
| `runtime_client_http_error` | Check `runtime status`; stop stale runtime and retry once. |
| `runtime_api_error` | Fix request arguments or datasource state. For datasource CRUD, check required flags and redact passwords in the report. |
| `invalid_connection_test_args` | Use either `--data-source-id` or temporary connection fields; do not mix both modes. |
| `invalid_datasource_create_args` | Use either `--url` or `--host --port --database`; do not mix both. Keep `--password` present, even when intentionally empty. |
| `datasource_connection_failed` | Fix connection parameters, credentials, driver, or network; retry only with changed inputs. |
| `datasource_not_found` | Refresh datasource list and use an existing datasource id. |
| `sql_result_set_not_found` | Re-run without `--result-set-id` or choose one of the returned available result set ids. |
| `runtime_token_generation_error` | Stop; host token generation failed. |

## MCP

| Code | Recovery |
| --- | --- |
| `mcp_settings_invalid_json` | Surface the settings path; user must fix or move it. |
| `mcp_settings_not_object` | Surface the settings path; user must fix it. |
| `mcp_port_ambiguous` | Stop the other edition, then retry the MCP command. |
| `mcp_endpoint_not_ready` | Run `mcp status`; retry once only after endpoint state is clear. |
| `mcp_endpoint_still_alive` | Check whether the other edition is running; inspect logs before retry. |
| `mcp_app_not_stopped` | Check app process state; ask before force-killing anything. |
| `mcp_app_not_started` | Verify App state with `chat2db status --json`; restart only after checking state. |
| `mcp_token_generation_error` | Stop; host token generation failed. |
| `mcp_frontend_store_error` | Ensure the app is closed, then retry once. |
| `mcp_frontend_store_invalid_json` | Surface the path; user must repair/reset it. |
| `mcp_token_missing` | Run `chat2db mcp enable --edition <edition> --json`. |
| `mcp_client_http_error` | Run `mcp status`; enable/restart if endpoint is not alive. |
| `mcp_client_protocol_error` | Check tool name and schema with `mcp tools`; do not retry blindly. |
| `mcp_args_invalid_json` | Pass a valid JSON object to `--args-json`. |
