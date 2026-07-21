//! Zip-slip-safe extraction used by the skill installer (and any future
//! release-binary path that ships zips).

use std::fs::{self, File};
use std::io;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use zip::ZipArchive;

use crate::error::{Chat2DbError, Result};

/// Extract a zip archive to `dest_dir`, refusing zip-slip and symlink entries.
/// Skips `.DS_Store` and `__MACOSX/` cruft.
pub fn safe_extract_zip(zip_path: &Path, dest_dir: &Path) -> Result<()> {
    let file = File::open(zip_path)?;
    let mut archive = ZipArchive::new(file)?;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let entry_name = entry.name().to_string();
        if should_ignore_zip_entry(&entry_name) {
            continue;
        }

        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| Chat2DbError::UnsafeZipPath(entry_name.clone()))?
            .to_owned();
        let output_path = dest_dir.join(enclosed);

        if is_zip_symlink(&entry) {
            return Err(Chat2DbError::UnsafeZipPath(entry_name));
        }

        if entry.is_dir() {
            fs::create_dir_all(&output_path)?;
            continue;
        }
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut output = File::create(&output_path)?;
        io::copy(&mut entry, &mut output)?;

        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            fs::set_permissions(&output_path, fs::Permissions::from_mode(mode & 0o777))?;
        }
    }

    Ok(())
}

fn should_ignore_zip_entry(name: &str) -> bool {
    name.is_empty() || name == ".DS_Store" || name.starts_with("__MACOSX/")
}

fn is_zip_symlink(entry: &zip::read::ZipFile<'_>) -> bool {
    entry
        .unix_mode()
        .map(|mode| (mode & 0o170000) == 0o120000)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn build_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default();
        for (name, contents) in entries {
            if name.ends_with('/') {
                writer.add_directory(*name, opts).unwrap();
            } else {
                writer.start_file(*name, opts).unwrap();
                writer.write_all(contents).unwrap();
            }
        }
        writer.finish().unwrap();
    }

    #[test]
    fn extracts_a_simple_zip_into_dest() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("bundle.zip");
        build_zip(
            &zip_path,
            &[
                ("chat2db-cli/", b"" as &[u8]),
                ("chat2db-cli/SKILL.md", b"---\nname: chat2db-cli\n---\n"),
                ("chat2db-cli/references/", b""),
                ("chat2db-cli/references/commands.md", b"# commands\n"),
            ],
        );

        let dest = dir.path().join("dest");
        fs::create_dir_all(&dest).unwrap();
        safe_extract_zip(&zip_path, &dest).unwrap();

        assert!(dest.join("chat2db-cli/SKILL.md").is_file());
        assert!(dest.join("chat2db-cli/references/commands.md").is_file());
    }

    #[test]
    fn rejects_relative_and_dangling_symlinks() {
        for (name, target) in [
            ("relative.zip", "references/commands.md"),
            ("dangling.zip", "missing"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let zip_path = dir.path().join(name);
            let file = File::create(&zip_path).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            writer
                .add_symlink("chat2db-cli", target, SimpleFileOptions::default())
                .unwrap();
            writer.finish().unwrap();

            let dest = dir.path().join("dest");
            fs::create_dir(&dest).unwrap();
            let error = safe_extract_zip(&zip_path, &dest).unwrap_err();
            assert!(matches!(error, Chat2DbError::UnsafeZipPath(_)));
            assert!(!dest.join("chat2db-cli").exists());
        }
    }
}
