use std::fs;
#[cfg(windows)]
use std::io::Seek;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;

use sha2::{Digest, Sha256};
#[cfg(windows)]
use zip::write::SimpleFileOptions;

const TEST_VERSION: &str = "0.2.0";

#[test]
fn windows_install_script_targets_windows_zip_manifest() {
    let script = windows_install_script();

    assert!(script.contains("$ManifestBase/latest/download/$manifestName"));
    assert!(script.contains("$ManifestBase/download/v$Version/$manifestName"));
    assert!(script.contains("manifest-cli-windows-$arch.json"));
    assert!(script.contains(
        "Windows CLI archive must contain only chat2db.exe, LICENSE, and THIRD_PARTY_LICENSES.txt at the root"
    ));
    assert!(script.contains("Join-Path $extractDir \"chat2db.exe\""));
    assert!(!script.contains("Expand-Archive"));
}

#[test]
fn windows_arm_uses_published_x86_64_artifact() {
    let script = windows_install_script();

    assert!(script.contains("\"ARM64\" { return \"x86_64\" }"));
    assert!(!script.contains("\"ARM64\" { \"arm64\" }"));
}

#[test]
fn windows_x86_requires_a_64_bit_native_architecture() {
    let script = windows_install_script();

    assert!(script.contains("$env:PROCESSOR_ARCHITEW6432"));
    assert!(script.contains("32-bit Windows is not supported"));
    assert!(!script.contains("\"x86\" { \"x86_64\" }"));
}

#[test]
fn windows_install_script_defaults_to_local_app_data_programs_dir() {
    let script = windows_install_script();

    assert!(script.contains("$localAppData = $env:LOCALAPPDATA"));
    assert!(script.contains("Join-Path $env:USERPROFILE \"AppData\\Local\""));
    assert!(script.contains("Join-Path $localAppData \"Programs\\chat2db-cli\""));
    assert!(script.contains("Join-Path $BinDir \"chat2db.exe\""));
}

#[test]
fn windows_install_script_supports_force_and_no_modify_path() {
    let script = windows_install_script();

    assert!(script.contains("[switch]$Force"));
    assert!(script.contains("[switch]$NoModifyPath"));
    assert!(script.contains("rerun with -Force to replace"));
    assert!(script.contains("$env:CHAT2DB_CLI_NO_MODIFY_PATH -ne \"1\""));
    assert!(script.contains("[Environment]::SetEnvironmentVariable(\"Path\", $newPath, \"User\")"));
}

#[test]
fn windows_install_script_hashes_without_optional_cmdlets() {
    let script = windows_install_script();

    assert!(script.contains("[System.Security.Cryptography.SHA256]::Create()"));
    assert!(!script.contains("Get-FileHash"));
}

#[test]
fn installers_default_to_github_release_manifests() {
    let unix = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/install-cli.sh"),
    )
    .unwrap();
    let windows = windows_install_script();

    for script in [&unix, &windows] {
        assert!(script.contains("https://github.com/OtterMind/Chat2DB-CLI/releases"));
        assert!(script.contains("CHAT2DB_CLI_MANIFEST_BASE"));
    }
}

#[cfg(unix)]
mod unix_install_tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::os::unix::fs::PermissionsExt;

    const PATH_MARKER: &str = "# >>> chat2db PATH >>>";

    struct FakeRelease {
        manifest_base_url: String,
        cdn_base_url: String,
    }

    #[test]
    fn install_script_writes_zsh_path_block() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let release = FakeRelease::start();

        let output = run_installer(&home, Some("/bin/zsh"), &release, &[], &[]);

        assert_success(&output);
        let zshrc = home.join(".zshrc");
        let rc_text = fs::read_to_string(&zshrc).unwrap();
        assert!(rc_text.contains(PATH_MARKER));
        assert!(rc_text.contains("export PATH=\"$HOME/.local/bin:$PATH\""));
        let stdout = stdout(&output);
        assert!(stdout.contains(&format!(
            "path: added {} to {}",
            home.join(".local/bin").display(),
            zshrc.display()
        )));
        assert!(stdout.contains("installed:"));
    }

    #[test]
    fn install_script_writes_bash_path_block() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let release = FakeRelease::start();

        let output = run_installer(&home, Some("/bin/bash"), &release, &[], &[]);

        assert_success(&output);
        let bashrc = home.join(".bashrc");
        let rc_text = fs::read_to_string(&bashrc).unwrap();
        assert!(rc_text.contains(PATH_MARKER));
        assert!(rc_text.contains("export PATH=\"$HOME/.local/bin:$PATH\""));
        assert!(stdout(&output).contains(&format!(
            "path: added {} to {}",
            home.join(".local/bin").display(),
            bashrc.display()
        )));
    }

    #[test]
    fn install_script_writes_fish_path_block() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let release = FakeRelease::start();

        let output = run_installer(&home, Some("/usr/local/bin/fish"), &release, &[], &[]);

        assert_success(&output);
        let fish_config = home.join(".config/fish/config.fish");
        let rc_text = fs::read_to_string(&fish_config).unwrap();
        assert!(rc_text.contains(PATH_MARKER));
        assert!(rc_text.contains("fish_add_path \"$HOME/.local/bin\""));
        assert!(stdout(&output).contains(&format!(
            "path: added {} to {}",
            home.join(".local/bin").display(),
            fish_config.display()
        )));
    }

    #[test]
    fn install_script_path_block_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let release = FakeRelease::start();

        assert_success(&run_installer(&home, Some("/bin/zsh"), &release, &[], &[]));
        let output = run_installer(&home, Some("/bin/zsh"), &release, &[], &[]);

        assert_success(&output);
        let rc_text = fs::read_to_string(home.join(".zshrc")).unwrap();
        assert_eq!(rc_text.matches(PATH_MARKER).count(), 1);
        assert!(stdout(&output).contains("path: already configured"));
    }

    #[test]
    fn install_script_no_modify_path_skips_rc_and_prints_manual_hint() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let release = FakeRelease::start();

        let output = run_installer(
            &home,
            Some("/bin/zsh"),
            &release,
            &["--no-modify-path"],
            &[],
        );

        assert_success(&output);
        assert!(!home.join(".zshrc").exists());
        let stdout = stdout(&output);
        assert!(stdout.contains("shell rc not modified"));
        assert!(stdout.contains("export PATH=\"$HOME/.local/bin:$PATH\""));
    }

    #[test]
    fn install_script_no_modify_path_env_skips_rc() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let release = FakeRelease::start();

        let output = run_installer(
            &home,
            Some("/bin/bash"),
            &release,
            &[],
            &[("CHAT2DB_CLI_NO_MODIFY_PATH", "1")],
        );

        assert_success(&output);
        assert!(!home.join(".bashrc").exists());
        assert!(stdout(&output).contains("shell rc not modified"));
    }

    #[test]
    fn install_script_missing_shell_prints_manual_hint_without_failure() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let release = FakeRelease::start();

        let output = run_installer(&home, None, &release, &[], &[]);

        assert_success(&output);
        assert!(!home.join(".zshrc").exists());
        assert!(!home.join(".bashrc").exists());
        assert!(!home.join(".config/fish/config.fish").exists());
        let stdout = stdout(&output);
        assert!(stdout.contains("add it manually"));
        assert!(stdout.contains("export PATH=\"$HOME/.local/bin:$PATH\""));
    }

    #[test]
    fn install_script_unsupported_shell_prints_manual_hint_without_rc() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let release = FakeRelease::start();

        let output = run_installer(&home, Some("/bin/tcsh"), &release, &[], &[]);

        assert_success(&output);
        assert!(!home.join(".zshrc").exists());
        assert!(!home.join(".bashrc").exists());
        assert!(!home.join(".config/fish/config.fish").exists());
        let stdout = stdout(&output);
        assert!(stdout.contains("add it manually"));
        assert!(stdout.contains("export PATH=\"$HOME/.local/bin:$PATH\""));
    }

    #[test]
    fn install_script_legacy_no_path_warn_skips_rc_and_path_output() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let release = FakeRelease::start();

        let output = run_installer(
            &home,
            Some("/bin/zsh"),
            &release,
            &[],
            &[("CHAT2DB_CLI_NO_PATH_WARN", "1")],
        );

        assert_success(&output);
        assert!(!home.join(".zshrc").exists());
        let stdout = stdout(&output);
        assert!(stdout.contains("installed:"));
        assert!(!stdout.contains("path:"));
    }

    #[test]
    fn install_script_accepts_force_with_no_modify_path() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let bin_dir = home.join(".local/bin");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(bin_dir.join("chat2db"), "#!/bin/sh\necho not-chat2db\n").unwrap();
        let release = FakeRelease::start();

        let output = run_installer(
            &home,
            Some("/bin/zsh"),
            &release,
            &["--force", "--no-modify-path"],
            &[],
        );

        assert_success(&output);
        assert!(!home.join(".zshrc").exists());
        assert!(stdout(&output).contains("installed:"));
    }

    #[test]
    fn install_script_restores_previous_binary_when_activation_fails() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let bin_dir = home.join(".local/bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let target = bin_dir.join("chat2db");
        let previous = "#!/bin/sh\necho 'chat2db previous'\n";
        fs::write(&target, previous).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        let release = FakeRelease::start();
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
        let path = format!("{}:{}", wrapper_dir.display(), test_path());

        let output = run_installer(
            &home,
            Some("/bin/sh"),
            &release,
            &["--no-modify-path"],
            &[
                ("PATH", &path),
                ("CHAT2DB_TEST_MV_COUNT", counter.to_str().unwrap()),
                ("CHAT2DB_TEST_MV_FAIL_AT", "2"),
            ],
        );

        assert!(!output.status.success());
        assert_eq!(fs::read_to_string(&target).unwrap(), previous);
        assert!(fs::read_dir(&bin_dir).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".chat2db.")
        }));
    }

    #[test]
    fn install_script_supports_a_pinned_release_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let release = FakeRelease::start();

        let output = run_installer(
            &home,
            Some("/bin/zsh"),
            &release,
            &["--no-modify-path"],
            &[("CHAT2DB_CLI_VERSION", TEST_VERSION)],
        );

        assert_success(&output);
        assert!(stdout(&output).contains(&format!(
            "/download/v{TEST_VERSION}/manifest-cli-{}-{}.json",
            current_os_label(),
            current_arch_label()
        )));
    }

    #[test]
    fn install_script_rejects_noncanonical_manifest_bases() {
        for base in [
            "http://example.com",
            "https://user@example.com/releases",
            "https://github.com/releases?channel=latest",
            "https://github.com/releases/../other",
            "https://github.com:443/releases",
            "https://GITHUB.com/releases",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let home = temp.path().join("home");
            fs::create_dir_all(&home).unwrap();
            let release = FakeRelease::start();
            let output = run_installer(
                &home,
                Some("/bin/zsh"),
                &release,
                &["--no-modify-path"],
                &[("CHAT2DB_CLI_MANIFEST_BASE", base)],
            );

            assert!(!output.status.success(), "unsafe base was accepted: {base}");
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("CHAT2DB_CLI_MANIFEST_BASE"),
                "unexpected error for {base}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn install_script_rejects_noncanonical_cdn_origins() {
        for base in [
            "http://example.com",
            "https://user@cdn.example.com",
            "https://cdn.example.com/download",
            "https://cdn.example.com?channel=latest",
            "https://cdn.example.com:443",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let home = temp.path().join("home");
            fs::create_dir_all(&home).unwrap();
            let release = FakeRelease::start();
            let output = run_installer(
                &home,
                Some("/bin/zsh"),
                &release,
                &["--no-modify-path"],
                &[("CHAT2DB_CLI_CDN_BASE", base)],
            );

            assert!(
                !output.status.success(),
                "unsafe CDN base was accepted: {base}"
            );
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("CHAT2DB_CLI_CDN_BASE"),
                "unexpected error for {base}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn install_script_rejects_artifact_url_outside_exact_release_coordinates() {
        let transforms: [fn(String) -> String; 4] = [
            |manifest| manifest.replace(".tar.gz\"", ".tar.gz?download=1\""),
            |manifest| manifest.replace("http://", "http://user@"),
            |manifest| {
                manifest.replace(
                    &format!("/{}/{}/", current_os_label(), current_arch_label()),
                    &format!("/{}/other/", current_os_label()),
                )
            },
            |manifest| manifest.replace(".tar.gz\"", ".zip\""),
        ];

        for transform in transforms {
            let temp = tempfile::tempdir().unwrap();
            let home = temp.path().join("home");
            fs::create_dir_all(&home).unwrap();
            let release = FakeRelease::start_with_manifest_transform(transform);
            let output = run_installer(
                &home,
                Some("/bin/zsh"),
                &release,
                &["--no-modify-path"],
                &[],
            );

            assert!(!output.status.success());
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("unofficial_artifact_url"),
                "unexpected installer error: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn install_script_rejects_invalid_manifest_version_type_and_sha() {
        let transforms: [fn(String) -> String; 4] = [
            |manifest| {
                manifest.replace(
                    &format!("\"version\": \"{TEST_VERSION}\""),
                    "\"version\": \"not-semver\"",
                )
            },
            |manifest| {
                manifest.replace(
                    "\"artifact_type\": \"tarball\"",
                    "\"artifact_type\": \"zip\"",
                )
            },
            |manifest| manifest.replace("\"sha256\": \"", "\"sha256\": \"xyz"),
            |manifest| {
                manifest.replace("{\n", &format!("{{\n  \"version\": \"{TEST_VERSION}\",\n"))
            },
        ];

        for transform in transforms {
            let temp = tempfile::tempdir().unwrap();
            let home = temp.path().join("home");
            fs::create_dir_all(&home).unwrap();
            let release = FakeRelease::start_with_manifest_transform(transform);
            let output = run_installer(
                &home,
                Some("/bin/zsh"),
                &release,
                &["--no-modify-path"],
                &[],
            );

            assert!(!output.status.success());
            assert!(!home.join(".local/bin/chat2db").exists());
        }
    }

    impl FakeRelease {
        fn start() -> Self {
            Self::start_with_manifest_transform(|manifest| manifest)
        }

        fn start_with_manifest_transform(transform: fn(String) -> String) -> Self {
            let tarball = fake_cli_tarball();
            let sha256 = format!("{:x}", Sha256::digest(&tarball));
            let artifact_listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let cdn_base_url = format!("http://{}", artifact_listener.local_addr().unwrap());
            let archive_name = format!(
                "chat2db-{TEST_VERSION}-{}-{}.tar.gz",
                current_os_label(),
                current_arch_label()
            );
            let artifact_path = format!(
                "/download/chat2db-cli/{TEST_VERSION}/{}/{}/{}",
                current_os_label(),
                current_arch_label(),
                archive_name
            );
            spawn_http_server(
                artifact_listener,
                vec![(artifact_path.clone(), "application/gzip", tarball)],
            );

            let manifest = transform(format!(
                "{{\n  \"version\": \"{TEST_VERSION}\",\n  \"artifact\": {{\n    \"url\": \"{cdn_base_url}{artifact_path}\",\n    \"sha256\": \"{sha256}\",\n    \"artifact_type\": \"tarball\"\n  }}\n}}\n"
            ));
            let manifest_listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let manifest_base_url = format!("http://{}", manifest_listener.local_addr().unwrap());
            let manifest_name = format!(
                "manifest-cli-{}-{}.json",
                current_os_label(),
                current_arch_label()
            );
            spawn_http_server(
                manifest_listener,
                vec![
                    (
                        format!("/latest/download/{manifest_name}"),
                        "application/json",
                        manifest.as_bytes().to_vec(),
                    ),
                    (
                        format!("/download/v{TEST_VERSION}/{manifest_name}"),
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

    fn run_installer(
        home: &Path,
        shell: Option<&str>,
        release: &FakeRelease,
        args: &[&str],
        extra_env: &[(&str, &str)],
    ) -> Output {
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/install-cli.sh");
        let mut command = Command::new("env");
        command
            .arg("-i")
            .arg(format!("HOME={}", home.display()))
            .arg(format!(
                "CHAT2DB_CLI_MANIFEST_BASE={}",
                release.manifest_base_url
            ))
            .arg(format!("CHAT2DB_CLI_CDN_BASE={}", release.cdn_base_url))
            .arg("CHAT2DB_CLI_NO_COMPLETIONS=1")
            .arg(format!("PATH={}", test_path()));
        if let Some(value) = shell {
            command.arg(format!("SHELL={value}"));
        }
        for (name, value) in extra_env {
            command.arg(format!("{name}={value}"));
        }
        command.arg("sh").arg(script).args(args);
        command.output().unwrap()
    }

    fn fake_cli_tarball() -> Vec<u8> {
        let cli = b"#!/bin/sh\ncase \"$1\" in\n  version) echo \"chat2db test\" ;;\n  completions) exit 0 ;;\n  *) echo \"chat2db test\" ;;\nesac\n";
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut archive = tar::Builder::new(&mut encoder);
            for (name, contents, mode) in [
                ("LICENSE", b"Apache License fixture".as_slice(), 0o644),
                (
                    "THIRD_PARTY_LICENSES.txt",
                    b"Third-party license fixture".as_slice(),
                    0o644,
                ),
                ("chat2db", cli.as_slice(), 0o755),
            ] {
                let mut header = tar::Header::new_gnu();
                header.set_path(name).unwrap();
                header.set_size(contents.len() as u64);
                header.set_mode(mode);
                header.set_cksum();
                archive.append(&header, contents).unwrap();
            }
            archive.finish().unwrap();
        }
        encoder.finish().unwrap()
    }

    fn current_os_label() -> &'static str {
        match std::env::consts::OS {
            "macos" => "darwin",
            "linux" => "linux",
            other => panic!("unsupported test OS: {other}"),
        }
    }

    fn current_arch_label() -> &'static str {
        match std::env::consts::ARCH {
            "aarch64" => "arm64",
            "x86_64" => "x86_64",
            other => panic!("unsupported test arch: {other}"),
        }
    }

    fn test_path() -> String {
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin:/usr/sbin:/sbin".to_string())
    }

    fn stdout(output: &Output) -> String {
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
}

fn windows_install_script() -> String {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/install-cli.ps1");
    fs::read_to_string(script).unwrap()
}

fn write_response(stream: &mut impl Write, content_type: &str, body: &[u8]) {
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
}

fn spawn_http_server(listener: TcpListener, routes: Vec<(String, &'static str, Vec<u8>)>) {
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
            if let Some((_, content_type, body)) = routes.iter().find(|(route, _, _)| route == path)
            {
                write_response(&mut stream, content_type, body);
            } else {
                write_not_found(&mut stream);
            }
        }
    });
}

fn write_not_found(stream: &mut impl Write) {
    let body = b"not found";
    let header = format!(
        "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "installer failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(windows)]
struct WindowsFakeRelease {
    manifest_base_url: String,
    cdn_base_url: String,
}

#[cfg(windows)]
impl WindowsFakeRelease {
    fn start(include_extra_entry: bool) -> Self {
        Self::start_with_manifest_transform(include_extra_entry, |manifest| manifest)
    }

    fn start_with_manifest_transform(
        include_extra_entry: bool,
        transform: fn(String) -> String,
    ) -> Self {
        let archive_bytes = fake_windows_cli_zip(include_extra_entry);
        let sha256 = format!("{:x}", Sha256::digest(&archive_bytes));
        let archive_name = format!("chat2db-{TEST_VERSION}-windows-x86_64.zip");

        let artifact_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let cdn_base_url = format!("http://{}", artifact_listener.local_addr().unwrap());
        let artifact_path =
            format!("/download/chat2db-cli/{TEST_VERSION}/windows/x86_64/{archive_name}");
        spawn_http_server(
            artifact_listener,
            vec![(artifact_path.clone(), "application/zip", archive_bytes)],
        );

        let manifest = transform(format!(
            "{{\n  \"version\": \"{TEST_VERSION}\",\n  \"artifact\": {{\n    \"url\": \"{cdn_base_url}{artifact_path}\",\n    \"sha256\": \"{sha256}\",\n    \"artifact_type\": \"zip\"\n  }}\n}}\n"
        ));
        let manifest_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let manifest_base_url = format!("http://{}", manifest_listener.local_addr().unwrap());
        spawn_http_server(
            manifest_listener,
            vec![(
                format!("/download/v{TEST_VERSION}/manifest-cli-windows-x86_64.json"),
                "application/json",
                manifest.into_bytes(),
            )],
        );

        Self {
            manifest_base_url,
            cdn_base_url,
        }
    }
}

#[cfg(windows)]
#[test]
fn powershell_installer_downloads_from_independent_local_services() {
    let temp = tempfile::tempdir().unwrap();
    let bin_dir = temp.path().join("bin");
    let release = WindowsFakeRelease::start(false);

    let output = run_powershell_installer(&bin_dir, &release);

    assert_success(&output);
    assert!(bin_dir.join("chat2db.exe").is_file());
}

#[cfg(windows)]
#[test]
fn powershell_installer_supports_wow64_on_64_bit_windows() {
    let temp = tempfile::tempdir().unwrap();
    let bin_dir = temp.path().join("bin");
    let release = WindowsFakeRelease::start(false);

    let output = run_powershell_installer_with_arch(&bin_dir, &release, "x86", Some("AMD64"));

    assert_success(&output);
    assert!(bin_dir.join("chat2db.exe").is_file());
}

#[cfg(windows)]
#[test]
fn powershell_installer_rejects_true_32_bit_windows() {
    let temp = tempfile::tempdir().unwrap();
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/install-cli.ps1");
    let output = Command::new("powershell.exe")
        .args(["-NoLogo", "-NoProfile", "-NonInteractive"])
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-File")
        .arg(script)
        .arg("-BinDir")
        .arg(temp.path())
        .arg("-NoModifyPath")
        .env("PROCESSOR_ARCHITECTURE", "x86")
        .env_remove("PROCESSOR_ARCHITEW6432")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let errors = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(errors.contains("32-bit Windows is not supported"));
}

#[cfg(windows)]
#[test]
fn powershell_installer_rejects_zip_with_additional_entries() {
    let temp = tempfile::tempdir().unwrap();
    let bin_dir = temp.path().join("bin");
    let release = WindowsFakeRelease::start(true);

    let output = run_powershell_installer(&bin_dir, &release);

    assert!(!output.status.success());
    assert!(!bin_dir.join("chat2db.exe").exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("must contain only chat2db.exe"));
}

#[cfg(windows)]
#[test]
fn powershell_installer_rejects_noncanonical_artifact_url() {
    let temp = tempfile::tempdir().unwrap();
    let bin_dir = temp.path().join("bin");
    let release = WindowsFakeRelease::start_with_manifest_transform(false, |manifest| {
        manifest.replace(".zip\"", ".zip?download=1\"")
    });

    let output = run_powershell_installer(&bin_dir, &release);

    assert!(!output.status.success());
    assert!(!bin_dir.join("chat2db.exe").exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("artifact URL"));
}

#[cfg(windows)]
fn run_powershell_installer(bin_dir: &Path, release: &WindowsFakeRelease) -> Output {
    run_powershell_installer_with_arch(bin_dir, release, "AMD64", None)
}

#[cfg(windows)]
fn run_powershell_installer_with_arch(
    bin_dir: &Path,
    release: &WindowsFakeRelease,
    process_architecture: &str,
    native_architecture: Option<&str>,
) -> Output {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/install-cli.ps1");
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoLogo", "-NoProfile", "-NonInteractive"])
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-File")
        .arg(script)
        .arg("-Version")
        .arg(TEST_VERSION)
        .arg("-BinDir")
        .arg(bin_dir)
        .arg("-NoModifyPath")
        .env("PROCESSOR_ARCHITECTURE", process_architecture)
        .env_remove("PROCESSOR_ARCHITEW6432")
        .env("CHAT2DB_CLI_MANIFEST_BASE", &release.manifest_base_url)
        .env("CHAT2DB_CLI_CDN_BASE", &release.cdn_base_url);
    if let Some(value) = native_architecture {
        command.env("PROCESSOR_ARCHITEW6432", value);
    }
    command.output().unwrap()
}

#[cfg(windows)]
fn fake_windows_cli_zip(include_extra_entry: bool) -> Vec<u8> {
    let executable = fs::read(env!("CARGO_BIN_EXE_chat2db")).unwrap();
    let file = tempfile::tempfile().unwrap();
    let mut writer = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default();
    writer.start_file("LICENSE", options).unwrap();
    writer.write_all(b"Apache License fixture").unwrap();
    writer
        .start_file("THIRD_PARTY_LICENSES.txt", options)
        .unwrap();
    writer.write_all(b"Third-party license fixture").unwrap();
    writer.start_file("chat2db.exe", options).unwrap();
    writer.write_all(&executable).unwrap();
    if include_extra_entry {
        writer.start_file("extra.txt", options).unwrap();
        writer.write_all(b"unexpected").unwrap();
    }
    let mut file = writer.finish().unwrap();
    file.rewind().unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    bytes
}
