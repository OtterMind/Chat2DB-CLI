//! Best-effort update notification for interactive CLI invocations.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::background;
use crate::error::Result;
use crate::http::{HttpClient, TimedReqwestHttpClient};
use crate::manifest::{BinaryManifest, binary_manifest_url};
use crate::platform::{
    current_arch_label, current_cli_os_label, try_acquire_exclusive_lock, write_private_atomic,
};

const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const NETWORK_TIMEOUT: Duration = Duration::from_secs(2);
const DISABLE_ENV: &str = "CHAT2DB_NO_UPDATE_CHECK";

#[derive(Debug, Clone, Copy)]
pub struct UpdateCheckPolicy {
    pub json_output: bool,
    pub interactive: bool,
    pub background_child: bool,
    pub excluded_command: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateNotice {
    pub current_version: String,
    pub latest_version: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct UpdateCheckState {
    checked_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_version: Option<String>,
}

pub fn enabled(policy: UpdateCheckPolicy) -> bool {
    !policy.json_output
        && policy.interactive
        && !policy.background_child
        && !policy.excluded_command
        && !env_flag_enabled(DISABLE_ENV)
}

pub fn check_for_update() -> Option<UpdateNotice> {
    let now = unix_timestamp(SystemTime::now());
    let state_path = background::state_dir().join("update-check.json");
    let client = TimedReqwestHttpClient::new(NETWORK_TIMEOUT);
    check_for_update_with_client(
        &client,
        &state_path,
        now,
        env!("CARGO_PKG_VERSION"),
        current_cli_os_label().ok()?,
        current_arch_label().ok()?,
    )
    .ok()
    .flatten()
}

fn check_for_update_with_client<C: HttpClient>(
    client: &C,
    state_path: &Path,
    now: u64,
    current_version: &str,
    os: &str,
    arch: &str,
) -> Result<Option<UpdateNotice>> {
    let state = read_state(state_path);
    if !check_due(state.as_ref(), now) {
        return Ok(notice_from_state(state.as_ref(), current_version));
    }
    let cached_notice = notice_from_state(state.as_ref(), current_version);

    let Ok(Some(_lock)) = try_acquire_exclusive_lock(&state_path.with_extension("lock")) else {
        return Ok(cached_notice);
    };

    let state = read_state(state_path);
    if !check_due(state.as_ref(), now) {
        return Ok(notice_from_state(state.as_ref(), current_version));
    }

    let mut next_state = UpdateCheckState {
        checked_at: now,
        latest_version: state.and_then(|state| state.latest_version),
    };
    write_state(state_path, &next_state)?;

    let manifest_url = binary_manifest_url(os, arch);
    let manifest = match client
        .get(&manifest_url)
        .and_then(|bytes| BinaryManifest::parse(&bytes, os, arch))
    {
        Ok(manifest) => manifest,
        Err(_) => return Ok(notice_from_state(Some(&next_state), current_version)),
    };
    next_state.latest_version = Some(manifest.version.clone());
    write_state(state_path, &next_state)?;

    Ok(notice_for_version(current_version, &manifest.version))
}

fn notice_from_state(
    state: Option<&UpdateCheckState>,
    current_version: &str,
) -> Option<UpdateNotice> {
    notice_for_version(current_version, state?.latest_version.as_deref()?)
}

fn notice_for_version(current_version: &str, latest_version: &str) -> Option<UpdateNotice> {
    let current = Version::parse(current_version).ok();
    let latest = Version::parse(latest_version).ok();
    if matches!((current, latest), (Some(current), Some(latest)) if latest > current) {
        return Some(UpdateNotice {
            current_version: current_version.to_string(),
            latest_version: latest_version.to_string(),
        });
    }
    None
}

fn check_due(state: Option<&UpdateCheckState>, now: u64) -> bool {
    match state {
        None => true,
        Some(state) if state.checked_at > now => true,
        Some(state) => now.saturating_sub(state.checked_at) >= CHECK_INTERVAL.as_secs(),
    }
}

fn read_state(path: &Path) -> Option<UpdateCheckState> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_state(path: &Path, state: &UpdateCheckState) -> Result<()> {
    let mut bytes = serde_json::to_vec(state)?;
    bytes.push(b'\n');
    write_private_atomic(path, &bytes)
}

fn env_flag_enabled(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
        .unwrap_or(false)
}

fn unix_timestamp(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::error::Chat2DbError;

    struct FakeClient {
        calls: Cell<usize>,
        response: Result<Vec<u8>>,
    }

    impl FakeClient {
        fn manifest(version: &str) -> Self {
            Self {
                calls: Cell::new(0),
                response: Ok(format!(
                    r#"{{"version":"{version}","artifact":{{"url":"https://cdn.chat2db-ai.com/download/chat2db-cli/{version}/linux/x86_64/chat2db-{version}-linux-x86_64.tar.gz","sha256":"{}","artifact_type":"tarball"}}}}"#,
                    "0".repeat(64)
                )
                .into_bytes()),
            }
        }

        fn failing() -> Self {
            Self {
                calls: Cell::new(0),
                response: Err(Chat2DbError::ManifestUnavailable {
                    url: "https://example.invalid/manifest.json".to_string(),
                }),
            }
        }
    }

    impl HttpClient for FakeClient {
        fn get(&self, _url: &str) -> Result<Vec<u8>> {
            self.calls.set(self.calls.get() + 1);
            match &self.response {
                Ok(bytes) => Ok(bytes.clone()),
                Err(_) => Err(Chat2DbError::ManifestUnavailable {
                    url: "https://example.invalid/manifest.json".to_string(),
                }),
            }
        }
    }

    #[test]
    fn policy_only_enables_interactive_human_commands() {
        let base = UpdateCheckPolicy {
            json_output: false,
            interactive: true,
            background_child: false,
            excluded_command: false,
        };
        assert!(enabled(base));
        assert!(!enabled(UpdateCheckPolicy {
            json_output: true,
            ..base
        }));
        assert!(!enabled(UpdateCheckPolicy {
            interactive: false,
            ..base
        }));
        assert!(!enabled(UpdateCheckPolicy {
            background_child: true,
            ..base
        }));
        assert!(!enabled(UpdateCheckPolicy {
            excluded_command: true,
            ..base
        }));
    }

    #[test]
    fn reports_cached_newer_version_without_repeating_network_check() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("update-check.json");
        let client = FakeClient::manifest("0.3.0");

        let notice =
            check_for_update_with_client(&client, &path, 100_000, "0.2.0", "linux", "x86_64")
                .unwrap();
        assert_eq!(
            notice,
            Some(UpdateNotice {
                current_version: "0.2.0".to_string(),
                latest_version: "0.3.0".to_string(),
            })
        );

        let second =
            check_for_update_with_client(&client, &path, 100_001, "0.2.0", "linux", "x86_64")
                .unwrap();
        assert_eq!(second, notice);
        assert_eq!(client.calls.get(), 1);
    }

    #[test]
    fn ignores_same_or_older_version() {
        for version in ["0.2.0", "0.1.9"] {
            let temp = tempfile::tempdir().unwrap();
            let client = FakeClient::manifest(version);
            let notice = check_for_update_with_client(
                &client,
                &temp.path().join("state.json"),
                100_000,
                "0.2.0",
                "linux",
                "x86_64",
            )
            .unwrap();
            assert_eq!(notice, None);
        }
    }

    #[test]
    fn failed_network_attempt_is_silent_and_throttled() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("update-check.json");
        let client = FakeClient::failing();

        assert_eq!(
            check_for_update_with_client(&client, &path, 100_000, "0.2.0", "linux", "x86_64")
                .unwrap(),
            None
        );
        assert_eq!(
            check_for_update_with_client(&client, &path, 100_001, "0.2.0", "linux", "x86_64")
                .unwrap(),
            None
        );
        assert_eq!(client.calls.get(), 1);
    }

    #[test]
    fn failed_refresh_falls_back_to_cached_newer_version() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("update-check.json");
        write_state(
            &path,
            &UpdateCheckState {
                checked_at: 1,
                latest_version: Some("0.3.0".to_string()),
            },
        )
        .unwrap();
        let client = FakeClient::failing();

        assert_eq!(
            check_for_update_with_client(&client, &path, 100_000, "0.2.0", "linux", "x86_64")
                .unwrap(),
            Some(UpdateNotice {
                current_version: "0.2.0".to_string(),
                latest_version: "0.3.0".to_string(),
            })
        );
        assert_eq!(client.calls.get(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn state_file_is_private() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.json");
        write_state(
            &path,
            &UpdateCheckState {
                checked_at: 1,
                latest_version: None,
            },
        )
        .unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
