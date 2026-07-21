use std::fmt;
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::edition::Edition;
use crate::error::{Chat2DbError, Result};
use crate::installer::SystemPaths;
use crate::mcp::{mcp_url, read_mcp_auth_token};

const MCP_SESSION_ID_HEADER: &str = "Mcp-Session-Id";
const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const JSONRPC_VERSION: &str = "2.0";

#[derive(Debug, Clone)]
pub struct McpToolsRequest {
    pub edition: Edition,
    pub paths: SystemPaths,
}

impl McpToolsRequest {
    pub fn production(edition: Edition) -> Self {
        Self {
            edition,
            paths: SystemPaths::production(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct McpCallRequest {
    pub edition: Edition,
    pub tool_name: String,
    pub arguments: Map<String, Value>,
    pub paths: SystemPaths,
}

impl McpCallRequest {
    pub fn production(edition: Edition, tool_name: String, args_json: &str) -> Result<Self> {
        Ok(Self {
            edition,
            tool_name,
            arguments: parse_args_json(args_json)?,
            paths: SystemPaths::production(),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct McpToolsOutcome {
    pub command: &'static str,
    pub edition: Edition,
    pub url: &'static str,
    pub tools: Vec<McpToolSummary>,
    pub raw_result: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpToolSummary {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Option<Value>,
    pub example_args: Value,
    pub risk_hint: String,
    pub call_example: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpCallOutcome {
    pub command: &'static str,
    pub edition: Edition,
    pub url: &'static str,
    pub tool_name: String,
    pub result: Value,
}

#[derive(Clone)]
struct McpSession {
    token: String,
    session_id: String,
}

impl fmt::Debug for McpSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpSession")
            .field("token", &"<redacted>")
            .field("session_id", &self.session_id)
            .finish()
    }
}

pub trait McpHttpTransport {
    fn post(
        &self,
        url: &str,
        headers: HeaderMap,
        body: Value,
        timeout: Duration,
    ) -> Result<McpHttpResponse>;
}

#[derive(Debug, Clone)]
pub struct McpHttpResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub session_id: Option<String>,
    pub body: String,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ReqwestMcpHttpTransport;

impl McpHttpTransport for ReqwestMcpHttpTransport {
    fn post(
        &self,
        url: &str,
        headers: HeaderMap,
        body: Value,
        timeout: Duration,
    ) -> Result<McpHttpResponse> {
        let client = Client::builder()
            .timeout(timeout)
            .user_agent(concat!("chat2db-cli/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|source| Chat2DbError::McpClientHttp {
                url: url.to_string(),
                message: source.to_string(),
            })?;
        let body = serde_json::to_vec(&body).map_err(|source| Chat2DbError::McpClientProtocol {
            message: format!("failed to encode JSON-RPC request: {source}"),
        })?;
        let response = client
            .post(url)
            .headers(headers)
            .body(body)
            .send()
            .map_err(|source| Chat2DbError::McpClientHttp {
                url: url.to_string(),
                message: source.to_string(),
            })?;

        let status = response.status();
        let headers = response.headers().clone();
        let content_type = headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        let session_id = headers
            .get(MCP_SESSION_ID_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        let body = response
            .text()
            .map_err(|source| Chat2DbError::McpClientHttp {
                url: url.to_string(),
                message: source.to_string(),
            })?;

        if !status.is_success() {
            return Err(Chat2DbError::McpClientHttp {
                url: url.to_string(),
                message: format!("HTTP {status}: {body}"),
            });
        }

        Ok(McpHttpResponse {
            status: status.as_u16(),
            content_type,
            session_id,
            body,
        })
    }
}

pub fn mcp_tools(request: McpToolsRequest) -> Result<McpToolsOutcome> {
    mcp_tools_with(request, &ReqwestMcpHttpTransport)
}

pub fn mcp_call(request: McpCallRequest) -> Result<McpCallOutcome> {
    mcp_call_with(request, &ReqwestMcpHttpTransport)
}

pub fn mcp_tools_with<T: McpHttpTransport + ?Sized>(
    request: McpToolsRequest,
    transport: &T,
) -> Result<McpToolsOutcome> {
    let mut client = McpClient::connect(request.edition, &request.paths.home_dir, transport)?;
    let response = client.request("tools/list", json!({}), transport)?;
    let tools = parse_tools(&response, request.edition)?;
    Ok(McpToolsOutcome {
        command: "mcp_tools",
        edition: request.edition,
        url: mcp_url(request.edition),
        tools,
        raw_result: response,
    })
}

pub fn mcp_call_with<T: McpHttpTransport + ?Sized>(
    request: McpCallRequest,
    transport: &T,
) -> Result<McpCallOutcome> {
    let mut client = McpClient::connect(request.edition, &request.paths.home_dir, transport)?;
    let result = client.request(
        "tools/call",
        json!({
            "name": request.tool_name,
            "arguments": Value::Object(request.arguments),
        }),
        transport,
    )?;
    Ok(McpCallOutcome {
        command: "mcp_call",
        edition: request.edition,
        url: mcp_url(request.edition),
        tool_name: result_tool_name(&result).unwrap_or_else(|| request.tool_name.clone()),
        result,
    })
}

struct McpClient {
    next_id: u64,
    url: &'static str,
    session: McpSession,
}

impl McpClient {
    fn connect<T: McpHttpTransport + ?Sized>(
        edition: Edition,
        home_dir: &std::path::Path,
        transport: &T,
    ) -> Result<Self> {
        let token = read_mcp_auth_token(edition, home_dir)?.ok_or_else(|| {
            Chat2DbError::McpTokenMissing {
                edition: edition.as_str().to_string(),
            }
        })?;
        let mut client = Self {
            next_id: 1,
            url: mcp_url(edition),
            session: McpSession {
                token,
                session_id: String::new(),
            },
        };
        let id = client.next_request_id();
        let response = transport.post(
            client.url,
            client.headers(None)?,
            json!({
                "jsonrpc": JSONRPC_VERSION,
                "id": id,
                "method": "initialize",
                "params": {
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {
                        "name": "chat2db-cli",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }
            }),
            Duration::from_secs(30),
        )?;
        client.session.session_id =
            response
                .session_id
                .clone()
                .ok_or_else(|| Chat2DbError::McpClientProtocol {
                    message: "initialize response missing Mcp-Session-Id".to_string(),
                })?;
        let _ = parse_jsonrpc_response(&response, id)?;
        client.notify_initialized(transport)?;
        Ok(client)
    }

    fn request<T: McpHttpTransport + ?Sized>(
        &mut self,
        method: &str,
        params: Value,
        transport: &T,
    ) -> Result<Value> {
        let id = self.next_request_id();
        let response = transport.post(
            self.url,
            self.headers(Some(&self.session.session_id))?,
            json!({
                "jsonrpc": JSONRPC_VERSION,
                "id": id,
                "method": method,
                "params": params,
            }),
            Duration::from_secs(300),
        )?;
        parse_jsonrpc_response(&response, id)
    }

    fn notify_initialized<T: McpHttpTransport + ?Sized>(&mut self, transport: &T) -> Result<()> {
        let response = transport.post(
            self.url,
            self.headers(Some(&self.session.session_id))?,
            json!({
                "jsonrpc": JSONRPC_VERSION,
                "method": "notifications/initialized",
            }),
            Duration::from_secs(30),
        )?;
        if !(200..300).contains(&response.status) {
            return Err(Chat2DbError::McpClientHttp {
                url: self.url.to_string(),
                message: format!("HTTP {}", response.status),
            });
        }
        Ok(())
    }

    fn next_request_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn headers(&self, session_id: Option<&str>) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/json, text/event-stream"),
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            HeaderName::from_static("x-chat2db-mcp-token"),
            HeaderValue::from_str(&self.session.token).map_err(|source| {
                Chat2DbError::McpClientProtocol {
                    message: format!("invalid MCP token header: {source}"),
                }
            })?,
        );
        headers.insert(
            HeaderName::from_static("mcp-protocol-version"),
            HeaderValue::from_static(MCP_PROTOCOL_VERSION),
        );
        if let Some(session_id) = session_id {
            headers.insert(
                HeaderName::from_static("mcp-session-id"),
                HeaderValue::from_str(session_id).map_err(|source| {
                    Chat2DbError::McpClientProtocol {
                        message: format!("invalid MCP session header: {source}"),
                    }
                })?,
            );
        }
        Ok(headers)
    }
}

pub fn parse_args_json(args_json: &str) -> Result<Map<String, Value>> {
    let value: Value = serde_json::from_str(args_json)
        .map_err(|source| Chat2DbError::McpArgsInvalidJson { source })?;
    value
        .as_object()
        .cloned()
        .ok_or(Chat2DbError::McpArgsNotObject)
}

fn parse_jsonrpc_response(response: &McpHttpResponse, id: u64) -> Result<Value> {
    let value = if response_is_sse(response) {
        parse_sse_jsonrpc_response(&response.body, id)?
    } else {
        serde_json::from_str::<Value>(&response.body).map_err(|source| {
            Chat2DbError::McpClientProtocol {
                message: format!("invalid JSON response: {source}"),
            }
        })?
    };
    parse_jsonrpc_value(value, id)
}

fn response_is_sse(response: &McpHttpResponse) -> bool {
    response
        .content_type
        .as_deref()
        .is_some_and(|content_type| content_type.contains("text/event-stream"))
}

fn parse_sse_jsonrpc_response(body: &str, id: u64) -> Result<Value> {
    let normalized_body = body.replace("\r\n", "\n");
    for event in split_sse_events(&normalized_body) {
        let data = event
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim_start)
            .collect::<Vec<_>>()
            .join("\n");
        if data.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            continue;
        };
        if value.get("id").and_then(Value::as_u64) == Some(id) {
            return Ok(value);
        }
    }
    Err(Chat2DbError::McpClientProtocol {
        message: format!("SSE response did not contain JSON-RPC response id {id}"),
    })
}

fn split_sse_events(body: &str) -> impl Iterator<Item = &str> {
    body.split("\n\n")
}

fn parse_jsonrpc_value(value: Value, id: u64) -> Result<Value> {
    if value.get("id").and_then(Value::as_u64) != Some(id) {
        return Err(Chat2DbError::McpClientProtocol {
            message: format!("response id did not match request id {id}"),
        });
    }
    if let Some(error) = value.get("error") {
        return Err(Chat2DbError::McpClientProtocol {
            message: format!("JSON-RPC error: {error}"),
        });
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| Chat2DbError::McpClientProtocol {
            message: "JSON-RPC response missing result".to_string(),
        })
}

fn parse_tools(result: &Value, edition: Edition) -> Result<Vec<McpToolSummary>> {
    let tools = result
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| Chat2DbError::McpClientProtocol {
            message: "tools/list response missing tools array".to_string(),
        })?;
    Ok(tools
        .iter()
        .filter_map(|tool| {
            let name = tool.get("name")?.as_str()?.to_string();
            let description = tool
                .get("description")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let input_schema = tool.get("inputSchema").cloned();
            let example_args = example_args_from_schema(input_schema.as_ref());
            let args_json = serde_json::to_string(&example_args).ok()?;
            let risk_hint = tool_risk_hint(&name).to_string();
            let call_example = format!(
                "chat2db mcp call {} --edition {} --args-json '{}' --json",
                name,
                edition.as_str(),
                args_json
            );
            Some(McpToolSummary {
                name,
                description,
                input_schema,
                example_args,
                risk_hint,
                call_example,
            })
        })
        .collect())
}

fn example_args_from_schema(input_schema: Option<&Value>) -> Value {
    let Some(schema) = input_schema else {
        return json!({});
    };
    example_value_from_schema(schema).unwrap_or_else(|| json!({}))
}

fn example_value_from_schema(schema: &Value) -> Option<Value> {
    if let Some(value) = schema.get("const") {
        return Some(value.clone());
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return values.first().cloned();
    }

    match schema.get("type").and_then(Value::as_str) {
        Some("object") => Some(example_object_from_schema(schema)),
        Some("array") => Some(json!([])),
        Some("boolean") => Some(json!(false)),
        Some("integer") => Some(json!(0)),
        Some("number") => Some(json!(0)),
        Some("string") => Some(json!("")),
        _ => schema
            .get("anyOf")
            .or_else(|| schema.get("oneOf"))
            .and_then(Value::as_array)
            .and_then(|schemas| schemas.first())
            .and_then(example_value_from_schema),
    }
}

fn example_object_from_schema(schema: &Value) -> Value {
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str);
    let properties = schema.get("properties").and_then(Value::as_object);
    let mut object = Map::new();

    for key in required {
        let value = properties
            .and_then(|properties| properties.get(key))
            .and_then(example_value_from_schema)
            .unwrap_or(Value::Null);
        object.insert(key.to_string(), value);
    }

    Value::Object(object)
}

fn tool_risk_hint(name: &str) -> &'static str {
    let name = name.to_ascii_lowercase();
    let high_risk_terms = [
        "execute", "update", "delete", "drop", "create", "insert", "alter", "import", "export",
        "remove",
    ];
    if high_risk_terms.iter().any(|term| name.contains(term)) {
        return "high-risk-write";
    }

    let read_terms = ["select", "list", "get", "read", "status"];
    if read_terms.iter().any(|term| name.contains(term)) {
        return "read";
    }

    let write_terms = ["write", "save"];
    if write_terms.iter().any(|term| name.contains(term)) {
        return "write";
    }

    "write"
}

fn result_tool_name(result: &Value) -> Option<String> {
    result
        .get("name")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;

    use super::*;
    use crate::installer::SystemPaths;

    #[derive(Debug)]
    struct FakeTransport {
        responses: RefCell<Vec<McpHttpResponse>>,
        requests: RefCell<Vec<Value>>,
        sessions: RefCell<Vec<Option<String>>>,
        urls: RefCell<Vec<String>>,
    }

    impl FakeTransport {
        fn new(responses: Vec<McpHttpResponse>) -> Self {
            Self {
                responses: RefCell::new(responses),
                requests: RefCell::new(Vec::new()),
                sessions: RefCell::new(Vec::new()),
                urls: RefCell::new(Vec::new()),
            }
        }
    }

    impl McpHttpTransport for FakeTransport {
        fn post(
            &self,
            url: &str,
            headers: HeaderMap,
            body: Value,
            _timeout: Duration,
        ) -> Result<McpHttpResponse> {
            self.urls.borrow_mut().push(url.to_string());
            self.requests.borrow_mut().push(body);
            self.sessions.borrow_mut().push(
                headers
                    .get(MCP_SESSION_ID_HEADER)
                    .and_then(|value| value.to_str().ok())
                    .map(ToOwned::to_owned),
            );
            Ok(self.responses.borrow_mut().remove(0))
        }
    }

    #[test]
    fn parses_args_json_object() {
        let args = parse_args_json(r#"{"sql":"select 1"}"#).unwrap();

        assert_eq!(args["sql"], "select 1");
    }

    #[test]
    fn rejects_args_json_array() {
        let error = parse_args_json("[]").unwrap_err();

        assert!(matches!(error, Chat2DbError::McpArgsNotObject));
        assert_eq!(error.code(), "mcp_args_invalid_json");
    }

    #[test]
    fn tools_request_initializes_session_and_lists_tools() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        write_token(&paths, Edition::Pro, "token");
        let transport = FakeTransport::new(vec![
            json_response(1, json!({"protocolVersion":"2025-06-18"}), Some("sid")),
            accepted_response(),
            sse_response(
                r#"event: message
data: {"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"execute_sql","description":"Execute SQL","inputSchema":{"type":"object"}}]}}

"#,
            ),
        ]);

        let outcome = mcp_tools_with(
            McpToolsRequest {
                edition: Edition::Pro,
                paths,
            },
            &transport,
        )
        .unwrap();

        assert_eq!(outcome.command, "mcp_tools");
        assert_eq!(outcome.tools[0].name, "execute_sql");
        assert_eq!(outcome.tools[0].example_args, json!({}));
        assert_eq!(outcome.tools[0].risk_hint, "high-risk-write");
        assert_eq!(
            outcome.tools[0].call_example,
            "chat2db mcp call execute_sql --edition pro --args-json '{}' --json"
        );
        assert_eq!(
            transport.sessions.borrow().as_slice(),
            &[None, Some("sid".to_string()), Some("sid".to_string())]
        );
    }

    #[test]
    fn community_client_uses_community_mcp_port() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        write_token(&paths, Edition::Community, "token");
        let transport = FakeTransport::new(vec![
            json_response(1, json!({"protocolVersion":"2025-06-18"}), Some("sid")),
            accepted_response(),
            json_response(2, json!({"tools":[]}), None),
        ]);

        let outcome = mcp_tools_with(
            McpToolsRequest {
                edition: Edition::Community,
                paths,
            },
            &transport,
        )
        .unwrap();

        assert_eq!(outcome.url, "http://127.0.0.1:10825/mcp");
        assert!(
            transport
                .urls
                .borrow()
                .iter()
                .all(|url| url == "http://127.0.0.1:10825/mcp")
        );
    }

    #[test]
    fn tools_summary_adds_conservative_examples_and_call_examples() {
        let result = json!({
            "tools": [
                {
                    "name": "list_all_tables",
                    "description": "List tables",
                    "inputSchema": {
                        "type": "object",
                        "required": ["datasourceId", "database", "includeSystem"],
                        "properties": {
                            "datasourceId": {"type": "integer"},
                            "database": {"type": "string", "enum": ["main"]},
                            "schema": {"type": "string"},
                            "includeSystem": {"type": "boolean"}
                        }
                    }
                },
                {
                    "name": "save_connection",
                    "inputSchema": {
                        "type": "object",
                        "required": ["config"],
                        "properties": {
                            "config": {
                                "type": "object",
                                "required": ["name"],
                                "properties": {
                                    "name": {"type": "string"}
                                }
                            }
                        }
                    }
                }
            ]
        });

        let tools = parse_tools(&result, Edition::Local).unwrap();

        assert_eq!(
            tools[0].example_args,
            json!({
                "datasourceId": 0,
                "database": "main",
                "includeSystem": false
            })
        );
        assert_eq!(tools[0].risk_hint, "read");
        assert_eq!(
            tools[0].call_example,
            r#"chat2db mcp call list_all_tables --edition local --args-json '{"database":"main","datasourceId":0,"includeSystem":false}' --json"#
        );
        assert_eq!(tools[1].example_args, json!({"config":{"name":""}}));
        assert_eq!(tools[1].risk_hint, "write");
    }

    #[test]
    fn tools_summary_defaults_unknown_tools_to_write() {
        let result = json!({
            "tools": [
                {
                    "name": "describe_capability",
                    "inputSchema": {
                        "type": "object",
                        "required": ["mode"],
                        "properties": {
                            "mode": {"const": "brief"}
                        }
                    }
                }
            ]
        });

        let tools = parse_tools(&result, Edition::Pro).unwrap();

        assert_eq!(tools[0].example_args, json!({"mode":"brief"}));
        assert_eq!(tools[0].risk_hint, "write");
    }

    #[test]
    fn call_request_sends_tool_name_and_arguments() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        write_token(&paths, Edition::Local, "token");
        let transport = FakeTransport::new(vec![
            json_response(1, json!({"protocolVersion":"2025-06-18"}), Some("sid")),
            accepted_response(),
            json_response(
                2,
                json!({"content":[{"type":"text","text":"ok"}],"isError":false}),
                None,
            ),
        ]);
        let request = McpCallRequest {
            edition: Edition::Local,
            tool_name: "execute_sql".to_string(),
            arguments: parse_args_json(r#"{"sql":"select 1"}"#).unwrap(),
            paths,
        };

        let outcome = mcp_call_with(request, &transport).unwrap();

        assert_eq!(outcome.tool_name, "execute_sql");
        let requests = transport.requests.borrow();
        assert_eq!(requests[2]["method"], "tools/call");
        assert_eq!(requests[2]["params"]["name"], "execute_sql");
        assert_eq!(requests[2]["params"]["arguments"]["sql"], "select 1");
    }

    #[test]
    fn missing_token_returns_stable_error() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let transport = FakeTransport::new(Vec::new());

        let error = mcp_tools_with(
            McpToolsRequest {
                edition: Edition::Pro,
                paths,
            },
            &transport,
        )
        .unwrap_err();

        assert!(matches!(error, Chat2DbError::McpTokenMissing { .. }));
        assert_eq!(error.code(), "mcp_token_missing");
    }

    #[test]
    fn jsonrpc_error_returns_protocol_error() {
        let response = McpHttpResponse {
            status: 200,
            content_type: Some("application/json".to_string()),
            session_id: None,
            body: r#"{"jsonrpc":"2.0","id":7,"error":{"code":-32601,"message":"missing"}}"#
                .to_string(),
        };

        let error = parse_jsonrpc_response(&response, 7).unwrap_err();

        assert!(matches!(error, Chat2DbError::McpClientProtocol { .. }));
        assert_eq!(error.code(), "mcp_client_protocol_error");
    }

    #[test]
    fn parses_crlf_sse_response() {
        let value = parse_sse_jsonrpc_response(
            "event: message\r\ndata: {\"jsonrpc\":\"2.0\",\"id\":9,\"result\":{\"ok\":true}}\r\n\r\n",
            9,
        )
        .unwrap();

        assert_eq!(value["result"]["ok"], true);
    }

    fn json_response(id: u64, result: Value, session_id: Option<&str>) -> McpHttpResponse {
        McpHttpResponse {
            status: 200,
            content_type: Some("application/json".to_string()),
            session_id: session_id.map(ToOwned::to_owned),
            body: json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result,
            })
            .to_string(),
        }
    }

    fn accepted_response() -> McpHttpResponse {
        McpHttpResponse {
            status: 202,
            content_type: None,
            session_id: None,
            body: String::new(),
        }
    }

    fn sse_response(body: &str) -> McpHttpResponse {
        McpHttpResponse {
            status: 200,
            content_type: Some("text/event-stream".to_string()),
            session_id: None,
            body: body.to_string(),
        }
    }

    fn test_paths(root: &std::path::Path) -> SystemPaths {
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

    fn write_token(paths: &SystemPaths, edition: Edition, token: &str) {
        let path = crate::mcp::mcp_settings_path(edition, &paths.home_dir);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!(r#"{{"enableMcp":true,"mcpAuthToken":"{token}"}}"#),
        )
        .unwrap();
    }
}
