-- M5 content and publishing: revision metadata, destination connection, delivery schedule, publications.

ALTER TABLE contents
    ADD COLUMN discarded_at timestamptz,
    ADD COLUMN update_failure jsonb;

ALTER TABLE content_revisions
    ADD COLUMN title text NOT NULL DEFAULT '',
    ADD COLUMN source text NOT NULL DEFAULT 'agent' CHECK (source IN ('pipeline', 'human', 'agent')),
    ADD COLUMN basis_revision_id uuid REFERENCES content_revisions ON DELETE SET NULL,
    ADD COLUMN evidence_bundle_id uuid,
    ADD COLUMN findings jsonb NOT NULL DEFAULT '[]',
    ADD COLUMN needs_review boolean NOT NULL DEFAULT false;

ALTER TABLE destinations
    ADD COLUMN connection jsonb NOT NULL DEFAULT '{"status":"problem","message":"Connect a GitHub repository."}',
    ADD COLUMN last_auto_at timestamptz;
-- Auto-publish is on for at most one destination of a project.
CREATE UNIQUE INDEX destinations_auto_publish_idx ON destinations (project_id) WHERE auto_publish;
-- One repository belongs to one destination: two workspaces must not write to the same one.
CREATE UNIQUE INDEX destinations_repository_idx
    ON destinations (lower(config ->> 'owner'), lower(config ->> 'name'))
    WHERE config ? 'owner';

ALTER TABLE deliveries
    ALTER COLUMN status SET DEFAULT 'scheduled',
    ADD COLUMN revision_id uuid REFERENCES content_revisions ON DELETE SET NULL,
    ADD COLUMN automatic boolean NOT NULL DEFAULT false,
    ADD COLUMN deliver_at timestamptz NOT NULL DEFAULT now(),
    ADD COLUMN cancelled_at timestamptz,
    ADD COLUMN attempted_at timestamptz,
    ADD COLUMN branch text,
    ADD COLUMN pr_number integer,
    ADD COLUMN publication_id uuid,
    -- The repository this delivery was opened on, so a later destination change cannot re-point it.
    ADD COLUMN config jsonb;
UPDATE deliveries SET status = 'scheduled' WHERE status = 'queued';
ALTER TABLE deliveries
    ADD CONSTRAINT deliveries_status_check
    CHECK (status IN ('scheduled', 'in_review', 'live', 'failed', 'cancelled'));
CREATE INDEX deliveries_content_idx ON deliveries (content_id, created_at DESC);

CREATE TABLE publications (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    content_id uuid NOT NULL REFERENCES contents ON DELETE CASCADE,
    delivery_id uuid REFERENCES deliveries ON DELETE SET NULL,
    url text NOT NULL,
    published_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (content_id, url)
);
