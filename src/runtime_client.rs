use std::fmt;
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::edition::Edition;
use crate::error::{Chat2DbError, Result};

const SUPPORTED_API_VERSIONS: &[&str] = &["v1"];
const RUNTIME_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const RUNTIME_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeRequestKind {
    Standard,
    Sql,
}

impl RuntimeRequestKind {
    fn timeout(self) -> Option<Duration> {
        match self {
            Self::Standard => Some(RUNTIME_REQUEST_TIMEOUT),
            Self::Sql => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeCapability {
    DatasourceList,
    DatasourceGet,
    DatasourceCreate,
    DatasourceUpdate,
    DatasourceDelete,
    MetadataDatabaseList,
    MetadataSchemaList,
    MetadataTableList,
    MetadataTableDetail,
    SqlQuery,
}

impl RuntimeCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DatasourceList => "datasource.list",
            Self::DatasourceGet => "datasource.get",
            Self::DatasourceCreate => "datasource.create",
            Self::DatasourceUpdate => "datasource.update",
            Self::DatasourceDelete => "datasource.delete",
            Self::MetadataDatabaseList => "metadata.database.list",
            Self::MetadataSchemaList => "metadata.schema.list",
            Self::MetadataTableList => "metadata.table.list",
            Self::MetadataTableDetail => "metadata.table.detail",
            Self::SqlQuery => "sql.query",
        }
    }
}

#[derive(Clone)]
pub struct RuntimeClient {
    base_url: String,
    token: String,
    client: Client,
    sql_client: Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeHealth {
    pub ready: bool,
    pub edition: Option<String>,
    pub version: Option<String>,
    #[serde(rename = "apiVersion")]
    pub api_version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeCapabilities {
    pub features: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeEnvelope {
    success: Option<bool>,
    data: Option<Value>,
    error: Option<RuntimeError>,
    #[serde(rename = "errorCode")]
    error_code: Option<String>,
    #[serde(rename = "errorMessage")]
    error_message: Option<String>,
    #[serde(rename = "errorDetail")]
    error_detail: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeError {
    code: Option<String>,
    message: Option<String>,
    #[serde(default)]
    details: Value,
}

fn build_client(kind: RuntimeRequestKind) -> Client {
    let builder = Client::builder()
        .connect_timeout(RUNTIME_CONNECT_TIMEOUT)
        .user_agent(concat!("chat2db-cli/", env!("CARGO_PKG_VERSION")));
    let builder = match kind.timeout() {
        Some(timeout) => builder.timeout(timeout),
        None => builder,
    };
    builder
        .build()
        .expect("reqwest client builder should not fail with static config")
}

impl RuntimeClient {
    pub fn new(base_url: String, token: String) -> Self {
        let client = build_client(RuntimeRequestKind::Standard);
        let sql_client = build_client(RuntimeRequestKind::Sql);
        Self {
            base_url,
            token,
            client,
            sql_client,
        }
    }

    pub fn health(&self) -> Result<RuntimeHealth> {
        let value = self.get("/runtime/health")?;
        serde_json::from_value(value).map_err(Chat2DbError::ManifestJson)
    }

    pub fn capabilities(&self) -> Result<RuntimeCapabilities> {
        let value = self.get("/runtime/capabilities")?;
        serde_json::from_value(value).map_err(|source| {
            runtime_contract_error(
                "runtime_capabilities_invalid",
                format!("runtime capabilities response is invalid: {source}"),
                json!({ "reason": source.to_string() }),
            )
        })
    }

    pub fn verify_contract(
        &self,
        expected_edition: Edition,
        required_capability: RuntimeCapability,
    ) -> Result<()> {
        let health = self.health()?;
        validate_runtime_health(&health, expected_edition)?;
        let capabilities = self.capabilities()?;
        validate_runtime_capability(&capabilities, required_capability)
    }

    pub fn shutdown(&self) -> Result<Value> {
        self.post("/runtime/shutdown", json!({}))
    }

    pub fn datasources(&self, page_no: u32, page_size: u32, refresh: bool) -> Result<Value> {
        self.post(
            "/datasources/list",
            json!({
                "pageNo": page_no,
                "pageSize": page_size,
                "refresh": refresh
            }),
        )
    }

    pub fn datasource_get(&self, data_source_id: u64) -> Result<Value> {
        self.post(
            "/datasources/get",
            json!({
                "dataSourceId": data_source_id
            }),
        )
    }

    pub fn datasource_create(&self, request: DatasourceCreateRequest) -> Result<Value> {
        self.post("/datasources/create", request.into_body())
    }

    pub fn datasource_update(&self, request: DatasourceUpdateRequest) -> Result<Value> {
        self.post("/datasources/update", request.into_body())
    }

    pub fn datasource_delete(&self, data_source_id: u64) -> Result<Value> {
        self.post(
            "/datasources/delete",
            json!({
                "dataSourceId": data_source_id
            }),
        )
    }

    pub fn datasource_connection_test(
        &self,
        request: DatasourceConnectionTestRequest,
    ) -> Result<Value> {
        self.post("/datasources/connection-test", request.into_body())
    }

    pub fn databases(&self, data_source_id: u64, refresh: bool) -> Result<Value> {
        self.post(
            "/metadata/databases/list",
            json!({
                "dataSourceId": data_source_id,
                "refresh": refresh
            }),
        )
    }

    pub fn schemas(
        &self,
        data_source_id: u64,
        database_name: String,
        refresh: bool,
    ) -> Result<Value> {
        self.post(
            "/metadata/schemas/list",
            json!({
                "dataSourceId": data_source_id,
                "databaseName": database_name,
                "refresh": refresh
            }),
        )
    }

    pub fn tables(
        &self,
        data_source_id: u64,
        database_name: String,
        schema_name: Option<String>,
        page_no: u32,
        page_size: u32,
        refresh: bool,
    ) -> Result<Value> {
        let mut body = json!({
            "dataSourceId": data_source_id,
            "databaseName": database_name,
            "pageNo": page_no,
            "pageSize": page_size,
            "refresh": refresh
        });
        if let Some(schema_name) = schema_name {
            body["schemaName"] = Value::String(schema_name);
        }
        self.post("/metadata/tables/list", body)
    }

    pub fn table_detail(
        &self,
        data_source_id: u64,
        database_name: String,
        schema_name: Option<String>,
        table_name: String,
    ) -> Result<Value> {
        let mut body = json!({
            "dataSourceId": data_source_id,
            "databaseName": database_name,
            "tableName": table_name
        });
        if let Some(schema_name) = schema_name {
            body["schemaName"] = Value::String(schema_name);
        }
        self.post("/metadata/tables/detail", body)
    }

    pub fn sql_query(&self, request: SqlQueryRequest) -> Result<Value> {
        let mut body = json!({
            "dataSourceId": request.data_source_id,
            "databaseName": request.database_name,
            "sql": request.sql,
            "pageNo": request.page_no,
            "pageSize": request.page_size,
            "includeRowNumber": request.include_row_number
        });
        if let Some(schema_name) = request.schema_name {
            body["schemaName"] = Value::String(schema_name);
        }
        if let Some(result_set_id) = request.result_set_id {
            body["resultSetId"] = json!(result_set_id);
        }
        self.post_with(&self.sql_client, "/sql/query", body)
    }

    fn get(&self, path: &str) -> Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let response = self
            .client
            .get(&url)
            .bearer_auth(&self.token)
            .header("X-Chat2DB-CLI-Version", env!("CARGO_PKG_VERSION"))
            .send()
            .map_err(|source| Chat2DbError::RuntimeClientHttp {
                url: url.clone(),
                message: source.to_string(),
            })?;
        self.parse_response(url, response)
    }

    fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.post_with(&self.client, path, body)
    }

    fn post_with(&self, client: &Client, path: &str, body: Value) -> Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let encoded = serde_json::to_vec(&body).map_err(Chat2DbError::ManifestJson)?;
        let response = client
            .post(&url)
            .bearer_auth(&self.token)
            .header("X-Chat2DB-CLI-Version", env!("CARGO_PKG_VERSION"))
            .header(CONTENT_TYPE, "application/json")
            .body(encoded)
            .send()
            .map_err(|source| Chat2DbError::RuntimeClientHttp {
                url: url.clone(),
                message: source.to_string(),
            })?;
        self.parse_response(url, response)
    }

    fn parse_response(&self, url: String, response: reqwest::blocking::Response) -> Result<Value> {
        let status = response.status();
        let body = response
            .text()
            .map_err(|source| Chat2DbError::RuntimeClientHttp {
                url: url.clone(),
                message: source.to_string(),
            })?;
        let envelope: RuntimeEnvelope =
            serde_json::from_str(&body).map_err(|source| Chat2DbError::RuntimeClientHttp {
                url: url.clone(),
                message: format!("failed to parse runtime response JSON: {source}; body: {body}"),
            })?;
        if !status.is_success() && envelope.success != Some(false) {
            return Err(Chat2DbError::RuntimeClientHttp {
                url,
                message: format!("HTTP {status}: {body}"),
            });
        }
        if envelope.success.unwrap_or(false) {
            Ok(envelope.data.unwrap_or(Value::Null))
        } else {
            let (code, message, details) = runtime_error_from_envelope(envelope).into_parts();
            Err(Chat2DbError::RuntimeApi {
                code,
                message,
                details,
            })
        }
    }
}

pub(crate) fn validate_runtime_health(
    health: &RuntimeHealth,
    expected_edition: Edition,
) -> Result<()> {
    if !health.ready {
        return Err(runtime_contract_error(
            "runtime_not_ready",
            "runtime reported that it is not ready".to_string(),
            json!({ "ready": health.ready }),
        ));
    }

    let expected = expected_edition.as_str();
    if health.edition.as_deref() != Some(expected) {
        return Err(runtime_contract_error(
            "runtime_edition_mismatch",
            format!(
                "runtime edition mismatch: expected {expected}, got {}",
                health.edition.as_deref().unwrap_or("<missing>")
            ),
            json!({
                "expectedEdition": expected,
                "actualEdition": health.edition,
            }),
        ));
    }

    let api_version = health.api_version.as_deref().unwrap_or("<missing>");
    if !SUPPORTED_API_VERSIONS.contains(&api_version) {
        return Err(runtime_contract_error(
            "runtime_api_version_unsupported",
            format!("runtime API version {api_version} is not supported"),
            json!({
                "actualApiVersion": health.api_version,
                "supportedApiVersions": SUPPORTED_API_VERSIONS,
            }),
        ));
    }

    Ok(())
}

fn validate_runtime_capability(
    capabilities: &RuntimeCapabilities,
    required: RuntimeCapability,
) -> Result<()> {
    let required = required.as_str();
    if capabilities
        .features
        .iter()
        .any(|feature| feature == required)
    {
        return Ok(());
    }

    Err(runtime_contract_error(
        "runtime_capability_missing",
        format!("runtime does not support required capability {required}"),
        json!({
            "requiredCapability": required,
            "availableCapabilities": capabilities.features,
        }),
    ))
}

fn runtime_contract_error(code: &str, message: String, details: Value) -> Chat2DbError {
    Chat2DbError::RuntimeApi {
        code: code.to_string(),
        message,
        details,
    }
}

impl fmt::Debug for RuntimeClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeClient")
            .field("base_url", &self.base_url)
            .field("token", &"<redacted>")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct SqlQueryRequest {
    pub data_source_id: u64,
    pub database_name: String,
    pub schema_name: Option<String>,
    pub sql: String,
    pub page_no: u32,
    pub page_size: u32,
    pub result_set_id: Option<u32>,
    pub include_row_number: bool,
}

#[derive(Clone)]
pub struct DatasourceCreateRequest {
    pub alias: Option<String>,
    pub db_type: String,
    pub url: Option<String>,
    pub user: Option<String>,
    pub password: String,
    pub environment_id: u64,
    pub host: Option<String>,
    pub port: Option<String>,
    pub database: Option<String>,
    pub service_name: Option<String>,
    pub service_type: Option<String>,
    pub storage_type: Option<String>,
    pub space_id: Option<u64>,
}

impl fmt::Debug for DatasourceCreateRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatasourceCreateRequest")
            .field("alias", &self.alias)
            .field("db_type", &self.db_type)
            .field("url", &self.url)
            .field("user", &self.user)
            .field("password", &"<redacted>")
            .field("environment_id", &self.environment_id)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("service_name", &self.service_name)
            .field("service_type", &self.service_type)
            .field("storage_type", &self.storage_type)
            .field("space_id", &self.space_id)
            .finish()
    }
}

#[derive(Clone)]
pub struct DatasourceUpdateRequest {
    pub data_source_id: u64,
    pub alias: Option<String>,
    pub db_type: Option<String>,
    pub url: Option<String>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub environment_id: Option<u64>,
    pub host: Option<String>,
    pub port: Option<String>,
    pub database: Option<String>,
    pub service_name: Option<String>,
    pub service_type: Option<String>,
    pub storage_type: Option<String>,
    pub space_id: Option<u64>,
}

impl fmt::Debug for DatasourceUpdateRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatasourceUpdateRequest")
            .field("data_source_id", &self.data_source_id)
            .field("alias", &self.alias)
            .field("db_type", &self.db_type)
            .field("url", &self.url)
            .field("user", &self.user)
            .field("password", &"<redacted>")
            .field("environment_id", &self.environment_id)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("service_name", &self.service_name)
            .field("service_type", &self.service_type)
            .field("storage_type", &self.storage_type)
            .field("space_id", &self.space_id)
            .finish()
    }
}

#[derive(Clone)]
pub struct DatasourceConnectionTestRequest {
    pub data_source_id: Option<u64>,
    pub db_type: Option<String>,
    pub url: Option<String>,
    pub host: Option<String>,
    pub port: Option<String>,
    pub database: Option<String>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub service_name: Option<String>,
    pub service_type: Option<String>,
}

impl fmt::Debug for DatasourceConnectionTestRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatasourceConnectionTestRequest")
            .field("data_source_id", &self.data_source_id)
            .field("db_type", &self.db_type)
            .field("url", &self.url)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("user", &self.user)
            .field("password", &"<redacted>")
            .field("service_name", &self.service_name)
            .field("service_type", &self.service_type)
            .finish()
    }
}

impl DatasourceCreateRequest {
    fn into_body(self) -> Value {
        let mut body = Map::new();
        insert_string(&mut body, "alias", self.alias);
        body.insert("dbType".to_string(), Value::String(self.db_type));
        insert_string(&mut body, "url", self.url);
        insert_string(&mut body, "user", self.user);
        body.insert("password".to_string(), Value::String(self.password));
        body.insert("environmentId".to_string(), json!(self.environment_id));
        insert_string(&mut body, "host", self.host);
        insert_string(&mut body, "port", self.port);
        insert_string(&mut body, "database", self.database);
        insert_string(&mut body, "serviceName", self.service_name);
        insert_string(&mut body, "serviceType", self.service_type);
        insert_string(&mut body, "storageType", self.storage_type);
        insert_number(&mut body, "spaceId", self.space_id);
        Value::Object(body)
    }
}

fn runtime_error_from_envelope(envelope: RuntimeEnvelope) -> RuntimeError {
    if let Some(error) = envelope.error {
        return RuntimeError {
            code: error.code,
            message: error.message,
            details: error.details,
        };
    }

    let code = envelope
        .error_code
        .unwrap_or_else(|| "runtime_api_error".to_string());
    let message = envelope
        .error_message
        .or(envelope.error_detail)
        .unwrap_or_else(|| "runtime API returned an error".to_string());
    RuntimeError {
        code: Some(code),
        message: Some(message),
        details: Value::Null,
    }
}

impl RuntimeError {
    fn into_parts(self) -> (String, String, Value) {
        let detail_message = self.details.get("message").and_then(Value::as_str);
        let code = self.code.unwrap_or_else(|| "runtime_api_error".to_string());
        let message = self
            .message
            .or_else(|| detail_message.map(ToString::to_string))
            .unwrap_or_else(|| "runtime API returned an error".to_string());
        (code, message, self.details)
    }
}

impl DatasourceConnectionTestRequest {
    fn into_body(self) -> Value {
        let mut body = Map::new();
        insert_number(&mut body, "dataSourceId", self.data_source_id);
        insert_string(&mut body, "dbType", self.db_type);
        insert_string(&mut body, "url", self.url);
        insert_string(&mut body, "host", self.host);
        insert_string(&mut body, "port", self.port);
        insert_string(&mut body, "database", self.database);
        insert_string(&mut body, "user", self.user);
        insert_string(&mut body, "password", self.password);
        insert_string(&mut body, "serviceName", self.service_name);
        insert_string(&mut body, "serviceType", self.service_type);
        Value::Object(body)
    }
}

impl DatasourceUpdateRequest {
    fn into_body(self) -> Value {
        let mut body = Map::new();
        body.insert("dataSourceId".to_string(), json!(self.data_source_id));
        insert_string(&mut body, "alias", self.alias);
        insert_string(&mut body, "dbType", self.db_type);
        insert_string(&mut body, "url", self.url);
        insert_string(&mut body, "user", self.user);
        insert_string(&mut body, "password", self.password);
        insert_number(&mut body, "environmentId", self.environment_id);
        insert_string(&mut body, "host", self.host);
        insert_string(&mut body, "port", self.port);
        insert_string(&mut body, "database", self.database);
        insert_string(&mut body, "serviceName", self.service_name);
        insert_string(&mut body, "serviceType", self.service_type);
        insert_string(&mut body, "storageType", self.storage_type);
        insert_number(&mut body, "spaceId", self.space_id);
        Value::Object(body)
    }
}

fn insert_string(body: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        body.insert(key.to_string(), Value::String(value));
    }
}

fn insert_number(body: &mut Map<String, Value>, key: &str, value: Option<u64>) {
    if let Some(value) = value {
        body.insert(key.to_string(), json!(value));
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    fn compatible_health(edition: &str) -> RuntimeHealth {
        RuntimeHealth {
            ready: true,
            edition: Some(edition.to_string()),
            version: Some("5.3.0".to_string()),
            api_version: Some("v1".to_string()),
        }
    }

    fn capabilities(features: &[&str]) -> RuntimeCapabilities {
        RuntimeCapabilities {
            features: features
                .iter()
                .map(|feature| (*feature).to_string())
                .collect(),
        }
    }

    fn runtime_api_code(error: Chat2DbError) -> String {
        match error {
            Chat2DbError::RuntimeApi { code, .. } => code,
            other => panic!("expected runtime API error, got {other}"),
        }
    }

    fn spawn_runtime_server(
        health: Value,
        capabilities: Value,
    ) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 2048];
                let bytes_read = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..bytes_read]);
                let data = if request.contains("GET /api/cli/v1/runtime/health ") {
                    &health
                } else if request.contains("GET /api/cli/v1/runtime/capabilities ") {
                    &capabilities
                } else {
                    panic!("unexpected runtime request: {request}");
                };
                let body = json!({ "success": true, "data": data }).to_string();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
                stream.flush().unwrap();
            }
        });
        (format!("http://{address}/api/cli/v1"), handle)
    }

    #[test]
    fn advertised_capability_names_match_backend_contract() {
        let cases = [
            (RuntimeCapability::DatasourceList, "datasource.list"),
            (RuntimeCapability::DatasourceGet, "datasource.get"),
            (RuntimeCapability::DatasourceCreate, "datasource.create"),
            (RuntimeCapability::DatasourceUpdate, "datasource.update"),
            (RuntimeCapability::DatasourceDelete, "datasource.delete"),
            (
                RuntimeCapability::MetadataDatabaseList,
                "metadata.database.list",
            ),
            (
                RuntimeCapability::MetadataSchemaList,
                "metadata.schema.list",
            ),
            (RuntimeCapability::MetadataTableList, "metadata.table.list"),
            (
                RuntimeCapability::MetadataTableDetail,
                "metadata.table.detail",
            ),
            (RuntimeCapability::SqlQuery, "sql.query"),
        ];

        for (capability, expected) in cases {
            assert_eq!(capability.as_str(), expected);
        }
    }

    #[test]
    fn sql_requests_have_no_total_timeout() {
        assert_eq!(
            RuntimeRequestKind::Standard.timeout(),
            Some(Duration::from_secs(60))
        );
        assert_eq!(RuntimeRequestKind::Sql.timeout(), None);
        assert_eq!(RUNTIME_CONNECT_TIMEOUT, Duration::from_secs(10));
    }

    #[test]
    fn supported_health_and_capability_pass_contract_validation() {
        for edition in [Edition::Community, Edition::Pro, Edition::Local] {
            validate_runtime_health(&compatible_health(edition.as_str()), edition).unwrap();
        }
        validate_runtime_capability(
            &capabilities(&["datasource.list", "sql.query"]),
            RuntimeCapability::SqlQuery,
        )
        .unwrap();
    }

    #[test]
    fn client_verifies_health_and_capabilities_over_runtime_api() {
        let (base_url, server) = spawn_runtime_server(
            json!({
                "ready": true,
                "edition": "pro",
                "version": "5.3.0",
                "apiVersion": "v1"
            }),
            json!({ "features": ["sql.query"] }),
        );
        let client = RuntimeClient::new(base_url, "test-token".to_string());

        client
            .verify_contract(Edition::Pro, RuntimeCapability::SqlQuery)
            .unwrap();
        server.join().unwrap();
    }

    #[test]
    fn runtime_must_be_ready() {
        let mut health = compatible_health("pro");
        health.ready = false;

        let error = validate_runtime_health(&health, Edition::Pro).unwrap_err();

        assert_eq!(runtime_api_code(error), "runtime_not_ready");
    }

    #[test]
    fn runtime_edition_must_match_request() {
        let error = validate_runtime_health(&compatible_health("local"), Edition::Pro).unwrap_err();

        assert_eq!(runtime_api_code(error), "runtime_edition_mismatch");
    }

    #[test]
    fn runtime_api_version_must_be_supported() {
        let mut health = compatible_health("pro");
        health.api_version = Some("v2".to_string());

        let error = validate_runtime_health(&health, Edition::Pro).unwrap_err();

        assert_eq!(runtime_api_code(error), "runtime_api_version_unsupported");
    }

    #[test]
    fn runtime_api_version_is_required() {
        let mut health = compatible_health("pro");
        health.api_version = None;

        let error = validate_runtime_health(&health, Edition::Pro).unwrap_err();

        assert_eq!(runtime_api_code(error), "runtime_api_version_unsupported");
    }

    #[test]
    fn required_capability_must_be_advertised() {
        let error = validate_runtime_capability(
            &capabilities(&["datasource.list"]),
            RuntimeCapability::SqlQuery,
        )
        .unwrap_err();

        assert_eq!(runtime_api_code(error), "runtime_capability_missing");
    }

    #[test]
    fn runtime_error_parts_preserve_details() {
        let error = RuntimeError {
            code: Some("sql_result_set_not_found".to_string()),
            message: Some("SQL result set not found: 2".to_string()),
            details: json!({
                "resultSetId": 2,
                "availableResultSetIds": [1]
            }),
        };

        let (code, message, details) = error.into_parts();

        assert_eq!(code, "sql_result_set_not_found");
        assert_eq!(message, "SQL result set not found: 2");
        assert_eq!(details["resultSetId"], 2);
        assert_eq!(details["availableResultSetIds"], json!([1]));
    }

    #[test]
    fn runtime_error_message_falls_back_to_details_message() {
        let error = RuntimeError {
            code: None,
            message: None,
            details: json!({
                "message": "Public Key Retrieval is not allowed"
            }),
        };

        let (code, message, details) = error.into_parts();

        assert_eq!(code, "runtime_api_error");
        assert_eq!(message, "Public Key Retrieval is not allowed");
        assert_eq!(details["message"], "Public Key Retrieval is not allowed");
    }
}
