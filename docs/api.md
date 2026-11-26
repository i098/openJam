# API

Base path `/v1`. Authenticate with an API key on every call except the three public operations:

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
| 429 | `cap_reached` | The daily scan cap stopped this call. |
| 500 | `internal_error` | Quote the `request_id`. |
| 501 | `not_implemented` | The operation is in the contract but not built yet. |

The full contract is `crates/api/contract/openapi.yaml`, also served at `/openapi.json`.
`web/src/lib/api-types.ts` is generated from it; CI fails when they drift.
