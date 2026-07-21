use std::fs;
use std::path::{Path, PathBuf};

use rusty_leveldb::{DB, Options};
use serde_json::{Map, Value, json};

use crate::edition::Edition;
use crate::error::{Chat2DbError, Result};

const GLOBAL_STORE_KEY: &[u8] = b"_file://\0\x01Chat2DB_Global_Store";
const UTF16_STORAGE_PREFIX: u8 = 0;

#[derive(Debug, Clone)]
pub struct FrontendStoreDefaults {
    pub appearance: String,
    pub language: String,
}

impl Default for FrontendStoreDefaults {
    fn default() -> Self {
        Self {
            appearance: "dark".to_string(),
            language: "zh-CN".to_string(),
        }
    }
}

pub fn local_storage_leveldb_path(edition: Edition, home_dir: &Path) -> PathBuf {
    edition.frontend_leveldb_path(home_dir)
}

pub fn sync_mcp_enabled(
    edition: Edition,
    home_dir: &Path,
    enabled: bool,
    defaults: FrontendStoreDefaults,
) -> Result<()> {
    let db_path = local_storage_leveldb_path(edition, home_dir);
    sync_mcp_enabled_at(&db_path, enabled, defaults)
}

#[cfg(test)]
pub(crate) fn read_mcp_enabled_for_test(edition: Edition, home_dir: &Path) -> Result<Option<bool>> {
    let db_path = local_storage_leveldb_path(edition, home_dir);
    if !db_path.exists() {
        return Ok(None);
    }

    let options = Options {
        create_if_missing: false,
        ..Default::default()
    };
    let mut db =
        DB::open(&db_path, options).map_err(|source| frontend_store_error(&db_path, source))?;
    let Some(raw) = db.get(GLOBAL_STORE_KEY) else {
        db.close()
            .map_err(|source| frontend_store_error(&db_path, source))?;
        return Ok(None);
    };
    db.close()
        .map_err(|source| frontend_store_error(&db_path, source))?;

    let (_, json_text) = decode_storage_value(&db_path, &raw)?;
    let value: Value = serde_json::from_str(&json_text).map_err(|source| {
        Chat2DbError::McpFrontendStoreInvalidJson {
            path: db_path,
            source,
        }
    })?;
    Ok(value
        .get("state")
        .and_then(|state| state.get("baseSetting"))
        .and_then(|base_setting| base_setting.get("enableMcp"))
        .and_then(Value::as_bool))
}

fn sync_mcp_enabled_at(
    db_path: &Path,
    enabled: bool,
    defaults: FrontendStoreDefaults,
) -> Result<()> {
    fs::create_dir_all(db_path)?;

    let options = Options {
        create_if_missing: true,
        ..Default::default()
    };

    let mut db =
        DB::open(db_path, options).map_err(|source| frontend_store_error(db_path, source))?;

    let (mut value, encoding) = match db.get(GLOBAL_STORE_KEY) {
        Some(raw) => {
            let (encoding, json_text) = decode_storage_value(db_path, &raw)?;
            let value = serde_json::from_str(&json_text).map_err(|source| {
                Chat2DbError::McpFrontendStoreInvalidJson {
                    path: db_path.to_path_buf(),
                    source,
                }
            })?;
            (value, encoding)
        }
        None => (
            default_global_store(defaults.clone()),
            StorageEncoding::Utf16,
        ),
    };

    apply_mcp_enabled(&mut value, enabled, defaults);

    let json_text = serde_json::to_string(&value).map_err(|source| {
        Chat2DbError::McpFrontendStoreInvalidJson {
            path: db_path.to_path_buf(),
            source,
        }
    })?;
    let raw = encode_storage_value(&json_text, encoding);
    db.put(GLOBAL_STORE_KEY, &raw)
        .map_err(|source| frontend_store_error(db_path, source))?;
    db.close()
        .map_err(|source| frontend_store_error(db_path, source))?;
    Ok(())
}

fn apply_mcp_enabled(value: &mut Value, enabled: bool, defaults: FrontendStoreDefaults) {
    if !value.is_object() {
        *value = default_global_store(defaults.clone());
    }

    let root = value.as_object_mut().expect("value is object");
    root.entry("version".to_string()).or_insert(json!(0));

    let state = root
        .entry("state".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !state.is_object() {
        *state = Value::Object(Map::new());
    }

    let state = state.as_object_mut().expect("state is object");
    let base_setting = state
        .entry("baseSetting".to_string())
        .or_insert_with(|| default_base_setting(&defaults));
    if !base_setting.is_object() {
        *base_setting = default_base_setting(&defaults);
    }

    let base_setting = base_setting.as_object_mut().expect("baseSetting is object");
    base_setting
        .entry("appearance".to_string())
        .or_insert_with(|| Value::String(defaults.appearance));
    base_setting
        .entry("language".to_string())
        .or_insert_with(|| Value::String(defaults.language));
    base_setting
        .entry("customFont".to_string())
        .or_insert_with(|| Value::String(String::new()));
    base_setting
        .entry("customFontSize".to_string())
        .or_insert_with(|| json!(13));
    base_setting.insert("enableMcp".to_string(), Value::Bool(enabled));
}

fn default_global_store(defaults: FrontendStoreDefaults) -> Value {
    json!({
        "state": {
            "baseSetting": default_base_setting(&defaults)
        },
        "version": 0
    })
}

fn default_base_setting(defaults: &FrontendStoreDefaults) -> Value {
    json!({
        "appearance": defaults.appearance,
        "language": defaults.language,
        "customFont": "",
        "customFontSize": 13,
        "enableMcp": false
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StorageEncoding {
    Utf16,
    Utf8WithPrefix,
    Utf8Plain,
}

fn decode_storage_value(db_path: &Path, raw: &[u8]) -> Result<(StorageEncoding, String)> {
    if raw.is_empty() {
        return Ok((StorageEncoding::Utf16, String::new()));
    }

    match raw[0] {
        UTF16_STORAGE_PREFIX => {
            let text = decode_utf16_le(db_path, &raw[1..])?;
            Ok((StorageEncoding::Utf16, text))
        }
        1 => {
            let text = String::from_utf8(raw[1..].to_vec())
                .map_err(|source| frontend_store_error(db_path, source))?;
            Ok((StorageEncoding::Utf8WithPrefix, text))
        }
        _ => {
            let text = String::from_utf8(raw.to_vec())
                .map_err(|source| frontend_store_error(db_path, source))?;
            Ok((StorageEncoding::Utf8Plain, text))
        }
    }
}

fn decode_utf16_le(db_path: &Path, raw: &[u8]) -> Result<String> {
    if !raw.len().is_multiple_of(2) {
        return Err(Chat2DbError::McpFrontendStore {
            path: db_path.to_path_buf(),
            message: "UTF-16 localStorage value has an odd byte length".to_string(),
        });
    }

    let code_units = raw
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    String::from_utf16(&code_units).map_err(|source| frontend_store_error(db_path, source))
}

fn encode_storage_value(text: &str, encoding: StorageEncoding) -> Vec<u8> {
    match encoding {
        StorageEncoding::Utf16 => {
            let mut raw = vec![UTF16_STORAGE_PREFIX];
            for code_unit in text.encode_utf16() {
                raw.extend_from_slice(&code_unit.to_le_bytes());
            }
            raw
        }
        StorageEncoding::Utf8WithPrefix => {
            let mut raw = vec![1];
            raw.extend_from_slice(text.as_bytes());
            raw
        }
        StorageEncoding::Utf8Plain => text.as_bytes().to_vec(),
    }
}

fn frontend_store_error<E: std::fmt::Display>(path: &Path, source: E) -> Chat2DbError {
    Chat2DbError::McpFrontendStore {
        path: path.to_path_buf(),
        message: source.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_existing_global_store_preserves_other_fields() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("leveldb");
        seed_global_store(
            &db_path,
            json!({
                "state": {
                    "mainPageActiveTab": "workspace",
                    "baseSetting": {
                        "appearance": "light",
                        "language": "en-US",
                        "customFontSize": 15,
                        "enableMcp": false
                    },
                    "shortcutConfig": {
                        "openSetting": { "key": "openSetting" }
                    }
                },
                "version": 0
            }),
            StorageEncoding::Utf16,
        );

        sync_mcp_enabled_at(
            &db_path,
            true,
            FrontendStoreDefaults {
                appearance: "dark".to_string(),
                language: "zh-CN".to_string(),
            },
        )
        .unwrap();

        let (encoding, value) = read_global_store(&db_path);
        assert_eq!(encoding, StorageEncoding::Utf16);
        assert_eq!(value["state"]["baseSetting"]["enableMcp"], true);
        assert_eq!(value["state"]["baseSetting"]["appearance"], "light");
        assert_eq!(value["state"]["baseSetting"]["language"], "en-US");
        assert_eq!(value["state"]["baseSetting"]["customFontSize"], 15);
        assert_eq!(
            value["state"]["shortcutConfig"]["openSetting"]["key"],
            "openSetting"
        );
    }

    #[test]
    fn sync_creates_store_when_missing() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("leveldb");

        sync_mcp_enabled_at(&db_path, true, FrontendStoreDefaults::default()).unwrap();

        let (encoding, value) = read_global_store(&db_path);
        assert_eq!(encoding, StorageEncoding::Utf16);
        assert_eq!(value["state"]["baseSetting"]["enableMcp"], true);
        assert_eq!(value["state"]["baseSetting"]["appearance"], "dark");
        assert_eq!(value["state"]["baseSetting"]["language"], "zh-CN");
    }

    #[test]
    fn sync_preserves_utf8_storage_encoding() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("leveldb");
        seed_global_store(
            &db_path,
            json!({
                "state": {
                    "baseSetting": {
                        "enableMcp": true
                    }
                },
                "version": 0
            }),
            StorageEncoding::Utf8Plain,
        );

        sync_mcp_enabled_at(&db_path, false, FrontendStoreDefaults::default()).unwrap();

        let (encoding, value) = read_global_store(&db_path);
        assert_eq!(encoding, StorageEncoding::Utf8Plain);
        assert_eq!(value["state"]["baseSetting"]["enableMcp"], false);
    }

    #[test]
    fn invalid_json_returns_stable_error() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("leveldb");
        seed_raw(&db_path, encode_storage_value("{", StorageEncoding::Utf16));

        let error =
            sync_mcp_enabled_at(&db_path, true, FrontendStoreDefaults::default()).unwrap_err();

        assert!(matches!(
            error,
            Chat2DbError::McpFrontendStoreInvalidJson { .. }
        ));
        assert_eq!(error.code(), "mcp_frontend_store_invalid_json");
    }

    fn seed_global_store(db_path: &Path, value: Value, encoding: StorageEncoding) {
        seed_raw(
            db_path,
            encode_storage_value(&serde_json::to_string(&value).unwrap(), encoding),
        );
    }

    fn seed_raw(db_path: &Path, raw: Vec<u8>) {
        fs::create_dir_all(db_path).unwrap();
        let mut db = DB::open(db_path, Options::default()).unwrap();
        db.put(GLOBAL_STORE_KEY, &raw).unwrap();
        db.close().unwrap();
    }

    fn read_global_store(db_path: &Path) -> (StorageEncoding, Value) {
        let options = Options {
            create_if_missing: false,
            ..Default::default()
        };
        let mut db = DB::open(db_path, options).unwrap();
        let raw = db.get(GLOBAL_STORE_KEY).unwrap();
        db.close().unwrap();
        let (encoding, text) = decode_storage_value(db_path, &raw).unwrap();
        (encoding, serde_json::from_str(&text).unwrap())
    }
}
