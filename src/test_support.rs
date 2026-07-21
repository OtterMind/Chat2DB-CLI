use std::fs;
use std::path::{Path, PathBuf};

use crate::edition::Edition;
#[cfg(windows)]
use crate::error::Chat2DbError;
use crate::error::Result;
use crate::installer::SystemPaths;
#[cfg(not(windows))]
use crate::installer::verify_app_bundle;
use crate::platform::current_os;

pub(crate) fn test_paths(root: &Path) -> SystemPaths {
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

pub(crate) fn create_installed_app(paths: &SystemPaths, edition: Edition) -> PathBuf {
    let app_path = edition.app_path_for_os(&paths.applications_dir, current_os());
    create_valid_app(&app_path, edition);
    app_path
}

pub(crate) fn create_invalid_app(paths: &SystemPaths, edition: Edition) -> PathBuf {
    let app_path = edition.app_path_for_os(&paths.applications_dir, current_os());
    create_invalid_app_at(&app_path);
    app_path
}

pub(crate) fn verify_app_fixture(app_path: &Path, edition: Edition) -> Result<()> {
    #[cfg(windows)]
    {
        if app_path.is_file() {
            Ok(())
        } else {
            Err(Chat2DbError::AppNotInstalled {
                edition: edition.to_string(),
                path: app_path.to_path_buf(),
            })
        }
    }

    #[cfg(not(windows))]
    {
        verify_app_bundle(app_path, edition)
    }
}

#[cfg(windows)]
fn create_valid_app(app_path: &Path, _edition: Edition) {
    fs::create_dir_all(app_path.parent().unwrap()).unwrap();
    fs::write(app_path, b"Chat2DB test executable").unwrap();
}

#[cfg(not(windows))]
fn create_valid_app(app_path: &Path, edition: Edition) {
    let contents = app_path.join("Contents");
    fs::create_dir_all(&contents).unwrap();
    fs::write(contents.join("Info.plist"), plist_xml(edition.bundle_id())).unwrap();
}

#[cfg(windows)]
fn create_invalid_app_at(app_path: &Path) {
    fs::create_dir_all(app_path).unwrap();
}

#[cfg(not(windows))]
fn create_invalid_app_at(app_path: &Path) {
    let contents = app_path.join("Contents");
    fs::create_dir_all(&contents).unwrap();
    fs::write(contents.join("Info.plist"), plist_xml("com.example.other")).unwrap();
}

#[cfg(not(windows))]
fn plist_xml(bundle_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>
  <string>{bundle_id}</string>
  <key>CFBundleShortVersionString</key>
  <string>1.2.3</string>
  <key>CFBundleExecutable</key>
  <string>Chat2DB</string>
</dict>
</plist>
"#
    )
}
