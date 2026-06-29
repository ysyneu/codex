use anyhow::Result;
use app_test_support::TestAppServer;
use app_test_support::create_mock_responses_server_repeating_assistant;
use app_test_support::to_response;
use codex_app_server_protocol::AgentViewAttachThreadParams;
use codex_app_server_protocol::AgentViewAttachThreadResponse;
use codex_app_server_protocol::AgentViewHideEntryParams;
use codex_app_server_protocol::AgentViewHideEntryResponse;
use codex_app_server_protocol::AgentViewListParams;
use codex_app_server_protocol::AgentViewListResponse;
use codex_app_server_protocol::AgentViewUpdateEntryParams;
use codex_app_server_protocol::AgentViewUpdateEntryResponse;
use codex_app_server_protocol::AgentViewWorkflowState;
use codex_app_server_protocol::JSONRPCResponse;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadReadParams;
use codex_app_server_protocol::ThreadReadResponse;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use pretty_assertions::assert_eq;
use serde::de::DeserializeOwned;
use std::path::Path;
use tempfile::TempDir;
use tokio::time::timeout;

const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

async fn init_mcp(codex_home: &Path) -> Result<TestAppServer> {
    let mut mcp = TestAppServer::new(codex_home).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;
    Ok(mcp)
}

async fn read_response<T>(mcp: &mut TestAppServer, request_id: i64) -> Result<T>
where
    T: DeserializeOwned,
{
    let response: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;
    to_response::<T>(response)
}

fn create_runtime_config(codex_home: &Path, server_uri: &str) -> std::io::Result<()> {
    std::fs::write(
        codex_home.join("config.toml"),
        format!(
            r#"
model = "mock-model"
approval_policy = "never"
sandbox_mode = "read-only"

model_provider = "mock_provider"

[model_providers.mock_provider]
name = "Mock provider for test"
base_url = "{server_uri}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
"#
        ),
    )
}

#[tokio::test]
async fn agent_view_lists_only_attached_threads_and_hides_without_deleting() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_runtime_config(codex_home.path(), &server.uri())?;
    let cwd = codex_home.path().join("workspace");
    std::fs::create_dir_all(&cwd)?;
    let cwd = cwd.display().to_string();
    let mut mcp = init_mcp(codex_home.path()).await?;

    let list_id = mcp
        .send_agent_view_list_request(AgentViewListParams {
            cwd: cwd.clone(),
            include_hidden: false,
        })
        .await?;
    let empty_list: AgentViewListResponse = read_response(&mut mcp, list_id).await?;
    assert!(empty_list.entries.is_empty());
    assert!(!empty_list.scope_key.is_empty());

    let start_id = mcp
        .send_thread_start_request(ThreadStartParams {
            cwd: Some(cwd.clone()),
            model: Some("mock-model".to_string()),
            ..Default::default()
        })
        .await?;
    let ThreadStartResponse { thread, .. } = read_response(&mut mcp, start_id).await?;

    let list_id = mcp
        .send_agent_view_list_request(AgentViewListParams {
            cwd: cwd.clone(),
            include_hidden: false,
        })
        .await?;
    let list_after_plain_thread: AgentViewListResponse = read_response(&mut mcp, list_id).await?;
    assert!(
        list_after_plain_thread.entries.is_empty(),
        "plain codex sessions must not appear until this view attaches them"
    );

    let attach_id = mcp
        .send_agent_view_attach_thread_request(AgentViewAttachThreadParams {
            cwd: cwd.clone(),
            thread_id: thread.id.clone(),
            initial_prompt: "audit browser sandbox".to_string(),
        })
        .await?;
    let attached: AgentViewAttachThreadResponse = read_response(&mut mcp, attach_id).await?;
    assert_eq!(attached.scope_key, empty_list.scope_key);
    assert_eq!(attached.entry.thread_id, thread.id);
    assert_eq!(attached.entry.initial_prompt, "audit browser sandbox");
    assert_eq!(
        attached.entry.view_state,
        AgentViewWorkflowState::ReadyForReview
    );
    assert!(
        attached.entry.thread.is_none(),
        "fresh threads can be attached before their first rollout item is written"
    );

    let update_id = mcp
        .send_agent_view_update_entry_request(AgentViewUpdateEntryParams {
            cwd: cwd.clone(),
            thread_id: thread.id.clone(),
            view_state: Some(AgentViewWorkflowState::Completed),
            pinned: Some(true),
            position: Some(4),
            title_override: Some(Some("browser sandbox mechanism audit".to_string())),
        })
        .await?;
    let updated: AgentViewUpdateEntryResponse = read_response(&mut mcp, update_id).await?;
    assert!(updated.entry.pinned);
    assert_eq!(updated.entry.position, 4);
    assert_eq!(updated.entry.view_state, AgentViewWorkflowState::Completed);
    assert_eq!(
        updated.entry.title_override.as_deref(),
        Some("browser sandbox mechanism audit")
    );

    let hide_id = mcp
        .send_agent_view_hide_entry_request(AgentViewHideEntryParams {
            cwd: cwd.clone(),
            thread_id: thread.id.clone(),
        })
        .await?;
    let _: AgentViewHideEntryResponse = read_response(&mut mcp, hide_id).await?;

    let list_id = mcp
        .send_agent_view_list_request(AgentViewListParams {
            cwd: cwd.clone(),
            include_hidden: false,
        })
        .await?;
    let visible_after_hide: AgentViewListResponse = read_response(&mut mcp, list_id).await?;
    assert!(visible_after_hide.entries.is_empty());

    let list_id = mcp
        .send_agent_view_list_request(AgentViewListParams {
            cwd: cwd.clone(),
            include_hidden: true,
        })
        .await?;
    let hidden_entries: AgentViewListResponse = read_response(&mut mcp, list_id).await?;
    assert_eq!(hidden_entries.entries.len(), 1);
    assert!(hidden_entries.entries[0].hidden);

    let read_id = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: thread.id.clone(),
            include_turns: false,
        })
        .await?;
    let read: ThreadReadResponse = read_response(&mut mcp, read_id).await?;
    assert_eq!(read.thread.id, thread.id);
    Ok(())
}
