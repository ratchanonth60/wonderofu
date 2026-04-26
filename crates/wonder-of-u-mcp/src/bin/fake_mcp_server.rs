use std::io::{self, BufRead, BufReader, Write};

use serde_json::{Value, json};
use wonder_of_u_mcp::{JsonRpcError, JsonRpcResponse};

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();

    loop {
        let Some(message) = read_message(&mut reader)? else {
            return Ok(());
        };
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let id = message.get("id").and_then(Value::as_u64);

        match method {
            "initialize" => write_response(
                &mut writer,
                JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id,
                    result: Some(json!({
                        "protocolVersion": "2024-11-05",
                        "capabilities": {
                            "tools": {"listChanged": true},
                            "resources": {"listChanged": true}
                        },
                        "serverInfo": {
                            "name": "fake-mcp",
                            "version": "0.1.0"
                        }
                    })),
                    error: None,
                },
            )?,
            "notifications/initialized" => {}
            "tools/list" => write_response(
                &mut writer,
                JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id,
                    result: Some(json!({
                        "tools": [
                            {
                                "name": "Echo Text",
                                "description": "Echo text back",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "text": {"type": "string"}
                                    },
                                    "required": ["text"],
                                    "additionalProperties": false
                                }
                            }
                        ]
                    })),
                    error: None,
                },
            )?,
            "resources/list" => write_response(
                &mut writer,
                JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id,
                    result: Some(json!({
                        "resources": [
                            {
                                "uri": "file:///workspace/Cargo.toml",
                                "name": "Workspace Manifest",
                                "description": "Workspace manifest",
                                "mimeType": "text/toml"
                            }
                        ]
                    })),
                    error: None,
                },
            )?,
            _ if id.is_some() => write_response(
                &mut writer,
                JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32601,
                        message: format!("unknown method: {method}"),
                        data: None,
                    }),
                },
            )?,
            _ => {}
        }
    }
}

fn read_message(reader: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut content_length = None;
    let mut line = String::new();
    loop {
        line.clear();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = Some(value.trim().parse::<usize>().map_err(invalid_data)?);
            }
        }
    }

    let content_length = content_length.ok_or_else(|| invalid_data("missing content length"))?;
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(invalid_data)
}

fn write_response(writer: &mut impl Write, response: JsonRpcResponse) -> io::Result<()> {
    let body = serde_json::to_vec(&response).map_err(invalid_data)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()
}

fn invalid_data(error: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}
