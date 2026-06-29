use anyhow::Result;
use codex_protocol::ThreadId;
use pretty_assertions::assert_eq;

use super::StateRuntime;
use super::test_support::test_thread_metadata;
use super::test_support::unique_temp_dir;
use crate::AgentViewState;
use crate::AgentViewThreadPatch;

fn thread_id(suffix: u32) -> ThreadId {
    ThreadId::from_string(&format!("00000000-0000-0000-0000-{suffix:012}"))
        .expect("valid thread id")
}

#[tokio::test]
async fn agent_view_threads_are_scoped_by_view() -> Result<()> {
    let codex_home = unique_temp_dir();
    let cwd_a = codex_home.join("repo-a");
    let cwd_b = codex_home.join("repo-b");
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string()).await?;

    runtime
        .upsert_agent_view("scope-a", codex_home.as_path(), cwd_a.as_path())
        .await?;
    runtime
        .upsert_agent_view("scope-b", codex_home.as_path(), cwd_b.as_path())
        .await?;
    runtime
        .attach_agent_view_thread("scope-a", thread_id(1), "audit browser sandbox")
        .await?;
    runtime
        .attach_agent_view_thread("scope-b", thread_id(2), "refactor event pagination")
        .await?;

    let scope_a_rows = runtime
        .list_agent_view_threads("scope-a", /*include_hidden*/ false)
        .await?;
    let scope_b_rows = runtime
        .list_agent_view_threads("scope-b", /*include_hidden*/ false)
        .await?;

    assert_eq!(scope_a_rows.len(), 1);
    assert_eq!(scope_a_rows[0].thread_id, thread_id(1));
    assert_eq!(scope_a_rows[0].initial_prompt, "audit browser sandbox");
    assert_eq!(scope_b_rows.len(), 1);
    assert_eq!(scope_b_rows[0].thread_id, thread_id(2));
    assert_eq!(scope_b_rows[0].initial_prompt, "refactor event pagination");

    runtime
        .attach_agent_view_thread("scope-a", thread_id(9), "same thread in one scope")
        .await?;
    let scope_b_rows = runtime
        .list_agent_view_threads("scope-b", /*include_hidden*/ false)
        .await?;
    assert!(!scope_b_rows.iter().any(|row| row.thread_id == thread_id(9)));
    Ok(())
}

#[tokio::test]
async fn agent_view_updates_pin_state_and_workflow_state() -> Result<()> {
    let codex_home = unique_temp_dir();
    let cwd = codex_home.join("repo");
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string()).await?;
    let thread_id = thread_id(3);

    runtime
        .upsert_agent_view("scope", codex_home.as_path(), cwd.as_path())
        .await?;
    runtime
        .attach_agent_view_thread("scope", thread_id, "implement payment integration")
        .await?;
    runtime
        .update_agent_view_thread(
            "scope",
            thread_id,
            AgentViewThreadPatch {
                view_state: Some(AgentViewState::Completed),
                pinned: Some(true),
                position: Some(7),
                title_override: Some(Some("stripe payment integration".to_string())),
            },
        )
        .await?;

    let rows = runtime
        .list_agent_view_threads("scope", /*include_hidden*/ false)
        .await?;

    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.thread_id, thread_id);
    assert_eq!(row.view_state, AgentViewState::Completed);
    assert!(row.pinned);
    assert_eq!(row.position, 7);
    assert_eq!(
        row.title_override.as_deref(),
        Some("stripe payment integration")
    );
    Ok(())
}

#[tokio::test]
async fn agent_view_patch_preserves_unspecified_fields_and_clears_title() -> Result<()> {
    let codex_home = unique_temp_dir();
    let cwd = codex_home.join("repo");
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string()).await?;
    let thread_id = thread_id(5);

    runtime
        .upsert_agent_view("scope", codex_home.as_path(), cwd.as_path())
        .await?;
    runtime
        .attach_agent_view_thread("scope", thread_id, "make session registry")
        .await?;
    runtime
        .update_agent_view_thread(
            "scope",
            thread_id,
            AgentViewThreadPatch {
                view_state: Some(AgentViewState::Completed),
                pinned: Some(true),
                position: Some(11),
                title_override: Some(Some("session registry".to_string())),
            },
        )
        .await?;
    runtime
        .update_agent_view_thread(
            "scope",
            thread_id,
            AgentViewThreadPatch {
                pinned: Some(false),
                title_override: Some(None),
                ..Default::default()
            },
        )
        .await?;

    let rows = runtime
        .list_agent_view_threads("scope", /*include_hidden*/ false)
        .await?;

    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.view_state, AgentViewState::Completed);
    assert!(!row.pinned);
    assert_eq!(row.position, 11);
    assert_eq!(row.title_override, None);
    Ok(())
}

#[tokio::test]
async fn hiding_agent_view_thread_does_not_remove_the_registry_row() -> Result<()> {
    let codex_home = unique_temp_dir();
    let cwd = codex_home.join("repo");
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string()).await?;
    let thread_id = thread_id(4);

    runtime
        .upsert_agent_view("scope", codex_home.as_path(), cwd.as_path())
        .await?;
    runtime
        .attach_agent_view_thread("scope", thread_id, "oauth flow design")
        .await?;
    runtime.hide_agent_view_thread("scope", thread_id).await?;

    let visible_rows = runtime
        .list_agent_view_threads("scope", /*include_hidden*/ false)
        .await?;
    let all_rows = runtime
        .list_agent_view_threads("scope", /*include_hidden*/ true)
        .await?;

    assert!(visible_rows.is_empty());
    assert_eq!(all_rows.len(), 1);
    assert_eq!(all_rows[0].thread_id, thread_id);
    assert!(all_rows[0].hidden_at.is_some());
    Ok(())
}

#[tokio::test]
async fn hiding_agent_view_thread_does_not_delete_the_thread() -> Result<()> {
    let codex_home = unique_temp_dir();
    let cwd = codex_home.join("repo");
    let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string()).await?;
    let thread_id = thread_id(6);

    runtime
        .upsert_thread(&test_thread_metadata(&codex_home, thread_id, cwd.clone()))
        .await?;
    runtime
        .upsert_agent_view("scope", codex_home.as_path(), cwd.as_path())
        .await?;
    runtime
        .attach_agent_view_thread("scope", thread_id, "keep original session")
        .await?;
    runtime.hide_agent_view_thread("scope", thread_id).await?;

    let thread = runtime
        .get_thread(thread_id)
        .await?
        .expect("thread should still exist after hiding its dashboard entry");
    assert_eq!(thread.id, thread_id);
    assert_eq!(thread.archived_at, None);
    Ok(())
}
