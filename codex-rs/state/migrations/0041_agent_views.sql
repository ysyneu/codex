CREATE TABLE agent_views (
  scope_key TEXT PRIMARY KEY,
  codex_home TEXT NOT NULL,
  cwd TEXT NOT NULL,
  created_at_ms INTEGER NOT NULL,
  updated_at_ms INTEGER NOT NULL
);

CREATE TABLE agent_view_threads (
  scope_key TEXT NOT NULL,
  thread_id TEXT NOT NULL,
  initial_prompt TEXT NOT NULL DEFAULT '',
  title_override TEXT,
  view_state TEXT NOT NULL DEFAULT 'ready_for_review',
  pinned INTEGER NOT NULL DEFAULT 0,
  position INTEGER NOT NULL DEFAULT 0,
  hidden_at_ms INTEGER,
  created_at_ms INTEGER NOT NULL,
  updated_at_ms INTEGER NOT NULL,
  last_opened_at_ms INTEGER,
  PRIMARY KEY (scope_key, thread_id),
  FOREIGN KEY (scope_key) REFERENCES agent_views(scope_key) ON DELETE CASCADE
);

CREATE INDEX idx_agent_view_threads_visible
ON agent_view_threads(scope_key, hidden_at_ms, pinned, position, updated_at_ms);
