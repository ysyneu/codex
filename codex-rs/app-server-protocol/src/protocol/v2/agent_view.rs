use super::Thread;
use codex_experimental_api_macros::ExperimentalApi;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase", export_to = "v2/")]
pub enum AgentViewWorkflowState {
    ReadyForReview,
    Completed,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS, ExperimentalApi)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentViewListParams {
    pub cwd: String,
    #[serde(default)]
    pub include_hidden: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS, ExperimentalApi)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentViewListResponse {
    pub scope_key: String,
    pub entries: Vec<AgentViewEntry>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS, ExperimentalApi)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentViewEntry {
    pub thread_id: String,
    pub initial_prompt: String,
    #[ts(optional = nullable)]
    pub title_override: Option<String>,
    pub view_state: AgentViewWorkflowState,
    pub pinned: bool,
    pub position: i64,
    pub hidden: bool,
    pub created_at: i64,
    pub updated_at: i64,
    #[ts(optional = nullable)]
    pub last_opened_at: Option<i64>,
    #[ts(optional = nullable)]
    pub thread: Option<Thread>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS, ExperimentalApi)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentViewAttachThreadParams {
    pub cwd: String,
    pub thread_id: String,
    pub initial_prompt: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS, ExperimentalApi)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentViewAttachThreadResponse {
    pub scope_key: String,
    pub entry: AgentViewEntry,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS, ExperimentalApi)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentViewUpdateEntryParams {
    pub cwd: String,
    pub thread_id: String,
    #[ts(optional = nullable)]
    pub view_state: Option<AgentViewWorkflowState>,
    #[ts(optional = nullable)]
    pub pinned: Option<bool>,
    #[ts(optional = nullable)]
    pub position: Option<i64>,
    #[serde(
        default,
        deserialize_with = "crate::protocol::serde_helpers::deserialize_double_option",
        serialize_with = "crate::protocol::serde_helpers::serialize_double_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(optional = nullable)]
    pub title_override: Option<Option<String>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS, ExperimentalApi)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentViewUpdateEntryResponse {
    pub scope_key: String,
    pub entry: AgentViewEntry,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS, ExperimentalApi)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentViewHideEntryParams {
    pub cwd: String,
    pub thread_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS, ExperimentalApi)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentViewHideEntryResponse {
    pub scope_key: String,
}
