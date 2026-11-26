-- Core tables: accounts, sessions, API keys, projects, job queue, usage ledger, engine backends.

CREATE TABLE workspaces (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name text NOT NULL,
    spend_cap_cents_month integer CHECK (spend_cap_cents_month >= 0),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE users (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    email text NOT NULL,
    name text,
    password_hash text,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX users_email_key ON users (lower(email));

CREATE TABLE memberships (
    workspace_id uuid NOT NULL REFERENCES workspaces ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    role text NOT NULL CHECK (role IN ('owner', 'member')),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, user_id)
);
CREATE INDEX memberships_user_idx ON memberships (user_id, created_at);

-- Session tokens are random; only their SHA-256 is stored.
CREATE TABLE sessions (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    token_hash text NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL
);
CREATE INDEX sessions_expires_idx ON sessions (expires_at);

-- API keys (`sk_...`) are shown once; only their SHA-256 is stored.
CREATE TABLE api_keys (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id uuid NOT NULL REFERENCES workspaces ON DELETE CASCADE,
    created_by uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    name text NOT NULL,
    prefix text NOT NULL,
    hash text NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz,
    revoked_at timestamptz
);
CREATE INDEX api_keys_workspace_idx ON api_keys (workspace_id, created_at DESC);

CREATE TABLE projects (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id uuid NOT NULL REFERENCES workspaces ON DELETE CASCADE,
    name text NOT NULL,
    domain text NOT NULL,
    site_url text NOT NULL,
    status text NOT NULL DEFAULT 'setup' CHECK (status IN ('setup', 'active', 'paused', 'blocked')),
    platforms text[] NOT NULL DEFAULT '{openai,perplexity,claude}',
    cadence text NOT NULL DEFAULT 'daily' CHECK (cadence IN ('daily', 'weekly')),
    locale text NOT NULL DEFAULT 'en-US',
    first_scan_at timestamptz,
    last_scan_at timestamptz,
    next_scan_at timestamptz,
    archived_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX projects_workspace_idx ON projects (workspace_id, created_at DESC);

-- Postgres job queue. Claim with FOR UPDATE SKIP LOCKED. Retry with backoff, then `dead`.
CREATE TABLE jobs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    queue text NOT NULL,
    payload jsonb NOT NULL DEFAULT '{}',
    idempotency_key text,
    status text NOT NULL DEFAULT 'queued' CHECK (status IN ('queued', 'running', 'done', 'dead')),
    run_at timestamptz NOT NULL DEFAULT now(),
    attempts integer NOT NULL DEFAULT 0,
    max_attempts integer NOT NULL DEFAULT 5,
    locked_by text,
    locked_at timestamptz,
    last_error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    finished_at timestamptz
);
CREATE INDEX jobs_claim_idx ON jobs (queue, run_at) WHERE status = 'queued';
CREATE INDEX jobs_running_idx ON jobs (locked_at) WHERE status = 'running';
CREATE UNIQUE INDEX jobs_idempotency_key ON jobs (queue, idempotency_key) WHERE idempotency_key IS NOT NULL;

-- One row per provider call, plus one per scan. Used for the optional scan cap and cost reports.
CREATE TABLE usage_ledger (
    id bigserial PRIMARY KEY,
    workspace_id uuid NOT NULL REFERENCES workspaces ON DELETE CASCADE,
    project_id uuid REFERENCES projects ON DELETE SET NULL,
    at timestamptz NOT NULL DEFAULT now(),
    kind text NOT NULL,
    provider text,
    model text,
    backend_id uuid,
    tokens_in integer NOT NULL DEFAULT 0,
    tokens_out integer NOT NULL DEFAULT 0,
    cost_cents numeric(12, 4) NOT NULL DEFAULT 0,
    ref text
);
CREATE INDEX usage_ledger_workspace_idx ON usage_ledger (workspace_id, kind, at DESC);

-- Engine access. One backend row per provider for a workspace; any number of accounts under it,
-- each an API key or a subscription login. Rotation and health fields come in the engine slices.
CREATE TABLE engine_backends (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id uuid NOT NULL REFERENCES workspaces ON DELETE CASCADE,
    provider text NOT NULL CHECK (provider IN ('openai', 'gemini', 'perplexity', 'claude')),
    model text,
    enabled boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, provider)
);

CREATE TABLE engine_accounts (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    backend_id uuid NOT NULL REFERENCES engine_backends ON DELETE CASCADE,
    mode text NOT NULL CHECK (mode IN ('plan', 'key')),
    label text NOT NULL,
    account_ref text,
    credential_ciphertext bytea,
    enabled boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (backend_id, label)
);
