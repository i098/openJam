//! An MCP server over the `/v1` API.
//!
//! Every built `/v1` operation in the OpenAPI contract becomes one tool. A tool call runs
//! the matching request through the `/v1` router with the caller's own `Authorization`
//! header, so the tools can do what the token can do and nothing more. Transport: Streamable
//! HTTP with one JSON reply per POST (no session, no server-initiated stream).

use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::State,
    http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::{Map, Value, json};
use std::{collections::BTreeSet, sync::Arc};
use tower::ServiceExt;

/// Newest first. A client asking for an older one gets it back; anything else gets the newest.
const PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const MAX_RESULT_BYTES: usize = 4 * 1024 * 1024;
/// Operations with these tags are for people at a dashboard, not for agents.
const HIDDEN_TAGS: [&str; 2] = ["API keys", "Agent connect"];
const NOT_BUILT: &str = "(not built yet)";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    Path,
    Query,
}

#[derive(Debug)]
struct Param {
    name: String,
    place: Place,
    required: bool,
}

/// One tool: a `/v1` operation and how to build its request.
#[derive(Debug)]
pub struct Tool {
    pub name: String,
    method: Method,
    path: String,
    params: Vec<Param>,
    has_body: bool,
    body_required: bool,
    listing: Value,
}

type BaseUrl = Arc<dyn Fn(&HeaderMap) -> String + Send + Sync>;

#[derive(Clone)]
struct Mcp {
    v1: Router,
    tools: Arc<Vec<Tool>>,
    base: BaseUrl,
}

/// The `/mcp` route. `v1` has its state applied. `openapi` is the contract as JSON.
/// `base` returns the public origin of this server for a request, for the sign-in challenge.
pub fn router(
    v1: Router,
    openapi: &Value,
    base: impl Fn(&HeaderMap) -> String + Send + Sync + 'static,
) -> Router {
    let mcp = Mcp {
        v1,
        tools: Arc::new(tools_from_openapi(openapi)),
        base: Arc::new(base),
    };
    Router::new().route("/mcp", post(handle)).with_state(mcp)
}

/// Names of the tools the contract yields. For tests and startup logs.
pub fn tool_names(openapi: &Value) -> Vec<String> {
    tools_from_openapi(openapi)
        .into_iter()
        .map(|t| t.name)
        .collect()
}

// ---- contract to tools ----------------------------------------------------------------------

fn tools_from_openapi(doc: &Value) -> Vec<Tool> {
    let schemas = doc
        .pointer("/components/schemas")
        .and_then(Value::as_object);
    let mut tools = Vec::new();
    let Some(paths) = doc.get("paths").and_then(Value::as_object) else {
        return tools;
    };
    for (path, item) in paths {
        for (verb, op) in item.as_object().into_iter().flatten() {
            let Ok(method) = verb.to_uppercase().parse::<Method>() else {
                continue;
            };
            if let Some(tool) = tool_from_op(path, method, op, schemas) {
                tools.push(tool);
            }
        }
    }
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    tools
}

fn tool_from_op(
    path: &str,
    method: Method,
    op: &Value,
    schemas: Option<&Map<String, Value>>,
) -> Option<Tool> {
    let name = op.get("operationId")?.as_str()?.to_string();
    let summary = op.get("summary").and_then(Value::as_str).unwrap_or("");
    let has_auth = op
        .get("security")
        .and_then(Value::as_array)
        .is_some_and(|s| !s.is_empty());
    let hidden = op.get("tags").and_then(Value::as_array).is_some_and(|t| {
        t.iter()
            .any(|t| HIDDEN_TAGS.contains(&t.as_str().unwrap_or("")))
    });
    // A stream never ends, so it cannot be a tool call.
    let streams = op
        .get("responses")
        .and_then(Value::as_object)
        .is_some_and(|r| {
            r.values()
                .any(|r| r.pointer("/content/text~1event-stream").is_some())
        });
    if !has_auth || hidden || streams || summary.ends_with(NOT_BUILT) {
        return None;
    }

    let mut properties = Map::new();
    let mut required = Vec::new();
    let mut params = Vec::new();
    for p in op
        .get("parameters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let place = match p.get("in").and_then(Value::as_str) {
            Some("path") => Place::Path,
            Some("query") => Place::Query,
            _ => continue,
        };
        let pname = p.get("name")?.as_str()?.to_string();
        let is_required =
            place == Place::Path || p.get("required").and_then(Value::as_bool).unwrap_or(false);
        let mut schema = p.get("schema").cloned().unwrap_or_else(|| json!({}));
        if let (Some(d), Some(obj)) = (p.get("description"), schema.as_object_mut()) {
            obj.entry("description").or_insert_with(|| d.clone());
        }
        properties.insert(pname.clone(), schema);
        if is_required {
            required.push(json!(pname));
        }
        params.push(Param {
            name: pname,
            place,
            required: is_required,
        });
    }
    let body = op.pointer("/requestBody/content/application~1json/schema");
    let body_required = op
        .pointer("/requestBody/required")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if let Some(body) = body {
        properties.insert("body".into(), body.clone());
        if body_required {
            required.push(json!("body"));
        }
    }

    let mut input = json!({ "type": "object", "properties": properties, "required": required });
    let mut refs = BTreeSet::new();
    collect_refs(&input, &mut refs);
    let mut defs = Map::new();
    while let Some(next) = refs.iter().find(|r| !defs.contains_key(*r)).cloned() {
        let Some(schema) = schemas.and_then(|s| s.get(&next)) else {
            defs.insert(next, json!({}));
            continue;
        };
        collect_refs(schema, &mut refs);
        defs.insert(next, schema.clone());
    }
    if !defs.is_empty() {
        input["$defs"] = Value::Object(defs);
    }
    let input: Value = serde_json::from_str(
        &input
            .to_string()
            .replace("#/components/schemas/", "#/$defs/"),
    )
    .unwrap_or(input);

    let description = match op.get("description").and_then(Value::as_str) {
        Some(d) if !d.is_empty() => format!("{summary}\n\n{d}"),
        _ => summary.to_string(),
    };
    let listing = json!({
        "name": name,
        "description": description,
        "inputSchema": input,
        "annotations": {
            "readOnlyHint": method == Method::GET,
            "destructiveHint": method == Method::DELETE,
        },
    });
    Some(Tool {
        name,
        method,
        path: path.to_string(),
        params,
        has_body: body.is_some(),
        body_required,
        listing,
    })
}

fn collect_refs(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if let Some(name) = map
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|r| r.strip_prefix("#/components/schemas/"))
            {
                out.insert(name.to_string());
            }
            map.values().for_each(|v| collect_refs(v, out));
        }
        Value::Array(items) => items.iter().for_each(|v| collect_refs(v, out)),
        _ => {}
    }
}

// ---- tool call to /v1 request ---------------------------------------------------------------

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn encode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl Tool {
    /// The request URI for these arguments, or what is wrong with them.
    fn uri(&self, args: &Map<String, Value>) -> Result<String, String> {
        let mut path = self.path.clone();
        let mut query = Vec::new();
        for p in &self.params {
            let value = args.get(&p.name).filter(|v| !v.is_null());
            let Some(value) = value else {
                if p.required {
                    return Err(format!("missing argument `{}`", p.name));
                }
                continue;
            };
            match p.place {
                Place::Path => {
                    let text = scalar(value)
                        .ok_or_else(|| format!("`{}` must be a string or number", p.name))?;
                    path = path.replace(&format!("{{{}}}", p.name), &encode(&text));
                }
                Place::Query => {
                    let items = value
                        .as_array()
                        .map_or_else(|| vec![value], |a| a.iter().collect());
                    for item in items {
                        let text = scalar(item).ok_or_else(|| {
                            format!("`{}` must be a scalar or a list of scalars", p.name)
                        })?;
                        query.push(format!("{}={}", encode(&p.name), encode(&text)));
                    }
                }
            }
        }
        if query.is_empty() {
            Ok(path)
        } else {
            Ok(format!("{path}?{}", query.join("&")))
        }
    }
}

enum CallOutcome {
    Result { text: String, is_error: bool },
    Unauthorized,
}

async fn call(
    mcp: &Mcp,
    tool: &Tool,
    args: &Map<String, Value>,
    auth: &HeaderValue,
) -> CallOutcome {
    let failed = |text: String| CallOutcome::Result {
        text,
        is_error: true,
    };
    let uri = match tool.uri(args) {
        Ok(uri) => uri,
        Err(e) => return failed(e),
    };
    let body = match args.get("body").filter(|b| !b.is_null()) {
        Some(b) if tool.has_body => Body::from(b.to_string()),
        None if tool.body_required => return failed("missing argument `body`".into()),
        _ => Body::empty(),
    };
    let request = Request::builder()
        .method(tool.method.clone())
        .uri(uri)
        .header(header::AUTHORIZATION, auth)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body);
    let Ok(request) = request else {
        return failed("the arguments do not make a valid request".into());
    };
    let Ok(response) = mcp.v1.clone().oneshot(request).await;
    let status = response.status();
    if status == StatusCode::UNAUTHORIZED {
        return CallOutcome::Unauthorized;
    }
    let bytes = to_bytes(response.into_body(), MAX_RESULT_BYTES)
        .await
        .unwrap_or_default();
    let text = if bytes.is_empty() {
        format!("HTTP {}", status.as_u16())
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    CallOutcome::Result {
        text,
        is_error: !status.is_success(),
    }
}

// ---- JSON-RPC over HTTP ---------------------------------------------------------------------

fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn json_response(status: StatusCode, body: &Value) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// 401 with the pointer a client needs to start the OAuth flow.
fn challenge(mcp: &Mcp, headers: &HeaderMap) -> Response {
    let base = (mcp.base)(headers);
    let mut response = json_response(
        StatusCode::UNAUTHORIZED,
        &json!({
            "error": "missing, malformed or revoked credentials",
            "code": "unauthorized",
        }),
    );
    if let Ok(value) = HeaderValue::from_str(&format!(
        "Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource\""
    )) {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, value);
    }
    response
}

async fn token_is_valid(mcp: &Mcp, auth: &HeaderValue) -> bool {
    let request = Request::builder()
        .uri("/v1/me")
        .header(header::AUTHORIZATION, auth)
        .body(Body::empty());
    let Ok(request) = request else { return false };
    let Ok(response) = mcp.v1.clone().oneshot(request).await;
    response.status().is_success()
}

async fn handle(State(mcp): State<Mcp>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(auth) = headers.get(header::AUTHORIZATION).cloned() else {
        return challenge(&mcp, &headers);
    };
    let Ok(message) = serde_json::from_slice::<Value>(&body) else {
        return json_response(
            StatusCode::BAD_REQUEST,
            &rpc_error(Value::Null, -32700, "parse error"),
        );
    };
    let Some(message) = message.as_object() else {
        return json_response(
            StatusCode::BAD_REQUEST,
            &rpc_error(Value::Null, -32600, "send one JSON-RPC request per POST"),
        );
    };
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let Some(id) = message.get("id").cloned() else {
        // A notification, or a response from the client. Nothing to answer.
        return StatusCode::ACCEPTED.into_response();
    };
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let reply = |result: Value| {
        json_response(
            StatusCode::OK,
            &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        )
    };

    match method {
        "ping" => reply(json!({})),
        "initialize" | "tools/list" if !token_is_valid(&mcp, &auth).await => {
            challenge(&mcp, &headers)
        }
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str);
            let version = PROTOCOLS
                .iter()
                .find(|v| Some(**v) == asked)
                .unwrap_or(&PROTOCOLS[0]);
            reply(json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "openjam", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Tools map one to one to the openJam /v1 API. Pass path and query parameters as named arguments and the JSON request body as `body`.",
            }))
        }
        "tools/list" => {
            reply(json!({ "tools": mcp.tools.iter().map(|t| &t.listing).collect::<Vec<_>>() }))
        }
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let Some(tool) = mcp.tools.iter().find(|t| t.name == name) else {
                return json_response(
                    StatusCode::OK,
                    &rpc_error(id, -32602, &format!("unknown tool `{name}`")),
                );
            };
            let args = match params.get("arguments") {
                Some(Value::Object(map)) => map.clone(),
                None | Some(Value::Null) => Map::new(),
                Some(_) => {
                    return json_response(
                        StatusCode::OK,
                        &rpc_error(id, -32602, "`arguments` must be an object"),
                    );
                }
            };
            match call(&mcp, tool, &args, &auth).await {
                CallOutcome::Unauthorized => challenge(&mcp, &headers),
                CallOutcome::Result { text, is_error } => reply(json!({
                    "content": [{ "type": "text", "text": text }],
                    "isError": is_error,
                })),
            }
        }
        other => json_response(
            StatusCode::OK,
            &rpc_error(id, -32601, &format!("method `{other}` is not supported")),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Value {
        json!({
            "paths": {
                "/v1/things/{thingId}": {
                    "get": {
                        "operationId": "get_thing", "summary": "Get a thing", "tags": ["Things"],
                        "security": [{"bearerAuth": []}],
                        "parameters": [
                            {"name": "thingId", "in": "path", "required": true, "schema": {"type": "string"}},
                            {"name": "tag", "in": "query", "schema": {"type": "array", "items": {"type": "string"}}},
                            {"name": "full", "in": "query", "schema": {"type": "boolean"}}
                        ]
                    },
                    "patch": {
                        "operationId": "patch_thing", "summary": "Change a thing", "tags": ["Things"],
                        "security": [{"bearerAuth": []}],
                        "parameters": [{"name": "thingId", "in": "path", "required": true, "schema": {"type": "string"}}],
                        "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Patch"}}}}
                    }
                },
                "/v1/later": {"get": {"operationId": "later", "summary": "Get /v1/later (not built yet)", "security": [{"bearerAuth": []}]}},
                "/v1/api-keys": {"post": {"operationId": "create_api_key", "summary": "Make", "tags": ["API keys"], "security": [{"bearerAuth": []}]}},
                "/v1/public": {"get": {"operationId": "public", "summary": "No auth"}}
            },
            "components": {"schemas": {
                "Patch": {"type": "object", "properties": {"inner": {"$ref": "#/components/schemas/Inner"}}},
                "Inner": {"type": "string"},
                "Unused": {"type": "string"}
            }}
        })
    }

    #[test]
    fn only_built_agent_operations_become_tools() {
        assert_eq!(tool_names(&doc()), ["get_thing", "patch_thing"]);
    }

    #[test]
    fn schema_carries_its_references_and_no_others() {
        let tools = tools_from_openapi(&doc());
        let patch = &tools[1].listing["inputSchema"];
        assert_eq!(patch["properties"]["body"]["$ref"], "#/$defs/Patch");
        assert_eq!(
            patch["$defs"]["Patch"]["properties"]["inner"]["$ref"],
            "#/$defs/Inner"
        );
        assert!(patch["$defs"].get("Unused").is_none());
        assert_eq!(patch["required"], json!(["thingId", "body"]));
    }

    #[test]
    fn uri_fills_path_and_query_and_encodes() {
        let tools = tools_from_openapi(&doc());
        let args = json!({"thingId": "a/b c", "tag": ["x", "y&z"], "full": true});
        assert_eq!(
            tools[0].uri(args.as_object().unwrap()).unwrap(),
            "/v1/things/a%2Fb%20c?tag=x&tag=y%26z&full=true"
        );
        assert_eq!(
            tools[0].uri(&Map::new()).unwrap_err(),
            "missing argument `thingId`"
        );
    }
}
