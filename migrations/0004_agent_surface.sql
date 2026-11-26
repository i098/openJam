-- Agent surface: OAuth, agent connect, webhooks, notifications, chat.

CREATE TABLE oauth_clients (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name text NOT NULL,
    redirect_uris text[] NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE oauth_tokens (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    client_id uuid NOT NULL REFERENCES oauth_clients ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    workspace_id uuid NOT NULL REFERENCES workspaces ON DELETE CASCADE,
    access_hash text NOT NULL UNIQUE,
    refresh_hash text UNIQUE,
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE agent_connect_sessions (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    poll_token_hash text NOT NULL UNIQUE,
    user_code text NOT NULL UNIQUE,
    workspace_id uuid REFERENCES workspaces ON DELETE CASCADE,
    approved_by uuid REFERENCES users ON DELETE SET NULL,
    api_key_id uuid REFERENCES api_keys ON DELETE SET NULL,
    expires_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE webhooks (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id uuid NOT NULL REFERENCES workspaces ON DELETE CASCADE,
    name text NOT NULL,
    url text NOT NULL,
    enabled boolean NOT NULL DEFAULT true,
    secret text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE webhook_deliveries (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    webhook_id uuid NOT NULL REFERENCES webhooks ON DELETE CASCADE,
    event text NOT NULL,
    payload jsonb NOT NULL,
    status text NOT NULL DEFAULT 'queued',
    response_code integer,
    attempts integer NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE notification_rules (
    workspace_id uuid NOT NULL REFERENCES workspaces ON DELETE CASCADE,
    category text NOT NULL,
    channels text[] NOT NULL DEFAULT '{}',
    is_active boolean NOT NULL DEFAULT true,
    batching_enabled boolean NOT NULL DEFAULT false,
    batch_window_minutes integer NOT NULL DEFAULT 60,
    quiet_start text,
    quiet_end text,
    timezone text,
    PRIMARY KEY (workspace_id, category)
);

CREATE TABLE notifications (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    kind text NOT NULL,
    body jsonb NOT NULL DEFAULT '{}',
    read_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX notifications_user_idx ON notifications (user_id, created_at DESC);

CREATE TABLE chat_runs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects ON DELETE CASCADE,
    status text NOT NULL DEFAULT 'queued',
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE chat_messages (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id uuid NOT NULL REFERENCES chat_runs ON DELETE CASCADE,
    role text NOT NULL,
    parts jsonb NOT NULL DEFAULT '[]',
    created_at timestamptz NOT NULL DEFAULT now()
);
