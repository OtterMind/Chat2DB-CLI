use std::collections::HashMap;
use std::fs;
use std::path::Path;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use chat2db::edition::{DesktopArtifactKind, Edition};
use chat2db::error::{Chat2DbError, Result};
use chat2db::http::HttpClient;
use chat2db::installer::{
    DesktopArtifactInstaller, InstallRequest, Installer, SystemPaths, UninstallRequest,
    verify_app_bundle,
};
use chat2db::manifest::sha256_hex;
use chat2db::platform::HostOs;
use sha2::{Digest, Sha512};

#[derive(Debug, Clone)]
struct FakeHttp {
    responses: HashMap<String, Vec<u8>>,
}

impl HttpClient for FakeHttp {
    fn get(&self, url: &str) -> Result<Vec<u8>> {
        self.responses.get(url).cloned().ok_or_else(|| {
            Chat2DbError::UnsafeZipPath(format!("test URL was not registered: {url}"))
        })
    }
}

#[derive(Debug, Clone)]
struct FakeArtifactInstaller {
    app_name: &'static str,
    bundle_id: &'static str,
}

impl DesktopArtifactInstaller for FakeArtifactInstaller {
    fn install_artifact(
        &self,
        artifact_path: &Path,
        stage_dir: &Path,
        target_path: &Path,
        edition: Edition,
        kind: DesktopArtifactKind,
    ) -> Result<()> {
        assert!(artifact_path.is_file());
        match kind {
            DesktopArtifactKind::Dmg => {
                let app_path = stage_dir.join(self.app_name);
                create_fake_app(&app_path, self.bundle_id);
                verify_app_bundle(&app_path, edition)?;
                fs::rename(app_path, target_path).unwrap();
            }
            DesktopArtifactKind::Exe | DesktopArtifactKind::Msi => {
                if let Some(parent) = target_path.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::write(target_path, b"fake-exe").unwrap();
            }
        }
        Ok(())
    }
}

#[test]
fn installs_fake_pro_dmg_into_injected_applications_dir() {
    let temp = tempfile::tempdir().unwrap();
    let paths = test_paths(temp.path());
    let (client, artifact_url, registry) = desktop_artifact_client(
        Edition::Pro,
        HostOs::Macos,
        current_test_arch(),
        "3.1.15",
        b"fake-dmg",
    );

    let installer = Installer::with_artifact_installer(
        client,
        paths.clone(),
        FakeArtifactInstaller {
            app_name: "Bundled Name.app",
            bundle_id: "com.chat2db.pro",
        },
    )
    .with_desktop_artifact_registry(registry);
    let outcome = installer
        .install_for_os(
            InstallRequest {
                edition: Edition::Pro,
                force: false,
                progress: false,
            },
            HostOs::Macos,
        )
        .unwrap();

    assert_eq!(outcome.status, "installed");
    assert_eq!(outcome.artifact_url, artifact_url);
    assert_eq!(outcome.artifact_type, "dmg");
    assert_eq!(
        outcome.app_path,
        paths.applications_dir.join("Chat2DB Pro.app")
    );
    verify_app_bundle(&outcome.app_path, Edition::Pro).unwrap();
}

#[test]
fn installs_fake_local_dmg_into_injected_applications_dir() {
    let temp = tempfile::tempdir().unwrap();
    let paths = test_paths(temp.path());
    let (client, artifact_url, registry) = desktop_artifact_client(
        Edition::Local,
        HostOs::Macos,
        current_test_arch(),
        "3.1.16",
        b"fake-local-dmg",
    );

    let installer = Installer::with_artifact_installer(
        client,
        paths.clone(),
        FakeArtifactInstaller {
            app_name: "Bundled Local Name.app",
            bundle_id: "com.chat2db.local",
        },
    )
    .with_desktop_artifact_registry(registry);
    let outcome = installer
        .install_for_os(
            InstallRequest {
                edition: Edition::Local,
                force: false,
                progress: false,
            },
            HostOs::Macos,
        )
        .unwrap();

    assert_eq!(outcome.status, "installed");
    assert_eq!(outcome.artifact_url, artifact_url);
    assert_eq!(outcome.artifact_type, "dmg");
    assert_eq!(
        outcome.app_path,
        paths.applications_dir.join("Chat2DB Local.app")
    );
    verify_app_bundle(&outcome.app_path, Edition::Local).unwrap();
}

#[cfg(not(windows))]
#[test]
fn installs_fake_windows_pro_exe_into_local_app_data() {
    let temp = tempfile::tempdir().unwrap();
    let paths = test_paths(temp.path());
    let (client, artifact_url, registry) = desktop_artifact_client(
        Edition::Pro,
        HostOs::Windows,
        current_test_arch(),
        "3.1.15",
        b"fake-exe",
    );

    let installer = Installer::with_artifact_installer(
        client,
        paths.clone(),
        FakeArtifactInstaller {
            app_name: "unused.app",
            bundle_id: "unused",
        },
    )
    .with_desktop_artifact_registry(registry);
    let outcome = installer
        .install_for_os(
            InstallRequest {
                edition: Edition::Pro,
                force: false,
                progress: false,
            },
            HostOs::Windows,
        )
        .unwrap();

    assert_eq!(outcome.status, "installed");
    assert_eq!(outcome.artifact_url, artifact_url);
    assert_eq!(outcome.artifact_type, "exe");
    assert_eq!(
        outcome.app_path,
        paths
            .applications_dir
            .join("Chat2DB Pro")
            .join("Chat2DB Pro.exe")
    );
    verify_app_bundle(&outcome.app_path, Edition::Pro).unwrap();
}

#[cfg(windows)]
#[test]
fn rejects_fake_windows_exe_without_matching_msi_identity() {
    let temp = tempfile::tempdir().unwrap();
    let paths = test_paths(temp.path());
    let (client, _, registry) = desktop_artifact_client(
        Edition::Pro,
        HostOs::Windows,
        current_test_arch(),
        "3.1.15",
        b"fake-exe",
    );
    let installer = Installer::with_artifact_installer(
        client,
        paths,
        FakeArtifactInstaller {
            app_name: "unused.app",
            bundle_id: "unused",
        },
    )
    .with_desktop_artifact_registry(registry);

    let error = installer
        .install_for_os(
            InstallRequest {
                edition: Edition::Pro,
                force: false,
                progress: false,
            },
            HostOs::Windows,
        )
        .unwrap_err();

    assert!(matches!(error, Chat2DbError::BundleIdMismatch { .. }));
}

#[test]
fn installs_community_dmg_from_compiled_registry() {
    let temp = tempfile::tempdir().unwrap();
    let paths = test_paths(temp.path());
    let version = "5.3.0";
    let artifact = b"fake-community-dmg";
    let (client, artifact_url, registry) = desktop_artifact_client(
        Edition::Community,
        HostOs::Macos,
        current_test_arch(),
        version,
        artifact,
    );
    let installer = Installer::with_artifact_installer(
        client,
        paths.clone(),
        FakeArtifactInstaller {
            app_name: "Bundled Community Name.app",
            bundle_id: "com.chat2db.community",
        },
    )
    .with_desktop_artifact_registry(registry);

    let outcome = installer
        .install_for_os(
            InstallRequest {
                edition: Edition::Community,
                force: false,
                progress: false,
            },
            HostOs::Macos,
        )
        .unwrap();

    assert_eq!(outcome.artifact_url, artifact_url);
    assert_eq!(outcome.artifact_type, "dmg");
    assert_eq!(
        outcome.app_path,
        paths.applications_dir.join("Chat2DB Community.app")
    );
    verify_app_bundle(&outcome.app_path, Edition::Community).unwrap();
}

#[cfg(not(windows))]
#[test]
fn installs_community_msi_from_compiled_registry() {
    let temp = tempfile::tempdir().unwrap();
    let paths = test_paths(temp.path());
    let version = "5.3.0";
    let artifact = b"fake-community-msi";
    let (client, artifact_url, registry) = desktop_artifact_client(
        Edition::Community,
        HostOs::Windows,
        "x86_64",
        version,
        artifact,
    );
    let installer = Installer::with_artifact_installer(
        client,
        paths.clone(),
        FakeArtifactInstaller {
            app_name: "unused.app",
            bundle_id: "unused",
        },
    )
    .with_desktop_artifact_registry(registry);

    let outcome = installer
        .install_for_os(
            InstallRequest {
                edition: Edition::Community,
                force: false,
                progress: false,
            },
            HostOs::Windows,
        )
        .unwrap();

    assert_eq!(outcome.artifact_url, artifact_url);
    assert_eq!(outcome.artifact_type, "msi");
    verify_app_bundle(&outcome.app_path, Edition::Community).unwrap();
}

#[test]
fn community_install_rejects_artifact_checksum_mismatch() {
    let temp = tempfile::tempdir().unwrap();
    let paths = test_paths(temp.path());
    let version = "5.3.0";
    let (mut client, artifact_url, registry) = desktop_artifact_client(
        Edition::Community,
        HostOs::Macos,
        current_test_arch(),
        version,
        b"original",
    );
    client
        .responses
        .insert(artifact_url.clone(), b"tampered".to_vec());
    let installer = Installer::with_artifact_installer(
        client,
        paths,
        FakeArtifactInstaller {
            app_name: "unused.app",
            bundle_id: "unused",
        },
    )
    .with_desktop_artifact_registry(registry);

    let error = installer
        .install_for_os(
            InstallRequest {
                edition: Edition::Community,
                force: false,
                progress: false,
            },
            HostOs::Macos,
        )
        .unwrap_err();

    assert!(matches!(error, Chat2DbError::ChecksumMismatch { .. }));
}

#[test]
fn pro_install_rejects_artifact_checksum_mismatch() {
    let temp = tempfile::tempdir().unwrap();
    let paths = test_paths(temp.path());
    let (mut client, artifact_url, registry) = desktop_artifact_client(
        Edition::Pro,
        HostOs::Macos,
        current_test_arch(),
        "3.1.15",
        b"original",
    );
    client.responses.insert(artifact_url, b"tampered".to_vec());
    let installer = Installer::with_artifact_installer(
        client,
        paths,
        FakeArtifactInstaller {
            app_name: "unused.app",
            bundle_id: "unused",
        },
    )
    .with_desktop_artifact_registry(registry);

    let error = installer
        .install_for_os(
            InstallRequest {
                edition: Edition::Pro,
                force: false,
                progress: false,
            },
            HostOs::Macos,
        )
        .unwrap_err();

    assert!(matches!(error, Chat2DbError::ChecksumMismatch { .. }));
}

#[test]
fn windows_status_path_resolves_existing_programs_install() {
    let temp = tempfile::tempdir().unwrap();
    let paths = test_paths(temp.path());
    let app = paths
        .applications_dir
        .join("Programs")
        .join("Chat2DB-Pro")
        .join("Chat2DB-Pro.exe");
    fs::create_dir_all(app.parent().unwrap()).unwrap();
    fs::write(&app, b"fake-exe").unwrap();

    assert_eq!(
        Edition::Pro.resolve_app_path_for_os(&paths.applications_dir, HostOs::Windows),
        app
    );
    verify_app_bundle(&app, Edition::Pro).unwrap();
}

#[test]
fn force_only_replaces_verified_chat2db_app() {
    let temp = tempfile::tempdir().unwrap();
    let paths = test_paths(temp.path());
    let existing = paths.applications_dir.join("Chat2DB Pro.app");
    create_fake_app(&existing, "com.example.not-chat2db");

    let client = FakeHttp {
        responses: HashMap::new(),
    };
    let installer = Installer::new(client, paths);

    let err = installer
        .install(InstallRequest {
            edition: Edition::Pro,
            force: true,
            progress: false,
        })
        .unwrap_err();

    assert!(matches!(err, Chat2DbError::UnsafeReplace { .. }));
    verify_app_bundle(&existing, Edition::Pro).unwrap_err();
}

#[test]
fn install_refuses_existing_app_without_force() {
    let temp = tempfile::tempdir().unwrap();
    let paths = test_paths(temp.path());
    create_fake_app(
        &paths.applications_dir.join("Chat2DB Local.app"),
        "com.chat2db.local",
    );

    let client = FakeHttp {
        responses: HashMap::new(),
    };
    let installer = Installer::new(client, paths);

    let err = installer
        .install(InstallRequest {
            edition: Edition::Local,
            force: false,
            progress: false,
        })
        .unwrap_err();

    assert!(matches!(err, Chat2DbError::AppAlreadyExists { .. }));
}

#[test]
fn uninstall_removes_only_verified_app_and_preserves_edition_home() {
    let temp = tempfile::tempdir().unwrap();
    let paths = test_paths(temp.path());
    let app = paths.applications_dir.join("Chat2DB Local.app");
    create_fake_app(&app, "com.chat2db.local");
    let data_dir = paths.edition_home(Edition::Local);
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(data_dir.join("settings.json"), "{}").unwrap();

    let installer = Installer::new(
        FakeHttp {
            responses: HashMap::new(),
        },
        paths.clone(),
    );
    let outcome = installer
        .uninstall(UninstallRequest {
            edition: Edition::Local,
        })
        .unwrap();

    assert_eq!(outcome.status, "removed");
    assert_eq!(outcome.preserved_data_dir, data_dir);
    assert!(!app.exists());
    assert!(data_dir.join("settings.json").exists());
}

#[test]
fn uninstall_refuses_non_chat2db_app() {
    let temp = tempfile::tempdir().unwrap();
    let paths = test_paths(temp.path());
    let app = paths.applications_dir.join("Chat2DB Pro.app");
    create_fake_app(&app, "com.example.other");

    let installer = Installer::new(
        FakeHttp {
            responses: HashMap::new(),
        },
        paths,
    );
    let err = installer
        .uninstall(UninstallRequest {
            edition: Edition::Pro,
        })
        .unwrap_err();

    assert!(matches!(err, Chat2DbError::UnsafeUninstall { .. }));
    assert!(app.exists());
}

fn test_paths(root: &Path) -> SystemPaths {
    let applications_dir = root.join("Applications");
    let home_dir = root.join("home");
    fs::create_dir_all(&applications_dir).unwrap();
    fs::create_dir_all(&home_dir).unwrap();
    SystemPaths {
        applications_dir,
        home_dir,
        background_state_dir: root.join("state"),
    }
}

fn create_fake_app(app_path: &Path, bundle_id: &str) {
    let contents = app_path.join("Contents");
    fs::create_dir_all(&contents).unwrap();
    fs::write(contents.join("Info.plist"), plist_xml(bundle_id)).unwrap();
}

fn plist_xml(bundle_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>
  <string>{bundle_id}</string>
</dict>
</plist>
"#
    )
}

fn current_test_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => panic!("unsupported test arch: {other}"),
    }
}

fn desktop_artifact_client(
    edition: Edition,
    os: HostOs,
    arch: &str,
    version: &str,
    artifact: &[u8],
) -> (FakeHttp, String, Vec<u8>) {
    let arch = if os == HostOs::Windows {
        "x86_64"
    } else {
        arch
    };
    let name = edition.desktop_artifact_name(version, os, arch).unwrap();
    let (os_label, target) = match os {
        HostOs::Macos => (
            "darwin",
            format!("mac/{}", if arch == "arm64" { "arm64" } else { "x64" }),
        ),
        HostOs::Windows => ("windows", "windows".to_string()),
        HostOs::Linux | HostOs::Other => panic!("unsupported desktop fixture OS"),
    };
    let artifact_url = match edition {
        Edition::Community => {
            format!("https://github.com/OtterMind/Chat2DB/releases/download/v{version}/{name}")
        }
        Edition::Pro => {
            format!("https://cdn.chat2db-ai.com/download/latest/{target}/{name}")
        }
        Edition::Local => {
            format!("https://cdn.chat2db-ai.com/offline/latest/{target}/{name}")
        }
    };
    let checksum = match edition {
        Edition::Community => serde_json::json!({
            "algorithm": "sha256",
            "value": sha256_hex(artifact),
        }),
        Edition::Pro | Edition::Local => serde_json::json!({
            "algorithm": "sha512",
            "value": STANDARD.encode(Sha512::digest(artifact)),
        }),
    };
    let registry = serde_json::to_vec(&serde_json::json!({
        "schema_version": 1,
        "artifacts": [{
            "edition": edition.as_str(),
            "os": os_label,
            "arch": arch,
            "version": version,
            "url": artifact_url,
            "size": artifact.len(),
            "checksum": checksum,
        }],
    }))
    .unwrap();
    (
        FakeHttp {
            responses: HashMap::from([(artifact_url.clone(), artifact.to_vec())]),
        },
        artifact_url,
        registry,
    )
}
