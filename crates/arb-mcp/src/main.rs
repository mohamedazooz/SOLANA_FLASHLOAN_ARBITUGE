//! Native Rust MCP Server Binary for Pump.fun and Solana Arbitrage Bot.
//!
//! Exposes Model Context Protocol (MCP) tools over stdio to AI agents and assistants.

pub mod tools;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();

    while let Ok(Some(line)) = reader.next_line().await {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(line) {
            Ok(req) => req,
            Err(e) => {
                let err_res = json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": format!("Parse error: {e}") }
                });
                stdout
                    .write_all(format!("{}\n", err_res).as_bytes())
                    .await?;
                stdout.flush().await?;
                continue;
            }
        };

        let req_id = request.id.clone().unwrap_or(Value::Null);

        let response = match request.method.as_str() {
            "initialize" => {
                let init_res = json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": "solana-pumpfun-mcp",
                        "version": "0.1.0"
                    }
                });
                Some(JsonRpcResponse {
                    jsonrpc: "2.0",
                    id: req_id,
                    result: Some(init_res),
                    error: None,
                })
            }
            "notifications/initialized" => None,
            "ping" => Some(JsonRpcResponse {
                jsonrpc: "2.0",
                id: req_id,
                result: Some(json!({})),
                error: None,
            }),
            "tools/list" => {
                let tools_list = json!({
                    "tools": tools::get_tool_definitions()
                });
                Some(JsonRpcResponse {
                    jsonrpc: "2.0",
                    id: req_id,
                    result: Some(tools_list),
                    error: None,
                })
            }
            "tools/call" => {
                let params = request.params.unwrap_or(Value::Null);
                let tool_name = params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let tool_args = params.get("arguments").unwrap_or(&Value::Null);

                match tools::execute_tool(tool_name, tool_args).await {
                    Ok(val) => {
                        let content = json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": serde_json::to_string_pretty(&val).unwrap_or_default()
                                }
                            ]
                        });
                        Some(JsonRpcResponse {
                            jsonrpc: "2.0",
                            id: req_id,
                            result: Some(content),
                            error: None,
                        })
                    }
                    Err(err_msg) => {
                        let err_content = json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": format!("Error executing tool {tool_name}: {err_msg}")
                                }
                            ],
                            "isError": true
                        });
                        Some(JsonRpcResponse {
                            jsonrpc: "2.0",
                            id: req_id,
                            result: Some(err_content),
                            error: None,
                        })
                    }
                }
            }
            other => Some(JsonRpcResponse {
                jsonrpc: "2.0",
                id: req_id,
                result: None,
                error: Some(json!({
                    "code": -32601,
                    "message": format!("Method not found: {other}")
                })),
            }),
        };

        if let Some(res) = response {
            let serialized = serde_json::to_string(&res)?;
            stdout
                .write_all(format!("{}\n", serialized).as_bytes())
                .await?;
            stdout.flush().await?;
        }
    }

    Ok(())
}
