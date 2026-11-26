-- M6 agent surface: OAuth codes, the event log, signed webhook retries, scan-run events.

-- One-use authorization codes for the OAuth 2.1 code flow with PKCE.
CREATE TABLE oauth_codes (
    code_hash text PRIMARY KEY,
    client_id uuid NOT NULL REFERENCES oauth_clients ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    workspace_id uuid NOT NULL REFERENCES workspaces ON DELETE CASCADE,
    redirect_uri text NOT NULL,
    code_challenge text NOT NULL,
    expires_at timestamptz NOT NULL,
    used_at timestamptz
);

ALTER TABLE agent_connect_sessions ADD COLUMN agent_name text NOT NULL DEFAULT 'Agent';

-- The workspace event log. Webhooks, inbox notifications and the SSE stream all read it.
CREATE TABLE events (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    workspace_id uuid NOT NULL REFERENCES workspaces ON DELETE CASCADE,
    type text NOT NULL,
    category text NOT NULL
        CHECK (category IN ('action_required', 'error', 'status_update', 'insight', 'completion')),
    payload jsonb NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now(),
    fanned_out_at timestamptz
);
CREATE INDEX events_workspace_idx ON events (workspace_id, id);
CREATE INDEX events_pending_idx ON events (id) WHERE fanned_out_at IS NULL;

ALTER TABLE webhook_deliveries
    ADD COLUMN event_id bigint REFERENCES events ON DELETE SET NULL,
    ADD COLUMN next_attempt_at timestamptz NOT NULL DEFAULT now(),
    ADD COLUMN last_error text;
CREATE INDEX webhook_deliveries_due_idx ON webhook_deliveries (next_attempt_at) WHERE status = 'queued';

ALTER TABLE notifications
    ADD COLUMN workspace_id uuid REFERENCES workspaces ON DELETE CASCADE,
    ADD COLUMN category text;

-- A scan run that reaches an end state is an event. A trigger keeps the worker free of this.
CREATE FUNCTION emit_scan_run_event() RETURNS trigger AS $$
DECLARE
    ws uuid;
BEGIN
    IF NEW.status = OLD.status OR NEW.status IN ('queued', 'running') THEN
        RETURN NEW;
    END IF;
    SELECT workspace_id INTO ws FROM projects WHERE id = NEW.project_id;
    INSERT INTO events (workspace_id, type, category, payload)
    VALUES (
        ws,
        'scan.' || NEW.status,
        CASE NEW.status
            WHEN 'complete' THEN 'completion'
            WHEN 'partial' THEN 'status_update'
            WHEN 'cap_reached' THEN 'action_required'
            ELSE 'error'
        END,
        jsonb_build_object(
            'runId', NEW.id,
            'projectId', NEW.project_id,
            'status', NEW.status,
            'promptCount', NEW.prompt_count,
            'platformCount', NEW.platform_count
        )
    );
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER scan_runs_event AFTER UPDATE OF status ON scan_runs
    FOR EACH ROW EXECUTE FUNCTION emit_scan_run_event();
