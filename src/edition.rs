use std::fmt;
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::error::{Chat2DbError, Result};
use crate::platform::{HostOs, current_os};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Edition {
    Community,
    Pro,
    Local,
}

impl Edition {
    pub const ALL: [Edition; 3] = [Edition::Community, Edition::Pro, Edition::Local];

    pub fn as_str(self) -> &'static str {
        match self {
            Edition::Community => "community",
            Edition::Pro => "pro",
            Edition::Local => "local",
        }
    }

    pub fn app_name(self) -> &'static str {
        match self {
            Edition::Community => "Chat2DB Community",
            Edition::Pro => "Chat2DB Pro",
            Edition::Local => "Chat2DB Local",
        }
    }

    pub fn bundle_id(self) -> &'static str {
        match self {
            Edition::Community => "com.chat2db.community",
            Edition::Pro => "com.chat2db.pro",
            Edition::Local => "com.chat2db.local",
        }
    }

    pub fn windows_exe_name(self) -> &'static str {
        match self {
            Edition::Community => "Chat2DB Community.exe",
            Edition::Pro => "Chat2DB Pro.exe",
            Edition::Local => "Chat2DB Local.exe",
        }
    }

    pub fn windows_upgrade_code(self) -> &'static str {
        match self {
            Edition::Community => "4D7C78BC-B42F-4F81-9F5F-56E3F5E4E9B2",
            Edition::Pro => "9F7E2D1C-4B65-4A8D-93F2-6C0B8E1D5A72",
            Edition::Local => "6B6F2D7E-0D36-4D9E-91C5-9C9F7D4A2B18",
        }
    }

    pub fn requires_login(self) -> bool {
        !matches!(self, Edition::Community)
    }

    pub fn mcp_url(self) -> &'static str {
        match self {
            Edition::Community => "http://127.0.0.1:10825/mcp",
            Edition::Pro | Edition::Local => "http://127.0.0.1:11924/mcp",
        }
    }

    pub fn data_dir_name(self) -> &'static str {
        match self {
            Edition::Community => ".chat2db-community",
            Edition::Pro => ".chat2db",
            Edition::Local => ".chat2db_local_edition",
        }
    }

    pub fn data_dir(self, home_dir: &Path) -> PathBuf {
        home_dir.join(self.data_dir_name())
    }

    pub fn cache_dir_name(self) -> &'static str {
        match self {
            Edition::Community => "chat2db_cache_community",
            Edition::Pro => "chat2db_cache_pro",
            Edition::Local => "chat2db_cache_local",
        }
    }

    pub fn settings_path(self, home_dir: &Path) -> PathBuf {
        self.data_dir(home_dir)
            .join(self.cache_dir_name())
            .join("settings.json")
    }

    pub fn frontend_leveldb_path(self, home_dir: &Path) -> PathBuf {
        self.data_dir(home_dir)
            .join(self.cache_dir_name())
            .join("Local Storage")
            .join("leveldb")
    }

    pub fn runtime_log_path(self, home_dir: &Path) -> PathBuf {
        let runtime_dir = match self {
            Edition::Community => "chat2db-community",
            Edition::Pro | Edition::Local => "chat2db-enterprise",
        };
        self.data_dir(home_dir)
            .join(runtime_dir)
            .join("logs")
            .join("application.log")
    }

    pub fn app_path(self, applications_dir: &Path) -> PathBuf {
        self.resolve_app_path_for_os(applications_dir, current_os())
    }

    pub fn app_path_for_os(self, applications_dir: &Path, os: HostOs) -> PathBuf {
        self.app_path_candidates_for_os(applications_dir, os)
            .remove(0)
    }

    pub fn app_path_candidates_for_os(self, applications_dir: &Path, os: HostOs) -> Vec<PathBuf> {
        match os {
            HostOs::Windows => windows_app_path_candidates(applications_dir, self),
            _ => vec![applications_dir.join(format!("{}.app", self.app_name()))],
        }
    }

    pub fn resolve_app_path_for_os(self, applications_dir: &Path, os: HostOs) -> PathBuf {
        let candidates = self.app_path_candidates_for_os(applications_dir, os);
        candidates
            .iter()
            .find(|path| path.exists())
            .cloned()
            .unwrap_or_else(|| candidates[0].clone())
    }

    pub fn desktop_artifact_name(self, version: &str, os: HostOs, arch: &str) -> Result<String> {
        let product = match self {
            Edition::Community => "Community",
            Edition::Pro => "Pro",
            Edition::Local => "Local",
        };
        match os {
            HostOs::Macos => {
                let architecture = match arch {
                    "arm64" => "arm64",
                    "x86_64" => "x64",
                    other => return Err(Chat2DbError::UnsupportedArch(other.to_string())),
                };
                Ok(format!("Chat2DB-{product}-{version}-{architecture}.dmg"))
            }
            HostOs::Windows => match arch {
                "arm64" | "x86_64" => {
                    let extension = if self == Edition::Community {
                        "msi"
                    } else {
                        "exe"
                    };
                    Ok(format!("Chat2DB-{product}-{version}.{extension}"))
                }
                other => Err(Chat2DbError::UnsupportedArch(other.to_string())),
            },
            HostOs::Linux | HostOs::Other => Err(Chat2DbError::UnsupportedPlatform),
        }
    }

    pub fn desktop_artifact_kind(self, os: HostOs) -> Result<DesktopArtifactKind> {
        match os {
            HostOs::Macos => Ok(DesktopArtifactKind::Dmg),
            HostOs::Windows if self == Edition::Community => Ok(DesktopArtifactKind::Msi),
            HostOs::Windows => Ok(DesktopArtifactKind::Exe),
            HostOs::Linux | HostOs::Other => Err(Chat2DbError::UnsupportedPlatform),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopArtifactKind {
    Dmg,
    Exe,
    Msi,
}

impl DesktopArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dmg => "dmg",
            Self::Exe => "exe",
            Self::Msi => "msi",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Dmg => "dmg",
            Self::Exe => "exe",
            Self::Msi => "msi",
        }
    }

    pub fn progress_label(self) -> &'static str {
        match self {
            Self::Dmg => "DMG",
            Self::Exe => "EXE",
            Self::Msi => "MSI",
        }
    }
}

fn windows_app_path_candidates(applications_dir: &Path, edition: Edition) -> Vec<PathBuf> {
    let spaced_dir = edition.app_name();
    let dashed_dir = match edition {
        Edition::Community => "Chat2DB-Community",
        Edition::Pro => "Chat2DB-Pro",
        Edition::Local => "Chat2DB-Local",
    };
    let dashed_exe = format!("{dashed_dir}.exe");
    let spaced_exe = edition.windows_exe_name();

    let mut dirs = if applications_dir
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("Programs"))
    {
        vec![applications_dir.to_path_buf()]
    } else {
        vec![
            applications_dir.to_path_buf(),
            applications_dir.join("Programs"),
        ]
    };
    for key in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(path) = std::env::var_os(key).map(PathBuf::from)
            && !dirs.contains(&path)
        {
            dirs.push(path);
        }
    }

    let mut candidates = Vec::new();
    for dir in dirs {
        candidates.push(dir.join(spaced_dir).join(spaced_exe));
        candidates.push(dir.join(dashed_dir).join(&dashed_exe));
        candidates.push(dir.join(spaced_dir).join(&dashed_exe));
        candidates.push(dir.join(dashed_dir).join(spaced_exe));
    }
    candidates
}

impl fmt::Display for Edition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// CLI-facing edition selector. Use `editions()` to expand `All` into the
/// concrete `Edition` values to dispatch over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum EditionSelector {
    Community,
    Pro,
    Local,
    All,
}

impl EditionSelector {
    pub fn editions(self) -> &'static [Edition] {
        match self {
            EditionSelector::Community => &[Edition::Community],
            EditionSelector::Pro => &[Edition::Pro],
            EditionSelector::Local => &[Edition::Local],
            EditionSelector::All => &Edition::ALL,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pro_uses_versioned_desktop_artifact_names() {
        assert_eq!(
            Edition::Pro
                .desktop_artifact_name("3.1.15", HostOs::Macos, "x86_64")
                .unwrap(),
            "Chat2DB-Pro-3.1.15-x64.dmg"
        );
    }

    #[test]
    fn local_uses_versioned_desktop_artifact_names() {
        assert_eq!(
            Edition::Local
                .desktop_artifact_name("3.1.16", HostOs::Macos, "arm64")
                .unwrap(),
            "Chat2DB-Local-3.1.16-arm64.dmg"
        );
    }

    #[test]
    fn windows_uses_edition_specific_artifact_types() {
        assert_eq!(
            Edition::Community
                .desktop_artifact_name("5.3.0", HostOs::Windows, "x86_64")
                .unwrap(),
            "Chat2DB-Community-5.3.0.msi"
        );
        assert_eq!(
            Edition::Local
                .desktop_artifact_name("3.1.16", HostOs::Windows, "arm64")
                .unwrap(),
            "Chat2DB-Local-3.1.16.exe"
        );
    }

    #[test]
    fn windows_app_paths_cover_jpackage_and_electron_names() {
        let base = Path::new(r"C:\Users\tester\AppData\Local");
        let candidates = Edition::Pro.app_path_candidates_for_os(base, HostOs::Windows);
        assert_eq!(
            candidates[0],
            base.join("Chat2DB Pro").join("Chat2DB Pro.exe")
        );
        assert!(
            candidates.contains(
                &base
                    .join("Programs")
                    .join("Chat2DB-Pro")
                    .join("Chat2DB-Pro.exe")
            )
        );
        assert!(
            candidates.contains(
                &base
                    .join("Programs")
                    .join("Chat2DB Pro")
                    .join("Chat2DB Pro.exe")
            )
        );
    }

    #[test]
    fn community_metadata_matches_desktop_release_contract() {
        let home = Path::new("/Users/tester");
        assert_eq!(
            Edition::ALL,
            [Edition::Community, Edition::Pro, Edition::Local]
        );
        assert_eq!(Edition::Community.app_name(), "Chat2DB Community");
        assert_eq!(Edition::Community.bundle_id(), "com.chat2db.community");
        assert_eq!(
            Edition::Community.windows_upgrade_code(),
            "4D7C78BC-B42F-4F81-9F5F-56E3F5E4E9B2"
        );
        assert_eq!(
            Edition::Community.data_dir(home),
            home.join(".chat2db-community")
        );
        assert_eq!(
            Edition::Community.settings_path(home),
            home.join(".chat2db-community/chat2db_cache_community/settings.json")
        );
        assert_eq!(
            Edition::Community.runtime_log_path(home),
            home.join(".chat2db-community/chat2db-community/logs/application.log")
        );
        assert_eq!(Edition::Community.mcp_url(), "http://127.0.0.1:10825/mcp");
        assert_eq!(
            Edition::Community
                .desktop_artifact_name("5.3.0", HostOs::Macos, "arm64")
                .unwrap(),
            "Chat2DB-Community-5.3.0-arm64.dmg"
        );
        assert_eq!(
            Edition::Community
                .desktop_artifact_name("5.3.0", HostOs::Windows, "x86_64")
                .unwrap(),
            "Chat2DB-Community-5.3.0.msi"
        );
    }
}
