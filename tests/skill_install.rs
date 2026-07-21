use std::fs;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

use chat2db::error::{Chat2DbError, Result};
use chat2db::http::HttpClient;
use chat2db::manifest::skill_manifest_url;
use chat2db::skill::{
    SkillClient, SkillInstallRequest, SkillTargetRequest, SkillUninstallRequest,
    SkillUpdateRequest, install, uninstall, update,
};
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;

const ARTIFACT_URL: &str =
    "https://cdn.chat2db-ai.com/download/chat2db-skill/0.1.0/chat2db-skill-0.1.0.zip";

struct FakeCdn {
    manifest: Vec<u8>,
    zip: Vec<u8>,
}

impl FakeCdn {
    fn new(marker: &str) -> Self {
        Self::with_skill("chat2db-cli", Some("0.1.0"), ">=0.1, <1", marker)
    }

    fn with_compat(compat: &str) -> Self {
        Self::with_skill("chat2db-cli", Some("0.1.0"), compat, "marker")
    }

    fn without_frontmatter_version() -> Self {
        Self::with_skill("chat2db-cli", None, ">=0.1, <1", "marker")
    }

    fn with_skill(name: &str, version: Option<&str>, compat: &str, marker: &str) -> Self {
        let zip = build_skill_zip(name, version, compat, marker);
        let sha = Sha256::digest(&zip);
        let manifest = format!(
            r#"{{
  "skill_version": "0.1.0",
  "compatible_cli_versions": "{compat}",
  "bundle_path": "chat2db-cli",
  "artifact": {{
    "url": "{ARTIFACT_URL}",
    "sha256": "{sha:x}",
    "artifact_type": "skill_zip"
  }}
}}"#
        )
        .into_bytes();
        Self { manifest, zip }
    }
}

impl HttpClient for FakeCdn {
    fn get(&self, url: &str) -> Result<Vec<u8>> {
        if url == skill_manifest_url() {
            Ok(self.manifest.clone())
        } else if url == ARTIFACT_URL {
            Ok(self.zip.clone())
        } else {
            Err(Chat2DbError::UnsafeZipPath(format!(
                "tests must not hit the network: {url}"
            )))
        }
    }
}

fn build_skill_zip(name: &str, version: Option<&str>, compat: &str, marker: &str) -> Vec<u8> {
    let file = tempfile::tempfile().unwrap();
    let mut writer = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default();
    writer.add_directory("chat2db-cli/", opts).unwrap();
    writer.start_file("chat2db-cli/SKILL.md", opts).unwrap();
    let version_line = version
        .map(|version| format!("version: \"{version}\"\n"))
        .unwrap_or_default();
    writer
        .write_all(
            format!(
                "---\nname: {name}\n{version_line}compatible-cli-versions: \"{compat}\"\n---\n# body\n"
            )
            .as_bytes(),
        )
        .unwrap();
    writer
        .add_directory("chat2db-cli/references/", opts)
        .unwrap();
    writer
        .start_file("chat2db-cli/references/marker.md", opts)
        .unwrap();
    writer.write_all(marker.as_bytes()).unwrap();
    let mut file = writer.finish().unwrap();
    file.rewind().unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    bytes
}

fn cli_version() -> String {
    "0.1.0".to_string()
}

fn write_skill_md(dir: &Path, name: &str, compat: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        format!(
            "---\nname: {name}\nversion: \"0.1.0\"\ncompatible-cli-versions: \"{compat}\"\n---\n# body\n"
        ),
    )
    .unwrap();
}

fn target_dir(current: &Path) -> PathBuf {
    current.join("chat2db-cli")
}

fn backup_dirs(parent: &Path) -> Vec<PathBuf> {
    fs::read_dir(parent)
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            let name = path.file_name()?.to_str()?;
            name.starts_with("chat2db-cli.bak-").then_some(path)
        })
        .collect()
}

fn target_request(client: Option<SkillClient>, home: &Path, current: &Path) -> SkillTargetRequest {
    SkillTargetRequest {
        client,
        home_dir: home.to_path_buf(),
        current_dir: current.to_path_buf(),
    }
}

fn install_request(
    client: Option<SkillClient>,
    home: &Path,
    current: &Path,
    force: bool,
) -> SkillInstallRequest {
    SkillInstallRequest {
        target: target_request(client, home, current),
        force,
        progress: false,
        cli_version: cli_version(),
    }
}

#[test]
fn install_clean_current_dir_creates_bundle_at_canonical_path() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let current = temp.path().join("project");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&current).unwrap();

    let outcome = install(
        &FakeCdn::new("installed"),
        install_request(None, &home, &current, false),
    )
    .unwrap();

    assert_eq!(outcome.command, "skill");
    assert_eq!(outcome.operation, "install");
    assert_eq!(outcome.status, "installed");
    assert_eq!(outcome.client, None);
    let target = target_dir(&current);
    assert_eq!(outcome.target_path, target);
    assert!(target.join("SKILL.md").is_file());
    assert_eq!(
        fs::read_to_string(target.join("references/marker.md")).unwrap(),
        "installed"
    );
}

#[test]
fn install_with_client_writes_global_client_skill_dir() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let current = temp.path().join("project");

    let claude = install(
        &FakeCdn::new("claude"),
        install_request(Some(SkillClient::ClaudeCode), &home, &current, false),
    )
    .unwrap();
    assert_eq!(claude.client, Some(SkillClient::ClaudeCode));
    assert_eq!(claude.target_path, home.join(".claude/skills/chat2db-cli"));

    let codex = install(
        &FakeCdn::new("codex"),
        install_request(Some(SkillClient::Codex), &home, &current, false),
    )
    .unwrap();
    assert_eq!(codex.client, Some(SkillClient::Codex));
    assert_eq!(codex.target_path, home.join(".codex/skills/chat2db-cli"));
}

#[test]
fn reinstall_without_force_is_already_installed_error() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let current = temp.path().join("project");

    install(
        &FakeCdn::new("first"),
        install_request(None, &home, &current, false),
    )
    .unwrap();

    let err = install(
        &FakeCdn::new("second"),
        install_request(None, &home, &current, false),
    )
    .unwrap_err();
    assert!(matches!(err, Chat2DbError::SkillAlreadyInstalled { .. }));
}

#[test]
fn reinstall_with_force_overwrites_old_bundle_without_backup() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let current = temp.path().join("project");

    install(
        &FakeCdn::new("old"),
        install_request(None, &home, &current, false),
    )
    .unwrap();

    let outcome = install(
        &FakeCdn::new("new"),
        install_request(None, &home, &current, true),
    )
    .unwrap();

    assert_eq!(outcome.status, "updated");
    let target = target_dir(&current);
    assert_eq!(
        fs::read_to_string(target.join("references/marker.md")).unwrap(),
        "new"
    );
    assert!(backup_dirs(&current).is_empty());
}

#[test]
fn git_target_blocks_install_update_and_uninstall() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let current = temp.path().join("project");
    let target = target_dir(&current);
    write_skill_md(&target, "chat2db-cli", ">=0.1");
    fs::create_dir_all(target.join(".git")).unwrap();

    let install_err = install(
        &FakeCdn::new("install"),
        install_request(None, &home, &current, true),
    )
    .unwrap_err();
    assert!(matches!(
        install_err,
        Chat2DbError::SkillTargetManagedExternally { .. }
    ));

    let update_err = update(
        &FakeCdn::new("update"),
        SkillUpdateRequest {
            target: target_request(None, &home, &current),
            progress: false,
            cli_version: cli_version(),
        },
    )
    .unwrap_err();
    assert!(matches!(
        update_err,
        Chat2DbError::SkillTargetManagedExternally { .. }
    ));

    let uninstall_err = uninstall(SkillUninstallRequest {
        target: target_request(None, &home, &current),
    })
    .unwrap_err();
    assert!(matches!(
        uninstall_err,
        Chat2DbError::SkillTargetManagedExternally { .. }
    ));
}

#[test]
fn foreign_target_requires_force_and_uninstall_refuses() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let current = temp.path().join("project");
    let target = target_dir(&current);
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("README.md"), "hello world").unwrap();

    let err = install(
        &FakeCdn::new("new"),
        install_request(None, &home, &current, false),
    )
    .unwrap_err();
    assert!(matches!(err, Chat2DbError::SkillTargetForeign { .. }));

    let uninstall_err = uninstall(SkillUninstallRequest {
        target: target_request(None, &home, &current),
    })
    .unwrap_err();
    assert!(matches!(
        uninstall_err,
        Chat2DbError::SkillTargetForeign { .. }
    ));

    let outcome = install(
        &FakeCdn::new("new"),
        install_request(None, &home, &current, true),
    )
    .unwrap();
    assert_eq!(outcome.status, "installed");
    assert!(target.join("SKILL.md").is_file());
    assert!(
        target
            .join("README.md")
            .try_exists()
            .is_ok_and(|exists| !exists)
    );
    assert!(backup_dirs(&current).is_empty());
}

#[test]
fn incompatible_compat_range_aborts_without_writing() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let current = temp.path().join("project");

    let err = install(
        &FakeCdn::with_compat(">=99"),
        install_request(None, &home, &current, false),
    )
    .unwrap_err();
    assert!(matches!(err, Chat2DbError::SkillVersionIncompatible { .. }));

    assert!(!target_dir(&current).exists());
}

#[test]
fn uninstall_happy_path_then_already_absent() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let current = temp.path().join("project");

    install(
        &FakeCdn::new("present"),
        install_request(None, &home, &current, false),
    )
    .unwrap();
    assert!(target_dir(&current).exists());

    let removed = uninstall(SkillUninstallRequest {
        target: target_request(None, &home, &current),
    })
    .unwrap();
    assert_eq!(removed.status, "removed");
    assert_eq!(removed.operation, "uninstall");
    assert!(!target_dir(&current).exists());

    let absent = uninstall(SkillUninstallRequest {
        target: target_request(None, &home, &current),
    })
    .unwrap();
    assert_eq!(absent.status, "already_absent");
}

#[test]
fn update_against_installed_bundle_marks_operation_update() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let current = temp.path().join("project");

    install(
        &FakeCdn::new("stale"),
        install_request(None, &home, &current, false),
    )
    .unwrap();
    let skill_md = target_dir(&current).join("SKILL.md");
    let installed = fs::read_to_string(&skill_md).unwrap();
    fs::write(
        &skill_md,
        installed.replacen("version: \"0.1.0\"", "version: \"0.0.9\"", 1),
    )
    .unwrap();

    let outcome = update(
        &FakeCdn::new("fresh"),
        SkillUpdateRequest {
            target: target_request(None, &home, &current),
            progress: false,
            cli_version: cli_version(),
        },
    )
    .unwrap();

    assert_eq!(outcome.operation, "update");
    assert_eq!(outcome.status, "updated");
    assert_eq!(
        fs::read_to_string(target_dir(&current).join("references/marker.md")).unwrap(),
        "fresh"
    );
    assert!(backup_dirs(&current).is_empty());
}

#[test]
fn install_rejects_cdn_bundle_without_frontmatter_version() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let current = temp.path().join("project");

    let err = install(
        &FakeCdn::without_frontmatter_version(),
        install_request(None, &home, &current, false),
    )
    .unwrap_err();
    let reason = match err {
        Chat2DbError::SkillBundleCorrupt { reason } => reason,
        other => panic!("expected SkillBundleCorrupt, got {other:?}"),
    };
    assert!(
        reason.contains("version"),
        "reason should mention missing version: {reason}"
    );
    assert!(!target_dir(&current).exists());
}

#[test]
fn skill_install_script_uses_github_release_manifest_and_cdn_artifact() {
    let script = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/install-skill.sh"),
    )
    .unwrap();
    assert!(script.contains("https://github.com/OtterMind/Chat2DB-CLI/releases"));
    assert!(script.contains("$MANIFEST_BASE/latest/download/manifest-skill.json"));
    assert!(script.contains("$MANIFEST_BASE/download/v$VERSION/manifest-skill.json"));
    assert!(script.contains("| bash"));
    assert!(!script.contains("| sh\n"));
}

#[cfg(unix)]
mod unix_script_tests {
    use super::*;
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Output};
    use std::thread;

    const SCRIPT_VERSION: &str = "0.2.0";

    struct ScriptRelease {
        manifest_base_url: String,
        cdn_base_url: String,
    }

    impl ScriptRelease {
        fn start() -> Self {
            Self::start_with_manifest_transform(|manifest| manifest)
        }

        fn start_with_manifest_transform(transform: fn(String) -> String) -> Self {
            let bundle = build_skill_script_zip();
            let sha256 = format!("{:x}", Sha256::digest(&bundle));
            let artifact_listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let cdn_base_url = format!("http://{}", artifact_listener.local_addr().unwrap());
            let archive_name = format!("chat2db-skill-{SCRIPT_VERSION}.zip");
            let artifact_path = format!("/download/chat2db-skill/{SCRIPT_VERSION}/{archive_name}");
            spawn_script_server(
                artifact_listener,
                vec![(artifact_path.clone(), "application/zip", bundle)],
            );

            let manifest = transform(format!(
                "{{\n  \"skill_version\": \"{SCRIPT_VERSION}\",\n  \"compatible_cli_versions\": \">=0.2,<0.3\",\n  \"bundle_path\": \"chat2db-cli\",\n  \"artifact\": {{\n    \"url\": \"{cdn_base_url}{artifact_path}\",\n    \"sha256\": \"{sha256}\",\n    \"artifact_type\": \"skill_zip\"\n  }}\n}}\n"
            ));
            let manifest_listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let manifest_base_url = format!("http://{}", manifest_listener.local_addr().unwrap());
            spawn_script_server(
                manifest_listener,
                vec![
                    (
                        "/latest/download/manifest-skill.json".to_string(),
                        "application/json",
                        manifest.as_bytes().to_vec(),
                    ),
                    (
                        format!("/download/v{SCRIPT_VERSION}/manifest-skill.json"),
                        "application/json",
                        manifest.into_bytes(),
                    ),
                ],
            );

            Self {
                manifest_base_url,
                cdn_base_url,
            }
        }
    }

    #[test]
    fn skill_install_script_installs_from_github_release_manifest_and_cdn_artifact() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let current = temp.path().join("project");
        let target = current.join("installed-skill");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&current).unwrap();
        let release = ScriptRelease::start();

        let output = run_skill_script(&home, &current, &target, &release, &[]);

        assert_script_success(&output);
        assert!(target.join("SKILL.md").is_file());
        assert_eq!(
            fs::read_to_string(target.join("references/marker.md")).unwrap(),
            "installed"
        );
    }

    #[test]
    fn skill_install_script_supports_pinned_release_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let current = temp.path().join("project");
        let target = current.join("installed-skill");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&current).unwrap();
        let release = ScriptRelease::start();

        let output = run_skill_script(
            &home,
            &current,
            &target,
            &release,
            &[("CHAT2DB_SKILL_VERSION", SCRIPT_VERSION)],
        );

        assert_script_success(&output);
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains(&format!("/download/v{SCRIPT_VERSION}/manifest-skill.json"))
        );
    }

    #[test]
    fn skill_install_script_rejects_canonical_home_alias() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let current = temp.path().join("project");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&current).unwrap();
        fs::write(home.join("sentinel"), "preserve").unwrap();
        let target = home.join("..").join(home.file_name().unwrap());
        let release = ScriptRelease::start();

        let output = run_skill_script(
            &home,
            &current,
            &target,
            &release,
            &[("CHAT2DB_SKILL_FORCE", "1")],
        );

        assert!(!output.status.success());
        assert_eq!(
            fs::read_to_string(home.join("sentinel")).unwrap(),
            "preserve"
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("refusing to replace HOME"));
    }

    #[test]
    fn skill_install_script_rejects_symlink_target_with_force() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let current = temp.path().join("project");
        let real_target = current.join("real-target");
        let target = current.join("installed-skill");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&real_target).unwrap();
        fs::write(real_target.join("sentinel"), "preserve").unwrap();
        symlink(&real_target, &target).unwrap();
        let release = ScriptRelease::start();

        let output = run_skill_script(
            &home,
            &current,
            &target,
            &release,
            &[("CHAT2DB_SKILL_FORCE", "1")],
        );

        assert!(!output.status.success());
        assert_eq!(
            fs::read_to_string(real_target.join("sentinel")).unwrap(),
            "preserve"
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("symlink target"));
    }

    #[test]
    fn skill_install_script_rejects_git_file_with_force() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let current = temp.path().join("project");
        let target = current.join("installed-skill");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(
            target.join("SKILL.md"),
            "---\nname: chat2db-cli\nversion: \"0.1.0\"\n---\n",
        )
        .unwrap();
        fs::write(target.join(".git"), "gitdir: ../worktrees/skill\n").unwrap();
        let release = ScriptRelease::start();

        let output = run_skill_script(
            &home,
            &current,
            &target,
            &release,
            &[("CHAT2DB_SKILL_FORCE", "1")],
        );

        assert!(!output.status.success());
        assert_eq!(
            fs::read_to_string(target.join(".git")).unwrap(),
            "gitdir: ../worktrees/skill\n"
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("contains .git"));
    }

    #[test]
    fn skill_install_script_restores_previous_bundle_when_activation_fails() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let current = temp.path().join("project");
        let target = current.join("installed-skill");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(target.join("references")).unwrap();
        let previous_skill = "---\nname: chat2db-cli\nversion: \"0.1.0\"\n---\n# Previous\n";
        fs::write(target.join("SKILL.md"), previous_skill).unwrap();
        fs::write(target.join("references/marker.md"), "previous").unwrap();
        let release = ScriptRelease::start();
        let wrapper_dir = temp.path().join("wrappers");
        fs::create_dir_all(&wrapper_dir).unwrap();
        let counter = temp.path().join("mv-count");
        let mv_wrapper = wrapper_dir.join("mv");
        fs::write(
        &mv_wrapper,
        "#!/bin/sh\ncount=$(cat \"$CHAT2DB_TEST_MV_COUNT\" 2>/dev/null || echo 0)\ncount=$((count + 1))\nprintf '%s\\n' \"$count\" > \"$CHAT2DB_TEST_MV_COUNT\"\nif [ \"$count\" -eq \"$CHAT2DB_TEST_MV_FAIL_AT\" ]; then exit 73; fi\nexec /bin/mv \"$@\"\n",
    )
    .unwrap();
        fs::set_permissions(&mv_wrapper, fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("{}:{}", wrapper_dir.display(), script_test_path());

        let output = run_skill_script(
            &home,
            &current,
            &target,
            &release,
            &[
                ("PATH", &path),
                ("CHAT2DB_SKILL_FORCE", "1"),
                ("CHAT2DB_TEST_MV_COUNT", counter.to_str().unwrap()),
                ("CHAT2DB_TEST_MV_FAIL_AT", "3"),
            ],
        );

        assert!(!output.status.success());
        assert_eq!(
            fs::read_to_string(target.join("SKILL.md")).unwrap(),
            previous_skill
        );
        assert_eq!(
            fs::read_to_string(target.join("references/marker.md")).unwrap(),
            "previous"
        );
        assert!(fs::read_dir(&current).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".chat2db-cli.")
        }));
    }

    #[test]
    fn skill_install_script_rejects_untrusted_manifest_fields() {
        let transforms: [fn(String) -> String; 6] = [
            |manifest| manifest.replace(".zip\"", ".zip?download=1\""),
            |manifest| manifest.replace("http://", "http://user@"),
            |manifest| {
                manifest.replace(
                    &format!("\"skill_version\": \"{SCRIPT_VERSION}\""),
                    "\"skill_version\": \"invalid\"",
                )
            },
            |manifest| manifest.replace("\"sha256\": \"", "\"sha256\": \"ABC"),
            |manifest| {
                manifest.replace(
                    "\"bundle_path\": \"chat2db-cli\"",
                    "\"bundle_path\": \"bundle\"",
                )
            },
            |manifest| {
                manifest.replace(
                    "{\n",
                    &format!("{{\n  \"skill_version\": \"{SCRIPT_VERSION}\",\n"),
                )
            },
        ];

        for transform in transforms {
            let temp = tempfile::tempdir().unwrap();
            let home = temp.path().join("home");
            let current = temp.path().join("project");
            let target = current.join("installed-skill");
            fs::create_dir_all(&home).unwrap();
            fs::create_dir_all(&current).unwrap();
            let release = ScriptRelease::start_with_manifest_transform(transform);

            let output = run_skill_script(&home, &current, &target, &release, &[]);

            assert!(!output.status.success());
            assert!(!target.exists());
        }
    }

    fn run_skill_script(
        home: &Path,
        current: &Path,
        target: &Path,
        release: &ScriptRelease,
        extra_env: &[(&str, &str)],
    ) -> Output {
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/install-skill.sh");
        let mut command = Command::new("env");
        command
            .arg("-i")
            .arg(format!("HOME={}", home.display()))
            .arg(format!(
                "CHAT2DB_SKILL_MANIFEST_BASE={}",
                release.manifest_base_url
            ))
            .arg(format!("CHAT2DB_SKILL_CDN_BASE={}", release.cdn_base_url))
            .arg(format!("PATH={}", script_test_path()))
            .current_dir(current);
        for (name, value) in extra_env {
            command.arg(format!("{name}={value}"));
        }
        command
            .arg("bash")
            .arg(script)
            .arg("--target")
            .arg(target)
            .output()
            .unwrap()
    }

    fn build_skill_script_zip() -> Vec<u8> {
        let file = tempfile::tempfile().unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default();
        writer.start_file("LICENSE", opts).unwrap();
        writer.write_all(b"Apache License fixture").unwrap();
        writer.start_file("THIRD_PARTY_LICENSES.txt", opts).unwrap();
        writer.write_all(b"Third-party license fixture").unwrap();
        writer.start_file("chat2db-cli/SKILL.md", opts).unwrap();
        writer
        .write_all(
            format!(
                "---\nname: chat2db-cli\nversion: \"{SCRIPT_VERSION}\"\ncompatible-cli-versions: \">=0.2,<0.3\"\n---\n# body\n"
            )
            .as_bytes(),
        )
        .unwrap();
        writer
            .start_file("chat2db-cli/references/marker.md", opts)
            .unwrap();
        writer.write_all(b"installed").unwrap();
        let mut file = writer.finish().unwrap();
        file.rewind().unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        bytes
    }

    fn spawn_script_server(listener: TcpListener, routes: Vec<(String, &'static str, Vec<u8>)>) {
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else {
                    continue;
                };
                let mut buffer = [0_u8; 4096];
                let bytes_read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..bytes_read]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                if let Some((_, content_type, body)) =
                    routes.iter().find(|(route, _, _)| route == path)
                {
                    write_script_response(&mut stream, "200 OK", content_type, body);
                } else {
                    write_script_response(&mut stream, "404 Not Found", "text/plain", b"not found");
                }
            }
        });
    }

    fn write_script_response(
        stream: &mut impl Write,
        status: &str,
        content_type: &str,
        body: &[u8],
    ) {
        let header = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(header.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
    }

    fn script_test_path() -> String {
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin:/usr/sbin:/sbin".to_string())
    }

    fn assert_script_success(output: &Output) {
        assert!(
            output.status.success(),
            "installer failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
