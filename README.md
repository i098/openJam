# openJam

Self-hosted AI search visibility tracker. See how AI assistants answer your customers'
questions, find where you are missing, and ship the fixes.

## Install

1. `cp .env.example .env` (optional; the defaults work).
2. `docker compose up -d --build`.
3. Open <http://localhost:8080> and create an account.

Then create an API key in the dashboard and call `GET /v1/projects`:

```sh
curl -H "Authorization: Bearer sk_..." http://localhost:8080/v1/projects
```

## What runs

| Service | What it does |
|---|---|
| `web` | SPA and one nginx origin. It proxies `/v1`, `/auth`, `/healthz` and `/openapi.json` to the API. |
| `api` | Rust API: sessions, API keys, the `/v1` contract, migrations on start. |
| `worker` | Rust job worker: claims Postgres jobs, retries with backoff, keeps a minute tick. |
| `db` | Postgres 16. Migrations in `migrations/`. |

## Limits

No scan limit by default. Set `OPENJAM_SCAN_CAP_PER_DAY` to cap scans per UTC day.

## Development

```sh
docker compose up -d db            # Postgres only
DATABASE_URL=postgres://openjam:openjam@localhost:5432/openjam cargo run -p openjam-api
DATABASE_URL=postgres://openjam:openjam@localhost:5432/openjam cargo run -p openjam-worker
bun --cwd web install && bun --cwd web dev   # SPA on :5173, proxied to :8081
```

`cargo run -p openjam-api --bin openjam-openapi` rewrites `crates/api/contract/openapi.yaml`.
`bun --cwd web run gen:types` rewrites the TypeScript types from it. CI fails when either drifts.

| Path | What |
|---|---|
| `crates/core` | config, errors, auth primitives, job queue, usage ledger |
| `crates/api` | HTTP routes, contract, migrations runner |
| `crates/worker` | job loop |
| `crates/engines` | `Engine` trait and account rotation; adapters come in M1 |
| `migrations/` | SQL; each slice owns a version range (see `migrations/README.md`) |
| `web/` | TanStack Router, React 19, Tailwind 4 |

## API

See [docs/api.md](docs/api.md). The contract is `crates/api/contract/openapi.yaml`.

## License

AGPL-3.0. See [LICENSE](LICENSE).
