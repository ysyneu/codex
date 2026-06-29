# `codex agents` command design

Status: draft design
Date: 2026-06-29
Target surface: Codex Rust CLI and TUI

## Problem

Users want a terminal dashboard that behaves like Claude Code's `claude agents`
command:

- start the dashboard from a working directory with `codex agents`;
- show only sessions created from that dashboard for that directory;
- group sessions by workflow status;
- keep a bottom composer permanently available for creating new sessions;
- reuse normal Codex input behavior, including slash commands, file mentions,
  image paste, prompt history, approvals, and streaming tool output;
- enter a session with Enter, Right, or mouse selection;
- return to the dashboard from a session with Left when the composer is idle;
- pin sessions and hide sessions from the dashboard without deleting the
  underlying Codex thread.

The old external wrapper architecture cannot satisfy this. It can launch Codex,
read transcript output, and send prompts, but it cannot become the native Codex
composer or own live approval, paste, mention, slash-command, and tool-streaming
state. The dashboard has to live inside Codex TUI.

## Current facts

- `codex resume` is a session picker for all resumable Codex sessions. It is not
  a scoped agents dashboard.
- `/agent` is a sub-agent picker for threads spawned by the current chat. It is
  not a top-level session dashboard.
- `ThreadStatus` currently exposes only `NotLoaded`, `Idle`, `SystemError`, and
  `Active { active_flags }`, where active flags cover waiting on approval and
  waiting on user input. It does not encode "ready for review" or "completed".
- `ChatComposer` owns the native input experience: slash commands, file search,
  skill/plugin popups, image paste, history, cursor movement, and text editing.
- The app-server already exposes the thread lifecycle needed by a real
  dashboard: `thread/start`, `turn/start`, `thread/resume`, `thread/read`,
  `thread/list`, loaded-thread status notifications, and turn notifications.
- The state database already persists thread metadata and should own any durable
  dashboard membership state.

## Goal

Implement `codex agents` as a native Codex TUI mode that matches Claude Agents'
interaction model while preserving Codex's real chat/session behavior.

The command is not a transcript browser and not an orchestration wrapper. Each
row is a real Codex thread. Opening a row shows the normal Codex chat UI for that
thread. Sending input in that view is the same as sending input in normal Codex.

## Non-goals

- Do not import all historical Codex sessions into the dashboard.
- Do not reuse or extend the rejected external wrapper design.
- Do not duplicate `ChatComposer` behavior.
- Do not replace `/agent`; keep it focused on sub-agent thread switching.
- Do not make this a plugin-only feature. Plugins do not currently have the TUI
  extension points needed for this command.
- Do not physically delete Codex threads when the user deletes a dashboard row.

## Product contract

### Scope

`codex agents` opens a dashboard scoped to the canonical current working
directory. Sessions appear only if they were created from that dashboard scope.
Starting `codex agents` from another directory opens another scoped dashboard.

The scope key is the canonical cwd path plus the active Codex home. This prevents
two users or two `CODEX_HOME` values from sharing accidental state for the same
filesystem path.

### Empty state

An empty dashboard shows the header, an empty list area, the bottom composer, and
the footer. It does not scan or import normal `codex resume` history.

### Header

The header should provide the same information density as Claude Agents while
using Codex facts:

- Codex product and CLI version;
- active model and reasoning effort;
- scoped cwd;
- counts for awaiting input, working, ready for review, and completed;
- authentication or configuration warnings if normal Codex would show them.

### Row layout

Rows are compact and scan-first:

- status glyph;
- title, derived from the first prompt until Codex title metadata is available;
- latest preview, using thread preview or the latest assistant/user summary;
- optional git branch or short SHA when available;
- optional counters for approvals, changed files, or spawned sub-agents when
  available from existing thread metadata;
- age since last activity.

The design should not copy Claude-specific columns such as PR counts unless
Codex has real data for them. The requirement is density and workflow value, not
fake parity.

### Groups

Rows render in this order:

1. Pinned
2. Awaiting input
3. Working
4. Ready for review
5. Completed
6. Failed

Pinned rows stay in `Pinned` regardless of their underlying state, but keep their
status glyph and right-side status text.

### Status derivation

The dashboard status is a view workflow state, not a direct alias of
`ThreadStatus`.

Derivation order:

1. hidden rows are omitted;
2. `ThreadStatus::SystemError` maps to `Failed`;
3. `ThreadStatus::Active` with `WaitingOnApproval` or `WaitingOnUserInput` maps
   to `Awaiting input`;
4. other `ThreadStatus::Active` maps to `Working`;
5. explicit view state `Completed` maps to `Completed`;
6. explicit view state `ReadyForReview` maps to `Ready for review`;
7. idle rows default to `Ready for review`.

This is intentionally separate from app-server thread status because app-server
status is liveness, while the agents dashboard is a review workflow.

### Bottom composer

The bottom composer must be the real Codex composer.

Dashboard composer behavior:

- text submission creates a new Codex thread in the dashboard scope and starts
  the first turn with that text;
- slash command completion uses the same command popup as normal Codex;
- file mention completion uses the same file search popup;
- paste behavior, including image paste, is the same as normal Codex;
- commands that configure the session or client work before the new thread is
  created;
- commands that require an existing thread return a clear inline error until a
  row is opened.

This preserves the user's expectation that input at the bottom of the dashboard
feels like Codex, not a second implementation of Codex input.

### Creating a session

On non-command submission in the dashboard:

1. create or find the dashboard scope record;
2. call `thread/start` with the current cwd and normal TUI config;
3. insert a dashboard entry for the returned thread id immediately;
4. call `turn/start` with the submitted prompt;
5. keep focus on the dashboard and stream row status/preview as events arrive.

The dashboard never creates a fake external process per row. It uses the same
app-server lifecycle as normal Codex.

### Opening a session

Enter, Right, or clicking a row opens that row's real Codex thread. If the thread
is already loaded, the dashboard switches to it. If not, it calls `thread/resume`
and constructs the normal `ChatWidget` state from the response.

Once opened, the user is in the actual Codex chat view for that thread:

- approvals render normally;
- tool calls stream normally;
- slash commands dispatch normally;
- follow-up prompts use the normal session state;
- images and file mentions work normally.

### Returning to dashboard

Left returns from an opened session to the dashboard only when the composer is
idle:

- no text draft;
- no command, file, skill, model, permission, or other popup;
- no active selection that expects Left for local navigation.

If the composer is not idle, Left remains a normal text-editing or popup key.
This is the only coherent way to copy Claude's navigation without breaking
Codex's existing input semantics. Esc can also return to the dashboard when the
composer is idle, because some terminals do not send reliable Left events in all
keyboard modes.

### Dashboard navigation

- Up and Down move the selected row.
- Enter and Right open the selected row.
- Left is a no-op in the dashboard.
- Space replies to the selected row by opening it and focusing the composer.
- `p` toggles pin.
- `ctrl+x` hides the selected row from the dashboard.
- `?` opens a shortcut overlay.
- Mouse click selects and opens a row.
- Mouse wheel scrolls the row list when rows exceed terminal height.

### Deleting a row

Delete means "hide from this dashboard". It sets `hidden_at` on the dashboard
entry. It does not archive, delete, truncate, or mutate the underlying Codex
thread.

A hidden row may be restored later by a dashboard-specific restore command, but
restore is not required for the first implementation.

## Architecture

### CLI entry

Add a top-level `agents` subcommand beside `resume`, `fork`, `archive`, and
`delete`.

The subcommand should reuse normal TUI shared options:

- `-C/--cd`;
- profile/config overrides;
- model and reasoning settings;
- sandbox/approval settings;
- `--search`;
- `--no-alt-screen`.

The top-level CLI forwards into TUI with an internal `agents_dashboard` flag, the
same way `resume` and `fork` use internal flags today.

### TUI mode

Introduce a first-class dashboard mode instead of bending `/agent`:

- `codex-rs/tui/src/agents_dashboard/model.rs`
- `codex-rs/tui/src/agents_dashboard/render.rs`
- `codex-rs/tui/src/agents_dashboard/input.rs`
- `codex-rs/tui/src/agents_dashboard/store.rs`
- `codex-rs/tui/src/agents_dashboard/mod.rs`

`App` owns mode switching:

- dashboard mode;
- opened thread mode;
- transition back to dashboard.

Existing `ChatWidget` and `ChatComposer` remain the only chat implementation.

### Persistent registry

Add state database tables owned by `codex_state`, exposed through app-server
methods so local and remote app-server modes behave the same.

Proposed migration:

```sql
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
  FOREIGN KEY (scope_key) REFERENCES agent_views(scope_key)
);

CREATE INDEX idx_agent_view_threads_visible
ON agent_view_threads(scope_key, hidden_at_ms, pinned, position, updated_at_ms);
```

The registry stores membership and view state only. Canonical session data stays
in existing thread metadata and rollout storage.

### App-server API

Expose narrow experimental requests:

- `agentView/list`: list dashboard entries for a scope, joined with thread
  metadata and loaded status;
- `agentView/attachThread`: add a newly started thread to a scope;
- `agentView/updateEntry`: update pin, position, title override, or view state;
- `agentView/hideEntry`: set `hidden_at_ms`.

These requests should be app-server owned instead of local TUI file writes. That
keeps embedded, daemon, and remote app-server usage consistent.

### Event flow

The dashboard subscribes to normal app-server notifications for threads in the
current scope:

- `thread/status/changed` updates liveness groups;
- `turn/started` and turn completion update activity time;
- thread metadata refresh updates title and preview;
- errors update the row into `Failed`.

If a listed thread is not loaded, the dashboard can still render stored metadata.
It should not eagerly resume every hidden or inactive thread just to populate the
view.

### Relationship to `/agent`

`/agent` remains a current-chat sub-agent picker. `codex agents` is a top-level
session dashboard.

Shared helpers are acceptable for row formatting, status glyphs, and navigation
state, but product semantics stay separate. This avoids repeating the mistake of
trying to turn the sub-agent picker into a general dashboard.

### Relationship to issue #22321

Issue #22321 is an open feature request and the inspected prototype branch only
enhances `/agent` for active/persisted sub-agent threads. This design does not
conflict with that work because it introduces a distinct CLI command and a
dashboard registry scoped to sessions created by the dashboard.

If upstream later accepts a broader agent-view design, this implementation can
share state and rendering pieces, but it should not depend on the current
prototype.

## Testing

Unit and snapshot tests:

- status derivation from `ThreadStatus` plus view state;
- group ordering and pinned behavior;
- row truncation across narrow terminal widths;
- keyboard navigation;
- hide semantics;
- scope-key generation;
- registry queries and migrations.

TUI integration tests:

- `codex agents` starts in dashboard mode;
- submitting a prompt calls `thread/start`, records membership, and calls
  `turn/start`;
- Enter/Right opens the selected real thread;
- Left returns to dashboard only when composer state is idle;
- slash and mention popups still use the normal composer paths.

Manual Ghostty QA:

- run `codex agents` in a repo with no dashboard sessions;
- create two sessions and confirm only those two appear;
- open a session, use `/`, `@`, and image paste;
- return with Left from an idle composer;
- pin a row;
- hide a row with `ctrl+x` and confirm `codex resume` can still find it;
- resize the terminal and confirm rows do not overlap the composer/footer.

## Implementation sequence

1. Add the CLI flag and empty dashboard mode with native composer rendering.
2. Add state registry migration and app-server registry requests.
3. Wire create-session flow through `thread/start` and `turn/start`.
4. Wire open-session flow through loaded-thread switch or `thread/resume`.
5. Add live row updates from app-server notifications.
6. Add pin, hide, shortcut overlay, mouse navigation, and focused tests.
7. Polish layout against Claude Agents screenshots and Ghostty behavior.

## Risks

- Left-arrow behavior conflicts with text editing unless it is gated on an idle
  composer. The design explicitly gates it.
- App-server thread status does not encode review workflow. The registry must
  carry view workflow state.
- If registry state is stored outside app-server, remote app-server users will
  see inconsistent dashboards. The design keeps registry ownership in
  app-server/state DB.
- Eagerly resuming every row would be slow and invasive. The dashboard renders
  stored metadata and only resumes when the user opens a row.

## Feasibility verdict

This is implementable inside Codex. It is not implementable as a faithful
external wrapper, and it is not currently implementable as a plugin-only feature.
The smallest end-state is a native TUI mode plus a small app-server/state
registry that records dashboard membership and view-specific workflow state.
