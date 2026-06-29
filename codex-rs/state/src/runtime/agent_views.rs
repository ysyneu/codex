use super::*;
use crate::AgentViewThread;
use crate::AgentViewThreadPatch;

impl StateRuntime {
    pub async fn upsert_agent_view(
        &self,
        scope_key: &str,
        codex_home: &Path,
        cwd: &Path,
    ) -> anyhow::Result<()> {
        let now_ms = datetime_to_epoch_millis(Utc::now());
        sqlx::query(
            r#"
INSERT INTO agent_views (
    scope_key,
    codex_home,
    cwd,
    created_at_ms,
    updated_at_ms
) VALUES (?, ?, ?, ?, ?)
ON CONFLICT(scope_key) DO UPDATE SET
    codex_home = excluded.codex_home,
    cwd = excluded.cwd,
    updated_at_ms = excluded.updated_at_ms
            "#,
        )
        .bind(scope_key)
        .bind(codex_home.to_string_lossy().to_string())
        .bind(cwd.to_string_lossy().to_string())
        .bind(now_ms)
        .bind(now_ms)
        .execute(self.pool.as_ref())
        .await?;
        Ok(())
    }

    pub async fn attach_agent_view_thread(
        &self,
        scope_key: &str,
        thread_id: ThreadId,
        initial_prompt: &str,
    ) -> anyhow::Result<()> {
        let now_ms = datetime_to_epoch_millis(Utc::now());
        let position = self.next_agent_view_thread_position(scope_key).await?;
        sqlx::query(
            r#"
INSERT INTO agent_view_threads (
    scope_key,
    thread_id,
    initial_prompt,
    position,
    created_at_ms,
    updated_at_ms
) VALUES (?, ?, ?, ?, ?, ?)
ON CONFLICT(scope_key, thread_id) DO UPDATE SET
    initial_prompt = excluded.initial_prompt,
    updated_at_ms = excluded.updated_at_ms
            "#,
        )
        .bind(scope_key)
        .bind(thread_id.to_string())
        .bind(initial_prompt)
        .bind(position)
        .bind(now_ms)
        .bind(now_ms)
        .execute(self.pool.as_ref())
        .await?;
        Ok(())
    }

    pub async fn list_agent_view_threads(
        &self,
        scope_key: &str,
        include_hidden: bool,
    ) -> anyhow::Result<Vec<AgentViewThread>> {
        let rows = if include_hidden {
            sqlx::query(
                r#"
SELECT
    scope_key,
    thread_id,
    initial_prompt,
    title_override,
    view_state,
    pinned,
    position,
    hidden_at_ms,
    created_at_ms,
    updated_at_ms,
    last_opened_at_ms
FROM agent_view_threads
WHERE scope_key = ?
ORDER BY pinned DESC, position ASC, updated_at_ms DESC, thread_id ASC
            "#,
            )
            .bind(scope_key)
            .fetch_all(self.pool.as_ref())
            .await?
        } else {
            sqlx::query(
                r#"
SELECT
    scope_key,
    thread_id,
    initial_prompt,
    title_override,
    view_state,
    pinned,
    position,
    hidden_at_ms,
    created_at_ms,
    updated_at_ms,
    last_opened_at_ms
FROM agent_view_threads
WHERE scope_key = ? AND hidden_at_ms IS NULL
ORDER BY pinned DESC, position ASC, updated_at_ms DESC, thread_id ASC
            "#,
            )
            .bind(scope_key)
            .fetch_all(self.pool.as_ref())
            .await?
        };

        rows.into_iter()
            .map(|row| {
                crate::model::AgentViewThreadRow::try_from_row(&row)
                    .and_then(AgentViewThread::try_from)
            })
            .collect()
    }

    pub async fn update_agent_view_thread(
        &self,
        scope_key: &str,
        thread_id: ThreadId,
        patch: AgentViewThreadPatch,
    ) -> anyhow::Result<()> {
        let now_ms = datetime_to_epoch_millis(Utc::now());
        let existing = self
            .get_agent_view_thread(scope_key, thread_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("agent view thread not found: {thread_id}"))?;
        let view_state = patch.view_state.unwrap_or(existing.view_state);
        let pinned = patch.pinned.unwrap_or(existing.pinned);
        let position = patch.position.unwrap_or(existing.position);
        let title_override = patch.title_override.unwrap_or(existing.title_override);

        sqlx::query(
            r#"
UPDATE agent_view_threads
SET
    view_state = ?,
    pinned = ?,
    position = ?,
    title_override = ?,
    updated_at_ms = ?
WHERE scope_key = ? AND thread_id = ?
            "#,
        )
        .bind(view_state.as_str())
        .bind(i64::from(pinned))
        .bind(position)
        .bind(title_override)
        .bind(now_ms)
        .bind(scope_key)
        .bind(thread_id.to_string())
        .execute(self.pool.as_ref())
        .await?;
        Ok(())
    }

    pub async fn hide_agent_view_thread(
        &self,
        scope_key: &str,
        thread_id: ThreadId,
    ) -> anyhow::Result<()> {
        let now_ms = datetime_to_epoch_millis(Utc::now());
        sqlx::query(
            r#"
UPDATE agent_view_threads
SET hidden_at_ms = ?, updated_at_ms = ?
WHERE scope_key = ? AND thread_id = ?
            "#,
        )
        .bind(now_ms)
        .bind(now_ms)
        .bind(scope_key)
        .bind(thread_id.to_string())
        .execute(self.pool.as_ref())
        .await?;
        Ok(())
    }

    async fn get_agent_view_thread(
        &self,
        scope_key: &str,
        thread_id: ThreadId,
    ) -> anyhow::Result<Option<AgentViewThread>> {
        let row = sqlx::query(
            r#"
SELECT
    scope_key,
    thread_id,
    initial_prompt,
    title_override,
    view_state,
    pinned,
    position,
    hidden_at_ms,
    created_at_ms,
    updated_at_ms,
    last_opened_at_ms
FROM agent_view_threads
WHERE scope_key = ? AND thread_id = ?
            "#,
        )
        .bind(scope_key)
        .bind(thread_id.to_string())
        .fetch_optional(self.pool.as_ref())
        .await?;

        row.map(|row| {
            crate::model::AgentViewThreadRow::try_from_row(&row).and_then(AgentViewThread::try_from)
        })
        .transpose()
    }

    async fn next_agent_view_thread_position(&self, scope_key: &str) -> anyhow::Result<i64> {
        let position = sqlx::query_scalar::<_, Option<i64>>(
            r#"
SELECT MAX(position)
FROM agent_view_threads
WHERE scope_key = ?
            "#,
        )
        .bind(scope_key)
        .fetch_one(self.pool.as_ref())
        .await?
        .map_or(0, |position| position + 1);
        Ok(position)
    }
}
