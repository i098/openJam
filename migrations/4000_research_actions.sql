-- Research runs, opportunities, actions and the action history. Extends the 0003 tables.

-- One run at a time for each project. `done` and `failed` end a run.
ALTER TABLE research_runs
    ADD COLUMN kind text NOT NULL DEFAULT 'manual'
        CHECK (kind IN ('first_scan', 'backfill', 'manual', 'top_up')),
    ADD COLUMN failure_message text,
    ADD COLUMN actions_proposed integer,
    ADD CONSTRAINT research_runs_status_check CHECK (status IN ('queued', 'running', 'done', 'failed'));
CREATE UNIQUE INDEX research_runs_in_flight_key ON research_runs (project_id)
    WHERE status IN ('queued', 'running');

-- `state`: open, planned, writing, claimed, creation_failed, dismissed.
-- `claim`: {kind, claimId, claimedAt, expiresAt} while an agent holds it.
-- `fingerprint`: the same finding in a later run updates this row and does not add one.
-- `evidence`: {prompts: [{promptId, evidenceKind}], sources: [{url, facts}], plan: {...}}.
ALTER TABLE opportunities
    ADD COLUMN research_run_id uuid REFERENCES research_runs ON DELETE SET NULL,
    ADD COLUMN fingerprint text,
    ADD COLUMN dismissed_at timestamptz,
    ADD COLUMN creation_failure jsonb,
    ADD COLUMN evidence jsonb NOT NULL DEFAULT '{}',
    ADD CONSTRAINT opportunities_state_check
        CHECK (state IN ('open', 'planned', 'writing', 'claimed', 'creation_failed', 'dismissed'));
CREATE UNIQUE INDEX opportunities_fingerprint_key ON opportunities (project_id, fingerprint)
    WHERE fingerprint IS NOT NULL;

-- `status` is derived from `action_events` and kept here for fast reads.
-- `check_spec`: what Jam looks at to check the action, for example {"type": "llms_txt"}.
ALTER TABLE actions
    ALTER COLUMN status SET DEFAULT 'suggested',
    ADD COLUMN research_run_id uuid REFERENCES research_runs ON DELETE SET NULL,
    ADD COLUMN fingerprint text,
    ADD COLUMN source text NOT NULL DEFAULT 'research' CHECK (source IN ('research', 'planner', 'user')),
    ADD COLUMN completion_token text,
    ADD COLUMN sources text[] NOT NULL DEFAULT '{}',
    ADD COLUMN questions jsonb NOT NULL DEFAULT '[]',
    ADD COLUMN check_spec jsonb,
    ADD COLUMN first_seen_at timestamptz NOT NULL DEFAULT now(),
    ADD COLUMN last_seen_at timestamptz NOT NULL DEFAULT now();
UPDATE actions SET status = 'suggested' WHERE status = 'open';
ALTER TABLE actions
    ADD CONSTRAINT actions_status_check CHECK (status IN
        ('suggested', 'in_progress', 'sent', 'done', 'verified', 'unverified', 'dismissed', 'superseded')),
    ADD CONSTRAINT actions_kind_check CHECK (kind IN
        ('page_edit', 'new_page', 'outreach', 'technical', 'community')),
    ADD CONSTRAINT actions_impact_check CHECK (impact IN ('high', 'medium', 'low'));
CREATE UNIQUE INDEX actions_fingerprint_key ON actions (project_id, fingerprint)
    WHERE fingerprint IS NOT NULL;
CREATE UNIQUE INDEX actions_completion_token_key ON actions (completion_token)
    WHERE completion_token IS NOT NULL;

CREATE TABLE action_events (
    id bigserial PRIMARY KEY,
    action_id uuid NOT NULL REFERENCES actions ON DELETE CASCADE,
    type text NOT NULL CHECK (type IN
        ('suggested', 'started', 'sent', 'marked_done', 'delivered', 'verification_passed',
         'verification_failed', 'dismissed', 'superseded', 'reopened', 'restored')),
    actor text NOT NULL CHECK (actor IN ('system', 'jam', 'user', 'agent')),
    at timestamptz NOT NULL DEFAULT now(),
    url text,
    evidence text,
    note text
);
CREATE INDEX action_events_action_idx ON action_events (action_id, id);
