//! Claude-style agents dashboard for `codex agents`.
//!
//! The dashboard is intentionally a thin view over real Codex app-server threads. It never drives
//! the model externally: creating or opening an entry attaches the normal `ChatWidget` to a real
//! thread, so slash commands, mentions, approvals, image paste, and subagents keep their native
//! behavior.

use super::*;
use crate::chatwidget::DashboardComposerInput;
use crate::line_truncation::truncate_line_with_ellipsis_if_overflow;
use codex_app_server_protocol::AgentViewEntry;
use codex_app_server_protocol::AgentViewUpdateEntryParams;
use codex_app_server_protocol::AgentViewWorkflowState;
use codex_app_server_protocol::ThreadActiveFlag;
use codex_app_server_protocol::ThreadStatus;
use crossterm::event::MouseButton;
use crossterm::event::MouseEvent;
use crossterm::event::MouseEventKind;
use ratatui::buffer::Buffer;
use ratatui::layout::Alignment;
use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Span;
use ratatui::widgets::Widget;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentsDashboardScreen {
    List,
    Thread,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentsDashboardGroup {
    Pinned,
    NeedsInput,
    Working,
    ReadyForReview,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RowHitbox {
    y: u16,
    entry_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DashboardRenderRow {
    Group(AgentsDashboardGroup),
    Entry {
        visible_index: usize,
        entry_index: usize,
    },
    Spacer,
}

#[derive(Debug)]
pub(super) struct AgentsDashboardState {
    cwd: PathBuf,
    entries: Vec<AgentViewEntry>,
    selected: usize,
    scroll_offset: usize,
    screen: AgentsDashboardScreen,
    row_hitboxes: Vec<RowHitbox>,
    last_refreshed_at: Option<Instant>,
}

impl AgentsDashboardState {
    pub(super) fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            entries: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            screen: AgentsDashboardScreen::List,
            row_hitboxes: Vec::new(),
            last_refreshed_at: None,
        }
    }

    fn cwd_string(&self) -> String {
        self.cwd.display().to_string()
    }

    fn is_list(&self) -> bool {
        self.screen == AgentsDashboardScreen::List
    }

    fn set_entries(&mut self, entries: Vec<AgentViewEntry>) {
        self.entries = entries;
        self.last_refreshed_at = Some(Instant::now());
        self.clamp_selection();
    }

    fn ordered_entry_indices(&self) -> Vec<usize> {
        let mut indices = Vec::new();
        for group in [
            AgentsDashboardGroup::Pinned,
            AgentsDashboardGroup::NeedsInput,
            AgentsDashboardGroup::Working,
            AgentsDashboardGroup::ReadyForReview,
            AgentsDashboardGroup::Completed,
        ] {
            indices.extend(
                self.entries
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| group_for_entry(entry) == group)
                    .map(|(idx, _)| idx),
            );
        }
        indices
    }

    fn selected_entry_index(&self) -> Option<usize> {
        self.ordered_entry_indices().get(self.selected).copied()
    }

    fn selected_thread_id(&self) -> Option<ThreadId> {
        let entry = self.entries.get(self.selected_entry_index()?)?;
        ThreadId::from_string(&entry.thread_id).ok()
    }

    fn clamp_selection(&mut self) {
        let len = self.ordered_entry_indices().len();
        if len == 0 {
            self.selected = 0;
            self.scroll_offset = 0;
        } else if self.selected >= len {
            self.selected = len - 1;
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.ordered_entry_indices().len();
        if len == 0 {
            self.selected = 0;
            self.scroll_offset = 0;
            return;
        }
        let current = self.selected.min(len - 1) as isize;
        self.selected = (current + delta).clamp(0, len as isize - 1) as usize;
    }

    fn select_entry_index(&mut self, entry_index: usize) {
        if let Some(visible_index) = self
            .ordered_entry_indices()
            .iter()
            .position(|idx| *idx == entry_index)
        {
            self.selected = visible_index;
        }
    }

    fn render_rows(&self) -> Vec<DashboardRenderRow> {
        let mut rows = Vec::new();
        let mut visible_index = 0usize;
        for group in [
            AgentsDashboardGroup::Pinned,
            AgentsDashboardGroup::NeedsInput,
            AgentsDashboardGroup::Working,
            AgentsDashboardGroup::ReadyForReview,
            AgentsDashboardGroup::Completed,
        ] {
            let group_start = rows.len();
            rows.push(DashboardRenderRow::Group(group));
            for (entry_index, entry) in self.entries.iter().enumerate() {
                if group_for_entry(entry) == group {
                    rows.push(DashboardRenderRow::Entry {
                        visible_index,
                        entry_index,
                    });
                    visible_index += 1;
                }
            }
            if rows.len() == group_start + 1 {
                rows.pop();
            } else {
                rows.push(DashboardRenderRow::Spacer);
            }
        }
        if matches!(rows.last(), Some(DashboardRenderRow::Spacer)) {
            rows.pop();
        }
        rows
    }

    fn ensure_selected_row_visible(&mut self, rows: &[DashboardRenderRow], height: usize) {
        if height == 0 || rows.is_empty() {
            self.scroll_offset = 0;
            return;
        }
        let max_offset = rows.len().saturating_sub(height);
        self.scroll_offset = self.scroll_offset.min(max_offset);
        let Some(selected_row) = rows.iter().position(|row| {
            matches!(
                row,
                DashboardRenderRow::Entry {
                    visible_index,
                    ..
                } if *visible_index == self.selected
            )
        }) else {
            return;
        };
        if selected_row < self.scroll_offset {
            self.scroll_offset = selected_row;
        } else if selected_row >= self.scroll_offset + height {
            self.scroll_offset = selected_row + 1 - height;
        }
        self.scroll_offset = self.scroll_offset.min(max_offset);
    }

    fn should_refresh(&self, now: Instant) -> bool {
        self.is_list()
            && self.last_refreshed_at.is_none_or(|last_refreshed_at| {
                now.duration_since(last_refreshed_at) >= REFRESH_INTERVAL
            })
    }

    fn mark_refresh_attempted(&mut self, now: Instant) {
        self.last_refreshed_at = Some(now);
    }
}

impl App {
    pub(super) fn apply_agents_dashboard_footer_hint(&mut self) {
        self.chat_widget.set_footer_hint_override(Some(vec![
            ("enter".to_string(), "to open".to_string()),
            ("space".to_string(), "to reply".to_string()),
            ("p".to_string(), "to pin".to_string()),
            ("ctrl+x".to_string(), "to hide".to_string()),
            ("?".to_string(), "for shortcuts".to_string()),
        ]));
    }

    pub(super) async fn refresh_agents_dashboard(
        &mut self,
        app_server: &mut AppServerSession,
    ) -> Result<()> {
        let Some(dashboard) = self.agents_dashboard.as_mut() else {
            return Ok(());
        };
        let response = app_server
            .agent_view_list(dashboard.cwd_string(), /*include_hidden*/ false)
            .await?;
        dashboard.set_entries(response.entries);
        Ok(())
    }

    pub(super) fn agents_dashboard_showing_list(&self) -> bool {
        self.agents_dashboard
            .as_ref()
            .is_some_and(AgentsDashboardState::is_list)
    }

    pub(super) fn agents_dashboard_showing_thread(&self) -> bool {
        self.agents_dashboard
            .as_ref()
            .is_some_and(|dashboard| dashboard.screen == AgentsDashboardScreen::Thread)
    }

    pub(super) fn agents_dashboard_refresh_due(&mut self) -> bool {
        let now = Instant::now();
        let Some(dashboard) = self.agents_dashboard.as_mut() else {
            return false;
        };
        if !dashboard.should_refresh(now) {
            return false;
        }
        dashboard.mark_refresh_attempted(now);
        true
    }

    pub(super) async fn handle_agents_dashboard_key_event(
        &mut self,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
        key_event: KeyEvent,
    ) -> Result<bool> {
        if self.agents_dashboard_showing_thread()
            && key_event.kind == KeyEventKind::Press
            && matches!(key_event.code, KeyCode::Left)
            && self.chat_widget.no_modal_or_popup_active()
        {
            if let Err(err) = self.return_to_agents_dashboard(tui, app_server).await {
                self.chat_widget
                    .add_error_message(format!("Failed to return to Codex agents view: {err}"));
            }
            return Ok(true);
        }

        if !self.agents_dashboard_showing_list() {
            return Ok(false);
        }

        if self.chat_widget.no_modal_or_popup_active() {
            match key_event {
                KeyEvent {
                    code: KeyCode::Up,
                    kind: KeyEventKind::Press | KeyEventKind::Repeat,
                    ..
                } => {
                    if let Some(dashboard) = self.agents_dashboard.as_mut() {
                        dashboard.move_selection(-1);
                    }
                    tui.frame_requester().schedule_frame();
                    return Ok(true);
                }
                KeyEvent {
                    code: KeyCode::Down,
                    kind: KeyEventKind::Press | KeyEventKind::Repeat,
                    ..
                } => {
                    if let Some(dashboard) = self.agents_dashboard.as_mut() {
                        dashboard.move_selection(1);
                    }
                    tui.frame_requester().schedule_frame();
                    return Ok(true);
                }
                KeyEvent {
                    code: KeyCode::Right,
                    kind: KeyEventKind::Press,
                    ..
                } => {
                    if let Err(err) = self
                        .open_selected_agents_dashboard_entry(tui, app_server)
                        .await
                    {
                        self.chat_widget.add_error_message(format!(
                            "Failed to open Codex agent session: {err}"
                        ));
                    }
                    return Ok(true);
                }
                KeyEvent {
                    code: KeyCode::Enter,
                    kind: KeyEventKind::Press,
                    ..
                } if self.chat_widget.composer_is_empty() => {
                    if let Err(err) = self
                        .open_selected_agents_dashboard_entry(tui, app_server)
                        .await
                    {
                        self.chat_widget.add_error_message(format!(
                            "Failed to open Codex agent session: {err}"
                        ));
                    }
                    return Ok(true);
                }
                KeyEvent {
                    code: KeyCode::Char(' '),
                    modifiers,
                    kind: KeyEventKind::Press,
                    ..
                } if modifiers.is_empty() && self.chat_widget.composer_is_empty() => {
                    if let Err(err) = self
                        .open_selected_agents_dashboard_entry(tui, app_server)
                        .await
                    {
                        self.chat_widget.add_error_message(format!(
                            "Failed to open Codex agent session: {err}"
                        ));
                    }
                    return Ok(true);
                }
                KeyEvent {
                    code: KeyCode::Char('x'),
                    modifiers,
                    kind: KeyEventKind::Press,
                    ..
                } if modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Err(err) = self.hide_selected_agents_dashboard_entry(app_server).await {
                        self.chat_widget.add_error_message(format!(
                            "Failed to remove Codex agent session from view: {err}"
                        ));
                    }
                    tui.frame_requester().schedule_frame();
                    return Ok(true);
                }
                KeyEvent {
                    code: KeyCode::Char('p'),
                    modifiers,
                    kind: KeyEventKind::Press,
                    ..
                } if modifiers.is_empty() && self.chat_widget.composer_is_empty() => {
                    if let Err(err) = self.toggle_selected_agents_dashboard_pin(app_server).await {
                        self.chat_widget
                            .add_error_message(format!("Failed to update Codex agent pin: {err}"));
                    }
                    tui.frame_requester().schedule_frame();
                    return Ok(true);
                }
                KeyEvent {
                    code: KeyCode::Char('c'),
                    modifiers,
                    kind: KeyEventKind::Press,
                    ..
                } if modifiers.is_empty() && self.chat_widget.composer_is_empty() => {
                    if let Err(err) = self
                        .set_selected_agents_dashboard_completed(app_server)
                        .await
                    {
                        self.chat_widget.add_error_message(format!(
                            "Failed to mark Codex agent session completed: {err}"
                        ));
                    }
                    tui.frame_requester().schedule_frame();
                    return Ok(true);
                }
                KeyEvent {
                    code: KeyCode::Char('r'),
                    modifiers,
                    kind: KeyEventKind::Press,
                    ..
                } if modifiers.is_empty() && self.chat_widget.composer_is_empty() => {
                    if let Err(err) = self.set_selected_agents_dashboard_ready(app_server).await {
                        self.chat_widget.add_error_message(format!(
                            "Failed to mark Codex agent session ready for review: {err}"
                        ));
                    }
                    tui.frame_requester().schedule_frame();
                    return Ok(true);
                }
                _ => {}
            }
        }

        match self
            .chat_widget
            .handle_dashboard_composer_key_event(key_event)
        {
            DashboardComposerInput::None => {}
            DashboardComposerInput::Submitted(user_message) => {
                if let Err(err) = self
                    .start_agents_dashboard_session(tui, app_server, user_message)
                    .await
                {
                    self.chat_widget
                        .add_error_message(format!("Failed to start Codex agent session: {err}"));
                }
            }
        }
        tui.frame_requester().schedule_frame();
        Ok(true)
    }

    pub(super) async fn handle_agents_dashboard_mouse_event(
        &mut self,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
        mouse_event: MouseEvent,
    ) -> Result<bool> {
        if !self.agents_dashboard_showing_list() {
            return Ok(false);
        }
        match mouse_event.kind {
            MouseEventKind::ScrollUp => {
                if let Some(dashboard) = self.agents_dashboard.as_mut() {
                    dashboard.move_selection(-1);
                }
                tui.frame_requester().schedule_frame();
                return Ok(true);
            }
            MouseEventKind::ScrollDown => {
                if let Some(dashboard) = self.agents_dashboard.as_mut() {
                    dashboard.move_selection(1);
                }
                tui.frame_requester().schedule_frame();
                return Ok(true);
            }
            MouseEventKind::Down(MouseButton::Left) => {}
            _ => return Ok(false),
        }
        let Some(dashboard) = self.agents_dashboard.as_mut() else {
            return Ok(false);
        };
        let Some(hitbox) = dashboard
            .row_hitboxes
            .iter()
            .find(|hitbox| hitbox.y == mouse_event.row)
            .copied()
        else {
            return Ok(false);
        };
        dashboard.select_entry_index(hitbox.entry_index);
        if let Err(err) = self
            .open_selected_agents_dashboard_entry(tui, app_server)
            .await
        {
            self.chat_widget
                .add_error_message(format!("Failed to open Codex agent session: {err}"));
        }
        Ok(true)
    }

    pub(super) async fn start_agents_dashboard_session(
        &mut self,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
        user_message: crate::chatwidget::UserMessage,
    ) -> Result<()> {
        self.refresh_in_memory_config_from_disk_best_effort("starting an agent dashboard thread")
            .await;
        let mut config = self.fresh_session_config();
        apply_managed_new_thread_defaults(
            &mut config,
            app_server.managed_new_thread_defaults(),
            &self.cli_kv_overrides,
            &self.harness_overrides,
        );
        self.unsubscribe_agents_dashboard_tracked_threads(app_server)
            .await;
        self.config = config.clone();
        let started = app_server
            .start_thread_with_session_start_source(&config, /*session_start_source*/ None)
            .await?;
        let thread_id = started.session.thread_id;
        let initial_prompt = initial_prompt_for_user_message(&user_message);
        let cwd = self
            .agents_dashboard
            .as_ref()
            .map(AgentsDashboardState::cwd_string)
            .unwrap_or_else(|| self.config.cwd.display().to_string());
        app_server
            .agent_view_attach_thread(cwd, thread_id, initial_prompt)
            .await?;
        self.replace_chat_widget_with_app_server_thread(
            tui,
            app_server,
            started,
            Some(user_message),
        )
        .await?;
        if let Some(dashboard) = self.agents_dashboard.as_mut() {
            dashboard.screen = AgentsDashboardScreen::Thread;
        }
        Ok(())
    }

    async fn open_selected_agents_dashboard_entry(
        &mut self,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
    ) -> Result<()> {
        let Some(thread_id) = self
            .agents_dashboard
            .as_ref()
            .and_then(AgentsDashboardState::selected_thread_id)
        else {
            return Ok(());
        };
        self.unsubscribe_agents_dashboard_tracked_threads(app_server)
            .await;
        let started = app_server
            .resume_thread(self.config.clone(), thread_id)
            .await?;
        self.replace_chat_widget_with_app_server_thread(
            tui, app_server, started, /*initial_user_message*/ None,
        )
        .await?;
        if let Some(dashboard) = self.agents_dashboard.as_mut() {
            dashboard.screen = AgentsDashboardScreen::Thread;
        }
        Ok(())
    }

    async fn return_to_agents_dashboard(
        &mut self,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
    ) -> Result<()> {
        self.store_active_thread_receiver().await;
        self.active_thread_id = None;
        self.active_thread_rx = None;
        self.refresh_pending_thread_approvals().await;
        if let Some(dashboard) = self.agents_dashboard.as_mut() {
            dashboard.screen = AgentsDashboardScreen::List;
        }
        self.refresh_agents_dashboard(app_server).await?;
        let init = self.chatwidget_init_for_forked_or_resumed_thread(
            tui,
            self.config.clone(),
            /*initial_user_message*/ None,
        );
        self.replace_chat_widget(ChatWidget::new_with_app_event(init));
        self.apply_agents_dashboard_footer_hint();
        self.reset_for_thread_switch(tui)?;
        tui.frame_requester().schedule_frame();
        Ok(())
    }

    async fn unsubscribe_agents_dashboard_tracked_threads(
        &mut self,
        app_server: &mut AppServerSession,
    ) {
        let tracked_thread_ids: Vec<ThreadId> =
            self.thread_event_channels.keys().copied().collect();
        for thread_id in tracked_thread_ids {
            if let Err(err) = app_server.thread_unsubscribe(thread_id).await {
                tracing::warn!("failed to unsubscribe dashboard thread {thread_id}: {err}");
            }
        }
    }

    async fn update_selected_agents_dashboard_entry(
        &mut self,
        app_server: &mut AppServerSession,
        view_state: Option<AgentViewWorkflowState>,
        pinned: Option<bool>,
    ) -> Result<()> {
        let Some(dashboard) = self.agents_dashboard.as_ref() else {
            return Ok(());
        };
        let Some(entry_index) = dashboard.selected_entry_index() else {
            return Ok(());
        };
        let Some(entry) = dashboard.entries.get(entry_index) else {
            return Ok(());
        };
        let response = app_server
            .agent_view_update_entry(AgentViewUpdateEntryParams {
                cwd: dashboard.cwd_string(),
                thread_id: entry.thread_id.clone(),
                view_state,
                pinned,
                position: None,
                title_override: None,
            })
            .await?;
        if let Some(dashboard) = self.agents_dashboard.as_mut()
            && let Some(slot) = dashboard
                .entries
                .iter_mut()
                .find(|entry| entry.thread_id == response.entry.thread_id)
        {
            *slot = response.entry;
            dashboard.clamp_selection();
        }
        Ok(())
    }

    async fn toggle_selected_agents_dashboard_pin(
        &mut self,
        app_server: &mut AppServerSession,
    ) -> Result<()> {
        let pinned = self
            .agents_dashboard
            .as_ref()
            .and_then(|dashboard| {
                dashboard
                    .selected_entry_index()
                    .and_then(|idx| dashboard.entries.get(idx))
            })
            .map(|entry| !entry.pinned);
        self.update_selected_agents_dashboard_entry(app_server, None, pinned)
            .await
    }

    async fn set_selected_agents_dashboard_completed(
        &mut self,
        app_server: &mut AppServerSession,
    ) -> Result<()> {
        self.update_selected_agents_dashboard_entry(
            app_server,
            Some(AgentViewWorkflowState::Completed),
            None,
        )
        .await
    }

    async fn set_selected_agents_dashboard_ready(
        &mut self,
        app_server: &mut AppServerSession,
    ) -> Result<()> {
        self.update_selected_agents_dashboard_entry(
            app_server,
            Some(AgentViewWorkflowState::ReadyForReview),
            None,
        )
        .await
    }

    async fn hide_selected_agents_dashboard_entry(
        &mut self,
        app_server: &mut AppServerSession,
    ) -> Result<()> {
        let Some(dashboard) = self.agents_dashboard.as_ref() else {
            return Ok(());
        };
        let Some(entry_index) = dashboard.selected_entry_index() else {
            return Ok(());
        };
        let Some(entry) = dashboard.entries.get(entry_index) else {
            return Ok(());
        };
        let thread_id = ThreadId::from_string(&entry.thread_id)
            .map_err(|err| color_eyre::eyre::eyre!("invalid dashboard thread id: {err}"))?;
        app_server
            .agent_view_hide_entry(dashboard.cwd_string(), thread_id)
            .await?;
        if let Some(dashboard) = self.agents_dashboard.as_mut() {
            dashboard.entries.remove(entry_index);
            dashboard.clamp_selection();
        }
        Ok(())
    }

    pub(super) fn render_agents_dashboard_frame(&mut self, tui: &mut tui::Tui) -> Result<Rect> {
        let terminal_area = tui.terminal.size()?;
        let composer_height = self
            .chat_widget
            .bottom_pane_desired_height(terminal_area.width)
            .min(terminal_area.height);
        let desired_height = terminal_area.height;
        let mut rendered_area = Rect::default();
        tui.draw_with_resize_reflow(desired_height, |frame| {
            let area = frame.area();
            rendered_area = area;
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(0),
                    Constraint::Length(composer_height.min(area.height)),
                ])
                .split(area);
            self.render_agents_dashboard_list(chunks[0], frame.buffer);
            self.chat_widget.render_bottom_pane(chunks[1], frame.buffer);
            if let Some((x, y)) = self.chat_widget.bottom_pane_cursor_pos(chunks[1]) {
                frame.set_cursor_style(self.chat_widget.bottom_pane_cursor_style(chunks[1]));
                frame.set_cursor_position((x, y));
            }
        })?;
        Ok(rendered_area)
    }

    fn render_agents_dashboard_list(&mut self, area: Rect, buf: &mut Buffer) {
        let Some(dashboard) = self.agents_dashboard.as_mut() else {
            return;
        };
        dashboard.row_hitboxes.clear();
        buf.set_style(area, Style::default());

        let mut y = area.y;
        render_line(
            buf,
            area,
            &mut y,
            Line::from(vec![
                "Codex ".bold(),
                "agents".bold(),
                "  ".into(),
                CODEX_CLI_VERSION.dim(),
            ]),
        );
        render_line(
            buf,
            area,
            &mut y,
            Line::from(vec![
                dashboard.cwd.display().to_string().dim(),
                "  ".into(),
                format!(
                    "{} awaiting input · {} working · {} completed",
                    count_group(&dashboard.entries, AgentsDashboardGroup::NeedsInput),
                    count_group(&dashboard.entries, AgentsDashboardGroup::Working),
                    count_group(&dashboard.entries, AgentsDashboardGroup::Completed)
                )
                .dim(),
            ]),
        );
        y = y.saturating_add(1);

        if dashboard.ordered_entry_indices().is_empty() {
            render_line(
                buf,
                area,
                &mut y,
                "No Codex agent sessions in this view yet.".dim().into(),
            );
            render_line(
                buf,
                area,
                &mut y,
                "Describe a task below and press Enter to create one."
                    .dim()
                    .into(),
            );
            return;
        }

        let rows = dashboard.render_rows();
        let body_height = area.bottom().saturating_sub(y) as usize;
        dashboard.ensure_selected_row_visible(&rows, body_height);
        for row in rows.iter().skip(dashboard.scroll_offset) {
            if y >= area.bottom() {
                break;
            }
            match *row {
                DashboardRenderRow::Group(group) => {
                    render_line(buf, area, &mut y, group.title().bold().into());
                }
                DashboardRenderRow::Entry {
                    visible_index,
                    entry_index,
                } => {
                    let entry = &dashboard.entries[entry_index];
                    let row_y = y;
                    if render_agent_entry_row(
                        buf,
                        area,
                        &mut y,
                        entry,
                        visible_index == dashboard.selected,
                    ) {
                        dashboard.row_hitboxes.push(RowHitbox {
                            y: row_y,
                            entry_index,
                        });
                    }
                }
                DashboardRenderRow::Spacer => {
                    y = y.saturating_add(1);
                }
            }
        }
    }
}

impl AgentsDashboardGroup {
    fn title(self) -> &'static str {
        match self {
            AgentsDashboardGroup::Pinned => "Pinned",
            AgentsDashboardGroup::NeedsInput => "Needs input",
            AgentsDashboardGroup::Working => "Working",
            AgentsDashboardGroup::ReadyForReview => "Ready for review",
            AgentsDashboardGroup::Completed => "Completed",
        }
    }
}

fn group_for_entry(entry: &AgentViewEntry) -> AgentsDashboardGroup {
    if entry.pinned {
        return AgentsDashboardGroup::Pinned;
    }
    match entry.thread.as_ref().map(|thread| &thread.status) {
        Some(ThreadStatus::Active { active_flags })
            if active_flags.contains(&ThreadActiveFlag::WaitingOnApproval)
                || active_flags.contains(&ThreadActiveFlag::WaitingOnUserInput) =>
        {
            AgentsDashboardGroup::NeedsInput
        }
        Some(ThreadStatus::Active { .. }) => AgentsDashboardGroup::Working,
        Some(ThreadStatus::SystemError) => AgentsDashboardGroup::Completed,
        Some(ThreadStatus::Idle | ThreadStatus::NotLoaded) | None => match entry.view_state {
            AgentViewWorkflowState::ReadyForReview => AgentsDashboardGroup::ReadyForReview,
            AgentViewWorkflowState::Completed => AgentsDashboardGroup::Completed,
        },
    }
}

fn count_group(entries: &[AgentViewEntry], group: AgentsDashboardGroup) -> usize {
    entries
        .iter()
        .filter(|entry| group_for_entry(entry) == group)
        .count()
}

fn initial_prompt_for_user_message(user_message: &crate::chatwidget::UserMessage) -> String {
    if !user_message.text.trim().is_empty() {
        user_message.text.trim().to_string()
    } else if !user_message.local_images.is_empty() || !user_message.remote_image_urls.is_empty() {
        "Image task".to_string()
    } else {
        String::new()
    }
}

fn render_agent_entry_row(
    buf: &mut Buffer,
    area: Rect,
    y: &mut u16,
    entry: &AgentViewEntry,
    selected: bool,
) -> bool {
    if *y >= area.bottom() {
        return false;
    }
    let title = entry_title(entry);
    let preview = entry_preview(entry);
    let age = entry_age(entry).unwrap_or_else(|| "now".to_string());
    let status = entry_status_label(entry);
    let marker = if entry.pinned { "✻" } else { "•" };
    let marker_color = match group_for_entry(entry) {
        AgentsDashboardGroup::NeedsInput => Color::Yellow,
        AgentsDashboardGroup::Working => Color::Green,
        AgentsDashboardGroup::Completed => Color::Red,
        AgentsDashboardGroup::Pinned | AgentsDashboardGroup::ReadyForReview => Color::Blue,
    };
    let style = if selected {
        Style::default()
            .bg(Color::Rgb(235, 235, 235))
            .fg(Color::Black)
    } else {
        Style::default()
    };
    let row = Rect::new(area.x, *y, area.width, 1);
    buf.set_style(row, style);
    if row.width < 28 {
        let line = truncate_line_with_ellipsis_if_overflow(
            Line::from(vec![
                Span::styled(marker, Style::default().fg(marker_color)),
                Span::raw(" "),
                Span::styled(title, Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(" "),
                Span::styled(status, status_style(entry)),
            ]),
            row.width as usize,
        );
        Paragraph::new(line).style(style).render(row, buf);
        *y = y.saturating_add(1);
        return true;
    }

    let meta_width = 22_u16.min(row.width / 3).max(10);
    let title_width = 38_u16.min(row.width.saturating_sub(meta_width + 2).max(12));
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(title_width),
            Constraint::Min(0),
            Constraint::Length(meta_width),
        ])
        .split(row);
    let title_line = truncate_line_with_ellipsis_if_overflow(
        Line::from(vec![
            Span::styled(marker, Style::default().fg(marker_color)),
            Span::raw(" "),
            Span::styled(title, Style::default().add_modifier(Modifier::BOLD)),
        ]),
        chunks[0].width as usize,
    );
    let preview_line = truncate_line_with_ellipsis_if_overflow(
        Line::from(Span::styled(preview, Style::default().fg(Color::DarkGray))),
        chunks[1].width as usize,
    );
    let meta_line = truncate_line_with_ellipsis_if_overflow(
        Line::from(vec![
            Span::styled(status, status_style(entry)),
            Span::raw("  "),
            Span::styled(age, Style::default().fg(Color::DarkGray)),
        ]),
        chunks[2].width as usize,
    );
    Paragraph::new(title_line)
        .style(style)
        .render(chunks[0], buf);
    Paragraph::new(preview_line)
        .style(style)
        .render(chunks[1], buf);
    Paragraph::new(meta_line)
        .style(style)
        .alignment(Alignment::Right)
        .render(chunks[2], buf);
    *y = y.saturating_add(1);
    true
}

fn render_line(buf: &mut Buffer, area: Rect, y: &mut u16, line: Line<'static>) {
    if *y >= area.bottom() {
        return;
    }
    let row = Rect::new(area.x, *y, area.width, 1);
    Paragraph::new(line).render(row, buf);
    *y = y.saturating_add(1);
}

fn entry_title(entry: &AgentViewEntry) -> String {
    entry
        .title_override
        .clone()
        .or_else(|| entry.thread.as_ref().and_then(|thread| thread.name.clone()))
        .unwrap_or_else(|| first_line(&entry.initial_prompt))
}

fn entry_preview(entry: &AgentViewEntry) -> String {
    entry
        .thread
        .as_ref()
        .map(|thread| thread.preview.clone())
        .filter(|preview| !preview.trim().is_empty())
        .unwrap_or_else(|| first_line(&entry.initial_prompt))
}

fn entry_status_label(entry: &AgentViewEntry) -> &'static str {
    match entry.thread.as_ref().map(|thread| &thread.status) {
        Some(ThreadStatus::Active { active_flags })
            if active_flags.contains(&ThreadActiveFlag::WaitingOnApproval)
                || active_flags.contains(&ThreadActiveFlag::WaitingOnUserInput) =>
        {
            "needs input"
        }
        Some(ThreadStatus::Active { .. }) => "working",
        Some(ThreadStatus::SystemError) => "error",
        Some(ThreadStatus::Idle | ThreadStatus::NotLoaded) | None => match entry.view_state {
            AgentViewWorkflowState::ReadyForReview => "ready",
            AgentViewWorkflowState::Completed => "completed",
        },
    }
}

fn status_style(entry: &AgentViewEntry) -> Style {
    match group_for_entry(entry) {
        AgentsDashboardGroup::NeedsInput => Style::default().fg(Color::Yellow),
        AgentsDashboardGroup::Working => Style::default().fg(Color::Green),
        AgentsDashboardGroup::Completed => Style::default().fg(Color::Red),
        AgentsDashboardGroup::Pinned | AgentsDashboardGroup::ReadyForReview => {
            Style::default().fg(Color::DarkGray)
        }
    }
}

fn entry_age(entry: &AgentViewEntry) -> Option<String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    let timestamp = entry
        .thread
        .as_ref()
        .and_then(|thread| thread.recency_at)
        .unwrap_or(entry.updated_at.max(entry.created_at));
    let elapsed = now.saturating_sub(timestamp);
    Some(if elapsed >= 86_400 {
        format!("{}d", elapsed / 86_400)
    } else if elapsed >= 3_600 {
        format!("{}h", elapsed / 3_600)
    } else if elapsed >= 60 {
        format!("{}m", elapsed / 60)
    } else {
        "now".to_string()
    })
}

fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    if line.is_empty() {
        "untitled agent".to_string()
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_app_server_protocol::GitInfo;
    use codex_app_server_protocol::SessionSource;
    use codex_app_server_protocol::Thread;
    use codex_app_server_protocol::ThreadHistoryMode;
    use codex_app_server_protocol::ThreadSource;
    use codex_utils_absolute_path::AbsolutePathBuf;

    fn entry(thread_id: &str, pinned: bool, status: Option<ThreadStatus>) -> AgentViewEntry {
        AgentViewEntry {
            thread_id: thread_id.to_string(),
            initial_prompt: format!("task {thread_id}"),
            title_override: None,
            view_state: AgentViewWorkflowState::ReadyForReview,
            pinned,
            position: 0,
            hidden: false,
            created_at: 1,
            updated_at: 1,
            last_opened_at: None,
            thread: status.map(|status| Thread {
                id: thread_id.to_string(),
                extra: None,
                session_id: thread_id.to_string(),
                forked_from_id: None,
                parent_thread_id: None,
                preview: String::new(),
                ephemeral: false,
                history_mode: ThreadHistoryMode::default(),
                model_provider: "openai".to_string(),
                created_at: 1,
                updated_at: 1,
                recency_at: Some(1),
                status,
                path: None,
                cwd: AbsolutePathBuf::try_from(test_path_buf("/tmp")).unwrap(),
                cli_version: "test".to_string(),
                source: SessionSource::Cli,
                thread_source: Some(ThreadSource::User),
                agent_nickname: None,
                agent_role: None,
                git_info: Some(GitInfo {
                    sha: None,
                    branch: None,
                    origin_url: None,
                }),
                name: None,
                turns: Vec::new(),
            }),
        }
    }

    #[test]
    fn groups_pinned_before_runtime_state() {
        let entry = entry(
            "01900000-0000-7000-8000-000000000001",
            true,
            Some(ThreadStatus::Active {
                active_flags: vec![ThreadActiveFlag::WaitingOnUserInput],
            }),
        );

        assert_eq!(group_for_entry(&entry), AgentsDashboardGroup::Pinned);
    }

    #[test]
    fn active_waiting_thread_is_needs_input() {
        let entry = entry(
            "01900000-0000-7000-8000-000000000002",
            false,
            Some(ThreadStatus::Active {
                active_flags: vec![ThreadActiveFlag::WaitingOnApproval],
            }),
        );

        assert_eq!(group_for_entry(&entry), AgentsDashboardGroup::NeedsInput);
    }

    #[test]
    fn status_label_reflects_runtime_thread_state() {
        let working = entry(
            "01900000-0000-7000-8000-000000000006",
            false,
            Some(ThreadStatus::Active {
                active_flags: vec![],
            }),
        );
        let waiting = entry(
            "01900000-0000-7000-8000-000000000007",
            false,
            Some(ThreadStatus::Active {
                active_flags: vec![ThreadActiveFlag::WaitingOnUserInput],
            }),
        );
        let failed = entry(
            "01900000-0000-7000-8000-000000000008",
            false,
            Some(ThreadStatus::SystemError),
        );

        assert_eq!(entry_status_label(&working), "working");
        assert_eq!(entry_status_label(&waiting), "needs input");
        assert_eq!(entry_status_label(&failed), "error");
    }

    #[test]
    fn render_rows_keep_group_order_without_duplicate_entries() {
        let mut dashboard = AgentsDashboardState::new(test_path_buf("/tmp"));
        dashboard.set_entries(vec![
            entry(
                "01900000-0000-7000-8000-000000000003",
                false,
                Some(ThreadStatus::SystemError),
            ),
            entry(
                "01900000-0000-7000-8000-000000000004",
                true,
                Some(ThreadStatus::Active {
                    active_flags: vec![],
                }),
            ),
            entry(
                "01900000-0000-7000-8000-000000000005",
                false,
                Some(ThreadStatus::Active {
                    active_flags: vec![ThreadActiveFlag::WaitingOnUserInput],
                }),
            ),
        ]);

        let rows = dashboard.render_rows();
        let groups: Vec<AgentsDashboardGroup> = rows
            .iter()
            .filter_map(|row| match row {
                DashboardRenderRow::Group(group) => Some(*group),
                _ => None,
            })
            .collect();
        let entry_indices: Vec<usize> = rows
            .iter()
            .filter_map(|row| match row {
                DashboardRenderRow::Entry { entry_index, .. } => Some(*entry_index),
                _ => None,
            })
            .collect();

        assert_eq!(
            groups,
            vec![
                AgentsDashboardGroup::Pinned,
                AgentsDashboardGroup::NeedsInput,
                AgentsDashboardGroup::Completed,
            ]
        );
        assert_eq!(entry_indices, vec![1, 2, 0]);
    }

    #[test]
    fn selected_render_row_scrolls_into_view() {
        let mut dashboard = AgentsDashboardState::new(test_path_buf("/tmp"));
        dashboard.set_entries(
            (0..12)
                .map(|idx| {
                    entry(
                        &format!("01900000-0000-7000-8000-{idx:012}"),
                        false,
                        Some(ThreadStatus::SystemError),
                    )
                })
                .collect(),
        );
        dashboard.selected = 11;
        let rows = dashboard.render_rows();
        dashboard.ensure_selected_row_visible(&rows, 5);
        let selected_row = rows
            .iter()
            .position(|row| {
                matches!(
                    row,
                    DashboardRenderRow::Entry {
                        visible_index: 11,
                        ..
                    }
                )
            })
            .expect("selected row");

        assert!(selected_row >= dashboard.scroll_offset);
        assert!(selected_row < dashboard.scroll_offset + 5);
    }
}
