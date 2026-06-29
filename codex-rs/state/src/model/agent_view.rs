use anyhow::Context;
use anyhow::Result;
use chrono::DateTime;
use chrono::Utc;
use codex_protocol::ThreadId;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;
use std::str::FromStr;

use super::epoch_millis_to_datetime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentViewState {
    ReadyForReview,
    Completed,
}

impl AgentViewState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadyForReview => "ready_for_review",
            Self::Completed => "completed",
        }
    }
}

impl FromStr for AgentViewState {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "ready_for_review" => Ok(Self::ReadyForReview),
            "completed" => Ok(Self::Completed),
            _ => anyhow::bail!("unknown agent view state: {value}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentViewThread {
    pub scope_key: String,
    pub thread_id: ThreadId,
    pub initial_prompt: String,
    pub title_override: Option<String>,
    pub view_state: AgentViewState,
    pub pinned: bool,
    pub position: i64,
    pub hidden_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_opened_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentViewThreadPatch {
    pub view_state: Option<AgentViewState>,
    pub pinned: Option<bool>,
    pub position: Option<i64>,
    pub title_override: Option<Option<String>>,
}

pub(crate) struct AgentViewThreadRow {
    pub scope_key: String,
    pub thread_id: String,
    pub initial_prompt: String,
    pub title_override: Option<String>,
    pub view_state: String,
    pub pinned: i64,
    pub position: i64,
    pub hidden_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub last_opened_at_ms: Option<i64>,
}

impl AgentViewThreadRow {
    pub(crate) fn try_from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            scope_key: row.try_get("scope_key")?,
            thread_id: row.try_get("thread_id")?,
            initial_prompt: row.try_get("initial_prompt")?,
            title_override: row.try_get("title_override")?,
            view_state: row.try_get("view_state")?,
            pinned: row.try_get("pinned")?,
            position: row.try_get("position")?,
            hidden_at_ms: row.try_get("hidden_at_ms")?,
            created_at_ms: row.try_get("created_at_ms")?,
            updated_at_ms: row.try_get("updated_at_ms")?,
            last_opened_at_ms: row.try_get("last_opened_at_ms")?,
        })
    }
}

impl TryFrom<AgentViewThreadRow> for AgentViewThread {
    type Error = anyhow::Error;

    fn try_from(row: AgentViewThreadRow) -> Result<Self> {
        Ok(Self {
            scope_key: row.scope_key,
            thread_id: ThreadId::from_string(&row.thread_id)
                .with_context(|| format!("invalid agent view thread id {}", row.thread_id))?,
            initial_prompt: row.initial_prompt,
            title_override: row.title_override,
            view_state: AgentViewState::from_str(&row.view_state)?,
            pinned: row.pinned != 0,
            position: row.position,
            hidden_at: row.hidden_at_ms.map(epoch_millis_to_datetime).transpose()?,
            created_at: epoch_millis_to_datetime(row.created_at_ms)?,
            updated_at: epoch_millis_to_datetime(row.updated_at_ms)?,
            last_opened_at: row
                .last_opened_at_ms
                .map(epoch_millis_to_datetime)
                .transpose()?,
        })
    }
}
