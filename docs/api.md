# API

Base path `/v1`. Authenticate with an API key or an OAuth access token on every call except the
public operations: start and poll an agent connect session, complete an action with its token,
and read pricing.

```
Authorization: Bearer sk_...
```

Create a key in the dashboard, or with `POST /v1/api-keys` from a signed-in session.

## Errors

Every failure returns a JSON body:

```json
{"error": "readable message", "code": "unauthorized", "request_id": "…", "docs": "…"}
```

| Status | Code | Meaning |
|---|---|---|
| 400 | `invalid_request` | A parameter or body field is wrong. |
| 401 | `unauthorized` | Missing, malformed or revoked credentials. |
| 403 | `forbidden` | The signed-in role cannot do this. |
| 403 | `forbidden_for_api_key` | Sign in to the dashboard instead. |
| 404 | `not_found` | The resource is not in this workspace. |
| 409 | `conflict` | The resource exists already. |
| 422 | `unprocessable` | The request is well formed but this build cannot do it. |
| 429 | `cap_reached` | The daily scan cap stopped this call. |
| 429 | `rate_limited` | Too many calls from this address. Wait, then retry. |
| 500 | `internal_error` | Quote the `request_id`. |
| 501 | `not_implemented` | The operation is in the contract but not built yet. |

The full contract is `crates/api/contract/openapi.yaml`, also served at `/openapi.json`.
`web/src/lib/api-types.ts` is generated from it; CI fails when they drift.

## Agents

### MCP

The server speaks MCP over Streamable HTTP at `/mcp`. Send one JSON-RPC message per `POST`.
Authenticate with the same `Authorization: Bearer` header as the API: an `sk_...` key, or an
OAuth access token. A request without valid credentials answers `401` with
`WWW-Authenticate: Bearer resource_metadata="<origin>/.well-known/oauth-protected-resource"`.

Each `/v1` operation that needs a key and is built becomes one tool.
Pass path and query parameters as named arguments, and the JSON request body as `body`.
Tools cannot do more than the caller's token can do.
Public operations, API key management, agent connect and the event stream are not tools.

### OAuth 2.1

Clients are public: no client secret, and PKCE `S256` is required.

| Path | What |
|---|---|
| `/.well-known/oauth-protected-resource` | The resource metadata for `/mcp`. |
| `/.well-known/oauth-authorization-server` | The issuer and endpoint metadata. |
| `POST /oauth/register` | Dynamic client registration with 1 to 10 `redirect_uris`. |
| `GET /oauth/authorize` | The consent page. Send `response_type=code`, `client_id`, `redirect_uri`, `code_challenge`, `code_challenge_method=S256`, and `state`. |
| `POST /oauth/token` | `authorization_code` (with `code_verifier`) or `refresh_token`. |
| `POST /oauth/revoke` | Revoke an access or refresh token. Always answers 200. |

An authorization code works once and lives 10 minutes. An access token lives 1 hour. A refresh
token lives 30 days and rotates: the old pair stops working when the new one is issued.

### Agent connect

An agent without a key asks for a code, and a workspace owner approves it in a browser.

1. `POST /v1/agent-connect` with `{"name": "..."}` and no credentials. The answer holds
   `connectUrl`, `userCode`, `pollToken`, `expiresAt` and `intervalSeconds`.
2. The owner opens `connectUrl` and approves. Only an owner can approve.
3. `GET /v1/agent-connect/{pollToken}` returns `{"status":"pending"}` until approval. The first
   poll after approval returns `{"status":"approved","apiKey":"sk_...","workspaceId":"..."}`.
   Later polls return `{"status":"claimed"}`. The code lives 15 minutes.

### Webhooks

`POST /v1/webhooks` registers one address. Every enabled webhook receives every event of a
category whose notification rule carries the `webhook` channel, as JSON:

| Header | Value |
|---|---|
| `X-OpenJam-Event` | The event type, for example `scan.complete`. |
| `X-OpenJam-Delivery` | The delivery id. It stays the same across retries. |
| `X-OpenJam-Timestamp` | Unix seconds of this attempt. |
| `X-OpenJam-Signature` | `t=<timestamp>,v1=<hex>`, where `<hex>` is `HMAC-SHA256(secret, "<timestamp>.<raw body>")`. |

The signing secret is in the create response only. A non-2xx answer or a network error retries
after 30 seconds, 2 minutes, 10 minutes, 1 hour and 6 hours. The sixth failed attempt marks the
delivery `failed`.

### Events

`GET /v1/events/stream` sends the workspace event log as server-sent events. Each event has
`id`, `event` (the type) and `data` (the same JSON a webhook receives). With no resume point the
stream starts at the next new event. Send `Last-Event-ID: <id>` or `?after=<id>` to replay
events above that id.
