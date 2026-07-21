use serde::Serialize;
use serde_json::{Value, json};

use crate::edition::Edition;
use crate::error::{Chat2DbError, Result};
use crate::installer::SystemPaths;
use crate::mcp::{mcp_url, read_mcp_auth_token};

pub const MCP_TOKEN_HEADER: &str = "x-chat2db-mcp-token";

#[derive(Debug, Clone)]
pub struct McpConfigRequest {
    pub edition: Edition,
    pub paths: SystemPaths,
}

impl McpConfigRequest {
    pub fn production(edition: Edition) -> Self {
        Self {
            edition,
            paths: SystemPaths::production(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct McpConfigOutcome {
    pub command: &'static str,
    pub edition: Edition,
    pub config: Value,
}

pub fn mcp_config(request: McpConfigRequest) -> Result<McpConfigOutcome> {
    let token =
        read_mcp_auth_token(request.edition, &request.paths.home_dir).and_then(|token| {
            token.ok_or_else(|| Chat2DbError::McpTokenMissing {
                edition: request.edition.to_string(),
            })
        })?;

    Ok(McpConfigOutcome {
        command: "mcp_config",
        edition: request.edition,
        config: render_mcp_config_json(&token, request.edition),
    })
}

fn render_mcp_config_json(token: &str, edition: Edition) -> Value {
    json!({
        "mcpServers": {
            "chat2db": {
                "transport": "streamable_http",
                "url": mcp_url(edition),
                "headers": {
                    MCP_TOKEN_HEADER: token,
                },
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_json_template_with_token() {
        let value = render_mcp_config_json("secret-token", Edition::Community);

        assert_eq!(
            value["mcpServers"]["chat2db"]["transport"],
            "streamable_http"
        );
        assert_eq!(
            value["mcpServers"]["chat2db"]["url"],
            "http://127.0.0.1:10825/mcp"
        );
        assert_eq!(
            value["mcpServers"]["chat2db"]["headers"][MCP_TOKEN_HEADER],
            "secret-token"
        );
    }
}
