use std::fs;
use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::error::{Chat2DbError, Result};

const MAX_INFO_PLIST_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppBundleMetadata {
    pub bundle_id: Option<String>,
    pub version: Option<String>,
    pub executable: Option<String>,
}

pub fn read_app_bundle_metadata(path: &Path) -> Result<AppBundleMetadata> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_INFO_PLIST_BYTES {
        return Err(plist_error(format!(
            "{} exceeds the {} byte limit",
            path.display(),
            MAX_INFO_PLIST_BYTES
        )));
    }
    let bytes = fs::read(path)?;
    parse_app_bundle_metadata(&bytes)
}

fn parse_app_bundle_metadata(bytes: &[u8]) -> Result<AppBundleMetadata> {
    if bytes.windows(b"<!ENTITY".len()).any(|window| {
        window
            .iter()
            .zip(b"<!ENTITY")
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
    }) {
        return Err(plist_error("entity declarations are not allowed"));
    }

    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut dict_depth = 0_u32;
    let mut capture: Option<&'static str> = None;
    let mut captured = String::new();
    let mut pending_key: Option<String> = None;
    let mut result = AppBundleMetadata::default();

    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => match event.name().as_ref() {
                b"dict" => {
                    if dict_depth == 1 {
                        pending_key = None;
                    }
                    dict_depth = dict_depth.saturating_add(1);
                }
                b"key" if dict_depth == 1 => {
                    capture = Some("key");
                    captured.clear();
                }
                b"string" if dict_depth == 1 => {
                    capture = Some("string");
                    captured.clear();
                }
                _ if dict_depth == 1 && capture.is_none() => pending_key = None,
                _ => {}
            },
            Ok(Event::Text(text)) if capture.is_some() => {
                let decoded = text
                    .decode()
                    .map_err(|error| plist_error(error.to_string()))?;
                let unescaped = quick_xml::escape::unescape(&decoded)
                    .map_err(|error| plist_error(error.to_string()))?;
                captured.push_str(&unescaped);
            }
            Ok(Event::End(event)) => match event.name().as_ref() {
                b"key" if capture == Some("key") => {
                    pending_key = Some(std::mem::take(&mut captured));
                    capture = None;
                }
                b"string" if capture == Some("string") => {
                    if let Some(key) = pending_key.take() {
                        set_metadata_field(&mut result, &key, std::mem::take(&mut captured))?;
                    }
                    capture = None;
                }
                b"dict" => dict_depth = dict_depth.saturating_sub(1),
                _ => {}
            },
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => return Err(plist_error(error.to_string())),
        }
        buffer.clear();
    }

    Ok(result)
}

fn set_metadata_field(result: &mut AppBundleMetadata, key: &str, value: String) -> Result<()> {
    let field = match key {
        "CFBundleIdentifier" => &mut result.bundle_id,
        "CFBundleShortVersionString" => &mut result.version,
        "CFBundleExecutable" => &mut result.executable,
        _ => return Ok(()),
    };
    if field.replace(value).is_some() {
        return Err(plist_error(format!("duplicate {key}")));
    }
    Ok(())
}

fn plist_error(message: impl Into<String>) -> Chat2DbError {
    Chat2DbError::Plist(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_allowlisted_bundle_fields() {
        let plist = br#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.chat2db.community</string>
<key>CFBundleShortVersionString</key><string>5.3.0</string>
<key>CFBundleExecutable</key><string>Chat2DB Community</string>
<key>Ignored</key><string>value</string>
</dict></plist>"#;

        assert_eq!(
            parse_app_bundle_metadata(plist).unwrap(),
            AppBundleMetadata {
                bundle_id: Some("com.chat2db.community".to_string()),
                version: Some("5.3.0".to_string()),
                executable: Some("Chat2DB Community".to_string()),
            }
        );
    }

    #[test]
    fn rejects_entity_declarations() {
        let error = parse_app_bundle_metadata(
            br#"<!DOCTYPE plist [<!ENTITY payload "x">]><plist><dict/></plist>"#,
        )
        .unwrap_err();

        assert!(matches!(error, Chat2DbError::Plist(_)));
    }

    #[test]
    fn rejects_duplicate_security_fields() {
        let error = parse_app_bundle_metadata(
            br#"<plist><dict><key>CFBundleIdentifier</key><string>first</string><key>CFBundleIdentifier</key><string>second</string></dict></plist>"#,
        )
        .unwrap_err();

        assert!(matches!(error, Chat2DbError::Plist(_)));
    }
}
