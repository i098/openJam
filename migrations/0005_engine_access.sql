-- Engine access and judge state: account rotation, blocks, plan sign-ins, answer attribution.
-- Extends the 0001 tables. `engine_accounts.credential_ciphertext` holds one sealed secret:
-- an API key (mode key), or the three tokens of a ChatGPT sign-in as JSON (mode plan, OpenAI).
-- A Claude plan account holds no secret: `account_ref` names its folder in the runner.

ALTER TABLE engine_backends
    ADD COLUMN plan_model text,
    -- `rotate`: a limit moves the prompt to the next account. `standby`: it never does.
    ADD COLUMN rotation text NOT NULL DEFAULT 'rotate' CHECK (rotation IN ('rotate', 'standby')),
    -- Plan accounts blocked or failed: use the key accounts.
    ADD COLUMN key_fallback boolean NOT NULL DEFAULT true;

ALTER TABLE engine_accounts
    ADD COLUMN priority integer NOT NULL DEFAULT 0,
    ADD COLUMN last_used_at timestamptz,
    ADD COLUMN disabled_cause text,
    ADD COLUMN identity_email text,
    ADD COLUMN expires_at timestamptz,
    ADD COLUMN earliest_refresh_at timestamptz,
    ADD COLUMN refresh_retry_at timestamptz,
    ADD COLUMN background_consent_at timestamptz;
CREATE INDEX engine_accounts_pick_idx ON engine_accounts (backend_id, mode, enabled);

-- The only place a usage limit lives. `scope` is '' for the whole account or 'model:<id>'.
CREATE TABLE engine_account_blocks (
    account_id uuid NOT NULL REFERENCES engine_accounts ON DELETE CASCADE,
    scope text NOT NULL,
    blocked_until timestamptz NOT NULL,
    backoff_level smallint NOT NULL DEFAULT 0 CHECK (backoff_level BETWEEN 0 AND 8),
    reason text NOT NULL CHECK (reason IN ('usage_limit', 'rate_limit', 'unavailable')),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id, scope)
);
CREATE INDEX engine_account_blocks_until_idx ON engine_account_blocks (blocked_until);

-- One refresh at a time for each account, across worker processes.
CREATE TABLE engine_account_refresh_leases (
    account_id uuid PRIMARY KEY REFERENCES engine_accounts ON DELETE CASCADE,
    owner text NOT NULL,
    expires_at timestamptz NOT NULL
);

-- Sign in with ChatGPT in progress. The row is deleted when the flow ends.
CREATE TABLE engine_login_flows (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id uuid NOT NULL REFERENCES workspaces ON DELETE CASCADE,
    label text NOT NULL,
    state text NOT NULL UNIQUE,
    nonce text NOT NULL,
    code_verifier text NOT NULL,
    redirect_uri text NOT NULL,
    -- The client id issued to a first registration comes back in the callback; a re-sign-in has it.
    client_id text NOT NULL,
    background_consent boolean NOT NULL DEFAULT false,
    expires_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- The id that names this host to OpenAI. Made once, never changed.
CREATE TABLE engine_settings (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    ext_agent_host_id text NOT NULL DEFAULT 'urn:uuid:' || gen_random_uuid(),
    created_at timestamptz NOT NULL DEFAULT now()
);
INSERT INTO engine_settings DEFAULT VALUES;

-- Which path gave an answer. A plan answer is an API-model answer on the owner's plan.
ALTER TABLE answers
    ADD COLUMN route text NOT NULL DEFAULT 'api_key' CHECK (route IN ('api_key', 'plan')),
    ADD COLUMN model text,
    ADD COLUMN account_id uuid REFERENCES engine_accounts ON DELETE SET NULL,
    ADD COLUMN call_id uuid REFERENCES scan_calls ON DELETE SET NULL;
CREATE UNIQUE INDEX answers_call_key ON answers (call_id) WHERE call_id IS NOT NULL;

-- A call that waits for a plan limit to end.
ALTER TABLE scan_calls
    ADD COLUMN run_after timestamptz,
    ADD COLUMN error_code text;

-- Cadence: every other day joins daily and weekly.
ALTER TABLE projects DROP CONSTRAINT projects_cadence_check;
ALTER TABLE projects ADD CONSTRAINT projects_cadence_check
    CHECK (cadence IN ('daily', 'every_other_day', 'weekly'));

-- The judge. One row. `unchecked` runs the small and the big model on each answer until 200
-- answers agree or disagree enough to choose `small_first` or `big_only`.
CREATE TABLE judge_state (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    mode text NOT NULL DEFAULT 'unchecked' CHECK (mode IN ('unchecked', 'small_first', 'big_only')),
    samples integer NOT NULL DEFAULT 0,
    agreed integer NOT NULL DEFAULT 0,
    decided_at timestamptz
);
INSERT INTO judge_state DEFAULT VALUES;
