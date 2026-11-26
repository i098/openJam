-- Tracking tables: brand, prompts, scans, answers.

CREATE TABLE brand_profiles (
    project_id uuid NOT NULL REFERENCES projects ON DELETE CASCADE,
    version integer NOT NULL,
    positioning text,
    audience text,
    product_description text,
    tone text,
    content_style text,
    excluded_topics text[] NOT NULL DEFAULT '{}',
    not_confused_with text[] NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, version)
);

CREATE TABLE competitors (
    project_id uuid NOT NULL REFERENCES projects ON DELETE CASCADE,
    name text NOT NULL,
    domain text,
    aliases text[] NOT NULL DEFAULT '{}',
    pinned_at timestamptz,
    PRIMARY KEY (project_id, name)
);

CREATE TABLE prompts (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects ON DELETE CASCADE,
    text text NOT NULL,
    topic text,
    persona text,
    intent text,
    list text NOT NULL DEFAULT 'tracked' CHECK (list IN ('tracked', 'archived')),
    status text NOT NULL DEFAULT 'active',
    paused_reason text,
    branded boolean NOT NULL DEFAULT false,
    scored boolean NOT NULL DEFAULT true,
    created_by text,
    updated_at timestamptz NOT NULL DEFAULT now(),
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX prompts_project_idx ON prompts (project_id, list);

CREATE TABLE prompt_versions (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    prompt_id uuid NOT NULL REFERENCES prompts ON DELETE CASCADE,
    text text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX prompt_versions_prompt_idx ON prompt_versions (prompt_id, created_at DESC);

CREATE TABLE scan_runs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects ON DELETE CASCADE,
    status text NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'running', 'complete', 'partial', 'failed', 'cap_reached')),
    prompt_count integer NOT NULL DEFAULT 0,
    platform_count integer NOT NULL DEFAULT 0,
    cost_cents numeric(12, 4) NOT NULL DEFAULT 0,
    started_at timestamptz,
    finished_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX scan_runs_project_idx ON scan_runs (project_id, created_at DESC);

-- One provider call for one prompt on one platform.
CREATE TABLE scan_calls (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id uuid NOT NULL REFERENCES scan_runs ON DELETE CASCADE,
    prompt_id uuid NOT NULL REFERENCES prompts ON DELETE CASCADE,
    platform text NOT NULL,
    backend_id uuid REFERENCES engine_backends ON DELETE SET NULL,
    status text NOT NULL DEFAULT 'queued',
    attempts integer NOT NULL DEFAULT 0,
    error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    finished_at timestamptz
);
CREATE INDEX scan_calls_run_idx ON scan_calls (run_id);

CREATE TABLE answers (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id uuid NOT NULL REFERENCES scan_runs ON DELETE CASCADE,
    prompt_id uuid NOT NULL REFERENCES prompts ON DELETE CASCADE,
    platform text NOT NULL,
    observed_at timestamptz NOT NULL DEFAULT now(),
    text text NOT NULL,
    assessed boolean NOT NULL DEFAULT false,
    brand_mentioned boolean,
    brand_recommended boolean,
    own_domain_cited boolean,
    position integer,
    sentiment text,
    raw jsonb
);
CREATE INDEX answers_prompt_idx ON answers (prompt_id, observed_at DESC);
CREATE INDEX answers_run_idx ON answers (run_id);

CREATE TABLE answer_brands (
    answer_id uuid NOT NULL REFERENCES answers ON DELETE CASCADE,
    name text NOT NULL,
    owned boolean NOT NULL DEFAULT false,
    recommended boolean NOT NULL DEFAULT false,
    list_rank integer,
    sentiment text,
    PRIMARY KEY (answer_id, name)
);

CREATE TABLE answer_citations (
    id bigserial PRIMARY KEY,
    answer_id uuid NOT NULL REFERENCES answers ON DELETE CASCADE,
    url text NOT NULL,
    domain text NOT NULL,
    title text
);
CREATE INDEX answer_citations_answer_idx ON answer_citations (answer_id);
