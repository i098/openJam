# Install, upgrade, back up

openJam runs as four containers: `web`, `api`, `worker` and `db` (Postgres 16).
You need Docker with the Compose plugin,.

## Install

```sh
git clone https://github.com/i098/openJam.git
cd openJam
cp .env.example .env        # optional; every value has a default
docker compose up -d --build
docker compose ps           # wait until api shows "healthy"
```

The first build compiles the Rust binaries and takes several minutes.
Open <http://localhost:8080> and create an account. The first account owns the workspace.
To stop new sign-ups after that, set `OPENJAM_ALLOW_SIGNUP=false` in `.env` and run `docker compose up -d`.

Check the stack:

```sh
curl -fsS http://localhost:8080/healthz        # {"ok":true}
curl -fsS http://127.0.0.1:8081/readyz         # {"database":"up","migrations":4,"ok":true}
```

Migrations run when the `api` and `worker` start. You do not run them by hand.

## Configuration

Compose reads `.env` in the project directory. [.env.example](../.env.example) lists every variable.

| Variable | Default | Meaning |
|---|---|---|
| `POSTGRES_PASSWORD` | `openjam` | Password of the Postgres user. It applies only when the volume is first created; see [Change the database password](#change-the-database-password). |
| `OPENJAM_PORT` | `8080` | Host port of the web app. |
| `OPENJAM_SCAN_CAP_PER_DAY` | empty | Empty: no scan limit. A number: the most scans a workspace may start per UTC day. Over the cap, `/v1` answers `429 cap_reached`. |
| `OPENJAM_COOKIE_SECURE` | `false` | Set `true` when you serve the site over HTTPS, so the browser sends the session cookie only on HTTPS. |
| `OPENJAM_ALLOW_SIGNUP` | `true` | `false` closes `/auth/signup`. |
| `OPENJAM_SESSION_DAYS` | `30` | Days a dashboard sign-in stays valid. |
| `RUST_LOG` | `info` | Log filter. Example: `openjam_api=debug,info`. |
| `OPENJAM_LOG_FORMAT` | `json` | API log format: `json` or `text`. |
| `DATABASE_URL` | set by `compose.yml` | Postgres connection string. Required when you run the binaries by hand. |
| `OPENJAM_BIND` | `0.0.0.0:8080` | Listen address of the API inside its container. |

An invalid value stops the service at start with a message that names the variable.
After you edit `.env`, run `docker compose up -d` to apply it.

### HTTPS

The `web` container serves plain HTTP. Put a reverse proxy (Caddy, nginx, Traefik) in front of
`OPENJAM_PORT`, terminate TLS there, and set `OPENJAM_COOKIE_SECURE=true`.

## Health, logs and metrics

The API listens on `127.0.0.1:8081` on the host, and on `api:8080` inside the Compose network.
The `web` container proxies only `/healthz`; use port 8081 for the other two.

| Endpoint | Use it for | Answer |
|---|---|---|
| `GET /healthz` | Liveness. The process runs. It does not touch the database. | `200 {"ok":true}` |
| `GET /readyz` | Readiness. The database answers and every migration succeeded. | `200`, or `503` with `"database":"down"` or `failed_migrations`. |
| `GET /metrics` | Prometheus scrape. | Text format, below. |

`/metrics` has no authentication. Keep port 8081 bound to `127.0.0.1` or behind your firewall.

| Metric | Type | Meaning |
|---|---|---|
| `openjam_build_info{version}` | gauge | API version. Always 1. |
| `openjam_http_requests_total{class}` | counter | Requests by status class (`2xx`, `4xx`, ...) since the API started. |
| `openjam_http_request_duration_seconds_sum` / `_count` | summary | Total handling time and request count. |
| `openjam_db_up` | gauge | 1 when the last database query worked. |
| `openjam_jobs{status}` | gauge | Jobs in `queued`, `running`, `done` and `dead`. A rising `dead` means jobs fail all retries. |

Prometheus scrape config:

```yaml
scrape_configs:
  - job_name: openjam
    static_configs:
      - targets: ["127.0.0.1:8081"]
```

The API writes one JSON object per line to stdout:

```sh
docker compose logs -f api
```

Every request logs one line with its method, path, status and latency. Set `OPENJAM_LOG_FORMAT=text` for readable lines.

## Upgrade

```sh
docker compose exec -T db pg_dump -U openjam -Fc openjam > openjam-before-upgrade.dump   # back up first
git pull
docker compose up -d --build
docker compose ps
curl -fsS http://127.0.0.1:8081/readyz
```

New migrations apply when the new `api` starts. Migrations only move forward; to go back to an
older version, restore the backup that you made before the upgrade (see below).

## Back up

All data is in the Postgres volume `openjam_pgdata`. The `pg_dump` file is the supported backup:
it is consistent while the stack runs.

```sh
docker compose exec -T db pg_dump -U openjam -Fc openjam > "openjam-$(date +%F).dump"
```

Run it from cron for a daily backup, and copy the file off the host. Keep `.env` with it.

Raw volume copy (stop the stack first, so the files are consistent):

```sh
docker compose stop api worker db
docker run --rm -v openjam_pgdata:/data -v "$PWD":/backup alpine tar czf /backup/pgdata.tar.gz -C /data .
docker compose start db api worker
```

## Restore

Restore into the same stack, or into a fresh install made with the steps above.

```sh
docker compose up -d db
docker compose stop api worker web
docker compose exec -T db psql -U openjam -d postgres -c "DROP DATABASE IF EXISTS openjam WITH (FORCE)" -c "CREATE DATABASE openjam OWNER openjam"
docker compose exec -T db pg_restore -U openjam -d openjam --no-owner < openjam-2026-01-01.dump
docker compose up -d
curl -fsS http://127.0.0.1:8081/readyz
```

To restore the raw volume copy instead, stop the stack, then run
`docker run --rm -v openjam_pgdata:/data -v "$PWD":/backup alpine sh -c "rm -rf /data/* && tar xzf /backup/pgdata.tar.gz -C /data"`
and start the stack.

## Change the database password

Postgres reads `POSTGRES_PASSWORD` only when it creates the volume. To change it later:

```sh
docker compose exec -T db psql -U openjam -d openjam -c "ALTER USER openjam PASSWORD 'new-password'"
# put POSTGRES_PASSWORD=new-password in .env, then:
docker compose up -d
```

## Remove

`docker compose down` stops the stack and keeps the data.
`docker compose down -v` also deletes the volume. This cannot be undone; back up first.

## Troubleshooting

| Symptom | Check |
|---|---|
| `api` stays `starting` | `docker compose logs api`. A wrong `DATABASE_URL` or password shows here. |
| `/readyz` answers 503 | `"database":"down"`: check `docker compose ps db`. `failed_migrations`: read the `api` log, fix, restore the last backup if needed. |
| Sign-in works but the next page asks for sign-in | You use HTTPS with `OPENJAM_COOKIE_SECURE=false`, or HTTP with `true`. |
| Port 8080 is busy | Set `OPENJAM_PORT` in `.env`. |
| `429 cap_reached` | `OPENJAM_SCAN_CAP_PER_DAY` is set. Raise it, or empty it for no cap. |
