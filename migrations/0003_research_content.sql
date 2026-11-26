-- Research, actions, content, publishing.

CREATE TABLE research_runs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects ON DELETE CASCADE,
    status text NOT NULL DEFAULT 'queued',
    answers_read integer NOT NULL DEFAULT 0,
    started_at timestamptz,
    finished_at timestamptz,
    report jsonb,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX research_runs_project_idx ON research_runs (project_id, created_at DESC);

CREATE TABLE opportunities (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects ON DELETE CASCADE,
    kind text NOT NULL CHECK (kind IN ('create_page', 'update_page')),
    change_kind text,
    target_page_id text,
    title text NOT NULL,
    rationale text,
    topic text,
    persona text,
    intent text,
    score numeric NOT NULL DEFAULT 0,
    state text NOT NULL DEFAULT 'open',
    claim jsonb,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX opportunities_project_idx ON opportunities (project_id, state);

CREATE TABLE actions (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects ON DELETE CASCADE,
    opportunity_id uuid REFERENCES opportunities ON DELETE SET NULL,
    kind text NOT NULL,
    problem text NOT NULL,
    title text NOT NULL,
    why text,
    impact text,
    rank integer NOT NULL DEFAULT 0,
    target_url text,
    status text NOT NULL DEFAULT 'open',
    details jsonb,
    check_result jsonb,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX actions_project_idx ON actions (project_id, status, rank);

CREATE TABLE contents (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects ON DELETE CASCADE,
    opportunity_id uuid REFERENCES opportunities ON DELETE SET NULL,
    state text NOT NULL DEFAULT 'drafted' CHECK (state IN ('drafted', 'discarded')),
    current_revision_id uuid,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX contents_project_idx ON contents (project_id, created_at DESC);

CREATE TABLE content_revisions (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    content_id uuid NOT NULL REFERENCES contents ON DELETE CASCADE,
    n integer NOT NULL,
    article jsonb NOT NULL,
    author text,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (content_id, n)
);

CREATE TABLE destinations (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects ON DELETE CASCADE,
    platform text NOT NULL DEFAULT 'github',
    auto_publish boolean NOT NULL DEFAULT false,
    cadence text NOT NULL DEFAULT 'weekly',
    display_name text,
    config jsonb NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE deliveries (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    content_id uuid NOT NULL REFERENCES contents ON DELETE CASCADE,
    destination_id uuid NOT NULL REFERENCES destinations ON DELETE CASCADE,
    status text NOT NULL DEFAULT 'queued',
    pr_url text,
    published_url text,
    error text,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE site_changes (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects ON DELETE CASCADE,
    kind text NOT NULL,
    diff text,
    prompt text,
    pr_url text,
    created_at timestamptz NOT NULL DEFAULT now()
);
