use super::resolve_request_cwd;
use super::thread_from_stored_thread;
use crate::error_code::internal_error;
use crate::error_code::invalid_params;
use crate::thread_status::ThreadWatchManager;
use codex_app_server_protocol::AgentViewAttachThreadParams;
use codex_app_server_protocol::AgentViewAttachThreadResponse;
use codex_app_server_protocol::AgentViewEntry;
use codex_app_server_protocol::AgentViewHideEntryParams;
use codex_app_server_protocol::AgentViewHideEntryResponse;
use codex_app_server_protocol::AgentViewListParams;
use codex_app_server_protocol::AgentViewListResponse;
use codex_app_server_protocol::AgentViewUpdateEntryParams;
use codex_app_server_protocol::AgentViewUpdateEntryResponse;
use codex_app_server_protocol::AgentViewWorkflowState;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::Thread;
use codex_core::config::Config;
use codex_protocol::ThreadId;
use codex_rollout::StateDbHandle;
use codex_state::AgentViewState;
use codex_state::AgentViewThread;
use codex_state::AgentViewThreadPatch;
use codex_thread_store::ReadThreadParams as StoreReadThreadParams;
use codex_thread_store::ThreadStore;
use codex_thread_store::ThreadStoreError;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::path::PathBuf;
use std::sync::Arc;

pub(crate) struct AgentViewRequestProcessor {
    config: Arc<Config>,
    state_db: Option<StateDbHandle>,
    thread_store: Arc<dyn ThreadStore>,
    thread_watch_manager: ThreadWatchManager,
}

struct AgentViewScope {
    key: String,
}

impl AgentViewRequestProcessor {
    pub(crate) fn new(
        config: Arc<Config>,
        state_db: Option<StateDbHandle>,
        thread_store: Arc<dyn ThreadStore>,
        thread_watch_manager: ThreadWatchManager,
    ) -> Self {
        Self {
            config,
            state_db,
            thread_store,
            thread_watch_manager,
        }
    }

    pub(crate) async fn list(
        &self,
        params: AgentViewListParams,
    ) -> Result<AgentViewListResponse, JSONRPCErrorError> {
        let scope = self.ensure_scope(params.cwd).await?;
        let state_db = self.state_db()?;
        let rows = state_db
            .list_agent_view_threads(&scope.key, params.include_hidden)
            .await
            .map_err(|err| internal_error(format!("failed to list agent view entries: {err}")))?;
        let entries = self.entries_from_rows(rows).await?;
        Ok(AgentViewListResponse {
            scope_key: scope.key,
            entries,
        })
    }

    pub(crate) async fn attach_thread(
        &self,
        params: AgentViewAttachThreadParams,
    ) -> Result<AgentViewAttachThreadResponse, JSONRPCErrorError> {
        let scope = self.ensure_scope(params.cwd).await?;
        let thread_id = parse_thread_id(&params.thread_id)?;
        let state_db = self.state_db()?;
        state_db
            .attach_agent_view_thread(&scope.key, thread_id, &params.initial_prompt)
            .await
            .map_err(|err| internal_error(format!("failed to attach agent view thread: {err}")))?;
        let entry = self.entry_for_thread(&scope.key, &params.thread_id).await?;
        Ok(AgentViewAttachThreadResponse {
            scope_key: scope.key,
            entry,
        })
    }

    pub(crate) async fn update_entry(
        &self,
        params: AgentViewUpdateEntryParams,
    ) -> Result<AgentViewUpdateEntryResponse, JSONRPCErrorError> {
        let scope = self.ensure_scope(params.cwd).await?;
        let thread_id = parse_thread_id(&params.thread_id)?;
        let state_db = self.state_db()?;
        state_db
            .update_agent_view_thread(
                &scope.key,
                thread_id,
                AgentViewThreadPatch {
                    view_state: params.view_state.map(agent_view_state_from_api),
                    pinned: params.pinned,
                    position: params.position,
                    title_override: params.title_override,
                },
            )
            .await
            .map_err(|err| internal_error(format!("failed to update agent view entry: {err}")))?;
        let entry = self.entry_for_thread(&scope.key, &params.thread_id).await?;
        Ok(AgentViewUpdateEntryResponse {
            scope_key: scope.key,
            entry,
        })
    }

    pub(crate) async fn hide_entry(
        &self,
        params: AgentViewHideEntryParams,
    ) -> Result<AgentViewHideEntryResponse, JSONRPCErrorError> {
        let scope = self.ensure_scope(params.cwd).await?;
        let thread_id = parse_thread_id(&params.thread_id)?;
        let state_db = self.state_db()?;
        state_db
            .hide_agent_view_thread(&scope.key, thread_id)
            .await
            .map_err(|err| internal_error(format!("failed to hide agent view entry: {err}")))?;
        Ok(AgentViewHideEntryResponse {
            scope_key: scope.key,
        })
    }

    fn state_db(&self) -> Result<&StateDbHandle, JSONRPCErrorError> {
        self.state_db
            .as_ref()
            .ok_or_else(|| internal_error("state db is unavailable"))
    }

    async fn ensure_scope(&self, cwd: String) -> Result<AgentViewScope, JSONRPCErrorError> {
        let cwd = resolve_request_cwd(Some(PathBuf::from(cwd)))?
            .ok_or_else(|| invalid_params("agent view cwd is required"))?;
        let key = self.scope_key_for_cwd(&cwd);
        self.state_db()?
            .upsert_agent_view(&key, self.config.codex_home.as_path(), cwd.as_path())
            .await
            .map_err(|err| internal_error(format!("failed to upsert agent view scope: {err}")))?;
        Ok(AgentViewScope { key })
    }

    fn scope_key_for_cwd(&self, cwd: &AbsolutePathBuf) -> String {
        format!(
            "codex_home={}\ncwd={}",
            self.config.codex_home.display(),
            cwd.display()
        )
    }

    async fn entry_for_thread(
        &self,
        scope_key: &str,
        thread_id: &str,
    ) -> Result<AgentViewEntry, JSONRPCErrorError> {
        let rows = self
            .state_db()?
            .list_agent_view_threads(scope_key, /*include_hidden*/ true)
            .await
            .map_err(|err| internal_error(format!("failed to read agent view entry: {err}")))?;
        let row = rows
            .into_iter()
            .find(|row| row.thread_id.to_string() == thread_id)
            .ok_or_else(|| invalid_params(format!("agent view entry not found: {thread_id}")))?;
        self.entry_from_row(row).await
    }

    async fn entries_from_rows(
        &self,
        rows: Vec<AgentViewThread>,
    ) -> Result<Vec<AgentViewEntry>, JSONRPCErrorError> {
        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            entries.push(self.entry_from_row(row).await?);
        }
        Ok(entries)
    }

    async fn entry_from_row(
        &self,
        row: AgentViewThread,
    ) -> Result<AgentViewEntry, JSONRPCErrorError> {
        let thread = self.read_thread(row.thread_id).await?;
        Ok(AgentViewEntry {
            thread_id: row.thread_id.to_string(),
            initial_prompt: row.initial_prompt,
            title_override: row.title_override,
            view_state: agent_view_state_to_api(row.view_state),
            pinned: row.pinned,
            position: row.position,
            hidden: row.hidden_at.is_some(),
            created_at: row.created_at.timestamp(),
            updated_at: row.updated_at.timestamp(),
            last_opened_at: row.last_opened_at.map(|ts| ts.timestamp()),
            thread,
        })
    }

    async fn read_thread(&self, thread_id: ThreadId) -> Result<Option<Thread>, JSONRPCErrorError> {
        match self
            .thread_store
            .read_thread(StoreReadThreadParams {
                thread_id,
                include_archived: true,
                include_history: false,
            })
            .await
        {
            Ok(stored_thread) => {
                let (mut thread, _) = thread_from_stored_thread(
                    stored_thread,
                    self.config.model_provider_id.as_str(),
                    &self.config.cwd,
                );
                let statuses = self
                    .thread_watch_manager
                    .loaded_statuses_for_threads(vec![thread.id.clone()])
                    .await;
                if let Some(status) = statuses.get(&thread.id) {
                    thread.status = status.clone();
                }
                Ok(Some(thread))
            }
            Err(ThreadStoreError::ThreadNotFound {
                thread_id: missing_thread_id,
            }) if missing_thread_id == thread_id => Ok(None),
            Err(ThreadStoreError::InvalidRequest { message })
                if message == format!("no rollout found for thread id {thread_id}") =>
            {
                Ok(None)
            }
            Err(ThreadStoreError::InvalidRequest { message }) => Err(invalid_params(message)),
            Err(ThreadStoreError::Unsupported { operation }) => Err(internal_error(format!(
                "thread store does not support {operation}"
            ))),
            Err(err) => Err(internal_error(format!("failed to read thread: {err}"))),
        }
    }
}

fn parse_thread_id(thread_id: &str) -> Result<ThreadId, JSONRPCErrorError> {
    ThreadId::from_string(thread_id)
        .map_err(|err| invalid_params(format!("invalid thread id `{thread_id}`: {err}")))
}

fn agent_view_state_to_api(state: AgentViewState) -> AgentViewWorkflowState {
    match state {
        AgentViewState::ReadyForReview => AgentViewWorkflowState::ReadyForReview,
        AgentViewState::Completed => AgentViewWorkflowState::Completed,
    }
}

fn agent_view_state_from_api(state: AgentViewWorkflowState) -> AgentViewState {
    match state {
        AgentViewWorkflowState::ReadyForReview => AgentViewState::ReadyForReview,
        AgentViewWorkflowState::Completed => AgentViewState::Completed,
    }
}
