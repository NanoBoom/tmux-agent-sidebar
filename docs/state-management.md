# State Management Architecture

## State Scope & Update Frequency

Every piece of state belongs to one of three scopes: **Global** (shared across all sidebar instances via tmux variables), **Per-pane** (keyed by tmux pane ID), or **Local** (single sidebar process only). The table below shows where each field lives, how often it updates, and what triggers the update.

### Global State (synced via tmux global variables)

Stored in `GlobalState`. Written to tmux on change, with the cursor save
debounced briefly so selection changes do not block redraw/input handling;
reloaded on SIGUSR1.

| Field | Tmux Variable | Update Trigger | Description |
|-------|--------------|----------------|-------------|
| `status_filter` | `@sidebar_filter` | User input (left/right key) | Active status filter (All/Running/Background/Waiting/Idle/Error) |
| `selected_pane_row` | `@sidebar_cursor` | User input (j/k key); tmux write flushed after a short debounce | Cursor position in agent list |
| `repo_filter` | `@sidebar_repo_filter` | User input (repo popup) | Repository filter (All or specific repo) |

Each field has a corresponding `last_saved_*` to prevent sync conflicts — only overwrites tmux if the local write succeeded.

### Per-pane State (keyed by pane ID)

Written by `cli/hook.rs` on agent events, read by `query_sessions()` every **1 second**.

Each pane's runtime data is split into two buckets:

| Source | Update Trigger | Description |
|--------|----------------|-------------|
| tmux pane options | Event-driven + cleanup on agent exit | Agent type, status, cwd, permission mode, prompt, subagents, worktree, etc. |
| `PaneRuntimeState` in `AppState` | Refresh cycle + cleanup on agent exit | `ports`, `command`, `task_progress`, `task_dismissed_total`, `inactive_since` |

Pane options written to tmux:

| Tmux Option | Update Trigger | Description |
|-------------|----------------|-------------|
| `@pane_agent` | SessionStart | Agent type ("claude" / "codex" / "opencode") |
| `@pane_status` | Every event | Status ("running" / "background" / "waiting" / "idle" / "error") |
| `@pane_cwd` | SessionStart, CwdChanged | Working directory |
| `@pane_permission_mode` | SessionStart, hook event | Permission mode |
| `@pane_prompt` | UserPromptSubmit, Stop | Latest prompt or response text |
| `@pane_prompt_source` | UserPromptSubmit, Stop | "user" or "response" |
| `@pane_started_at` | UserPromptSubmit | Unix epoch when agent started |
| `@pane_attention` | SessionStart, Stop, StopFailure (clear); Notification, PermissionDenied, TeammateIdle (set) | "notification" or "clear" |
| `@pane_wait_reason` | StopFailure, PermissionDenied, TeammateIdle | Reason for waiting/error (`permission_denied`, `teammate_idle:<name>`, or error text) |
| `@pane_bg_cmd` | ActivityLog (bg Bash), Refresh sweep (clear), SessionEnd (clear) | Latest sanitized command of a Bash tool started with `run_in_background`. Its presence is the single source of truth for "live bg shell" — Stop routes to `background` while it is set, and the row body renders the command. Persists across UserPromptSubmit so shells spanning turns stay visible; overwritten by the next bg Bash. The refresh loop runs a `ps`-based liveness sweep each tick and clears the marker (plus downgrades `background → idle`) when no process matches the stored command. Only the most recent bg Bash is tracked; older ones are not retained. |
| `@pane_subagents` | SubagentStart/Stop | Comma-separated active subagent list |
| `@pane_worktree_name` | SessionStart | Worktree name (if applicable) |
| `@pane_worktree_branch` | SessionStart | Worktree branch (if applicable) |
| `@pane_session_id` | SessionStart, UserPromptSubmit, Notification, Stop, StopFailure, PermissionDenied, CwdChanged | Agent-reported session id (skipped when subagents are active) |

In-memory per-pane runtime state. Every field lives inside
`PaneRuntimeState` so the whole record is dropped together when its
pane disappears (`prune_pane_states_to_current_panes`).

| Field | Update Frequency | Description |
|-------|-----------------|-------------|
| `pane_states.map[...].ports` | Every 10s (port scan) | Listening localhost ports detected from the pane process tree |
| `pane_states.map[...].command` | Every 10s (port scan) | Best-effort commandline for the pane process tree, with tmux command fallback in the UI |
| `pane_states.map[...].task_progress` | Every 1s (refresh cycle) | Parsed from activity log — task list per pane |
| `pane_states.map[...].task_dismissed_total` | On task completion | Tracks dismissed completed-task counts |
| `pane_states.map[...].inactive_since` | On status change | Debounce timestamp (3s grace before hiding tasks) |
| `pane_states.map[...].tab_pref` | On user tab switch | Remembered bottom tab choice per pane (cleared on relaunch) |
| `pane_states.map[...].task_progress_log_mtime` | Every 1s (refresh cycle) | mtime of the task-progress log last parsed; skips re-parsing when unchanged |
| `pane_states.map[...].dead_scan_misses` | Every 1s and every 10s | Consecutive liveness misses for the pane. Shared by both detectors — `sweep_exited_agent_panes` (every tick, Codex/OpenCode panes that fell back to a shell) and `refresh_port_data` (every 10s, all agent panes). At `DEAD_SCAN_THRESHOLD` (2) the pane's `@pane_*` metadata and activity log are wiped and it leaves the list; any sample that finds the agent resets it to 0. A pane a detector cannot judge is left untouched, so the two never eat each other's progress. Debouncing matters because a single racy `ps` sample would otherwise make a live agent's pane vanish |

Per-pane file-based state:

| File | Update Trigger | Read Frequency | Description |
|------|---------------|----------------|-------------|
| `/tmp/tmux-agent-activity_{pane_id}.log` | Each ActivityLog event | Every 1s | Tool usage log (`HH:MM\|tool\|label`), max 200 lines |

### Local State (single sidebar process only)

| Field | Update Frequency | Description |
|-------|-----------------|-------------|
| `repo_groups` | Every 1s | Panes grouped by git repo root (built directly from `tmux::query_sessions()` output, not stored separately as a session list) |
| `focus_state.focused_pane_id` | Every 1s, plus immediately on user-initiated pane jumps | Currently focused agent pane |
| `focus_state.sidebar_focused` | Every 1s | Whether sidebar pane itself has focus |
| `focus_state.focus` | On user input | UI focus: `Filter` / `Panes` / `ActivityLog`; input also triggers an immediate redraw so focus changes appear without waiting for the next poll tick |
| `focus_state.prev_focused_pane_id` | Every 1s | Previous focused pane ID (for detecting focus changes) |
| `now` | Every 1s | Current Unix epoch |
| `scrolls.panes` | On user input / render | Agent list scroll position |
| `scrolls.git` | On user input / render | Git status scroll position |
| `activity.scroll` | On user input / render | Activity log scroll position |
| `activity.entries` | Every 1s | Focused pane's activity entries (max 50) |
| `activity.max_entries` | Once at startup | Max activity log entries to display |
| `activity.log_cache` | Every 1s | `(focused_pane_id, mtime)` of the last-rendered activity log; skips re-reads when unchanged |
| `git` | Every 2s (bg thread) | Branch, diff stats, ahead/behind, PR number |
| `bottom_tab` | On user input / auto-switch | Current bottom panel tab |
| `theme` | Once at startup | Color theme from tmux `@sidebar_color_*` variables |
| `popup` | On user input / render | `PopupState` enum: `None` / `Repo { selected, area }` / `Notices { area }` / `SpawnInput { … }` / `RemoveConfirm { … }` / `OpenWorktree { … }`. Enforces "at most one popup open" via the type system |
| `layout` | Every frame (render) | `FrameLayout` sub-struct bundling the ephemeral fields the UI rewrites every frame for click hit-testing: `pane_row_targets`, `line_to_row`, `repo_button_col`, `repo_spawn_targets`, `spawn_remove_targets`, `hyperlink_overlays` |
| `notices` | Once at startup / on copy | `NoticesState` sub-struct: `button_col`, `missing_hook_groups`, `claude_plugin_status`, `claude_settings_has_residual_hooks`, `claude_plugin_notice`, `copy_targets`, `copied_at` |
| `timers` | Refresh cycle / on user input | `RefreshTimers` sub-struct gating periodic work: `last_filter_click` (debounce), `last_port_refresh`, `port_scan_initialized` |
| `pending_osc52_copy` | On successful copy / frame flush | OSC 52 clipboard payload queued for terminal forwarding |
| `pet_state` | Every 200ms (animation) | `Idle` / `WalkRight` / `Working` / `WalkLeft` |
| `pet_x` | Every 200ms (animation) | Pet X position |
| `pet_frame` | Every 200ms (animation) | Animation frame counter |
| `pet_bob_timer` | Every 200ms (animation) | Idle bob motion timer |
| `pet_enabled` | Once at startup | Whether the pet is drawn and ticked (from `@sidebar_pet`) |
| `spinner_frame` | Every 200ms (animation) | Spinner animation frame counter |
| `icons` | Once at startup | `StatusIcons` theme (overridable via tmux options) |
| `tmux_pane` | Once at startup | This sidebar's own tmux pane ID |
| `pane_states.seen` | Every 1s | Set of pane IDs that have been seen as agents (bundled with `pane_states.map` under the `PaneRuntimeMap` wrapper) |
| `version_notice` | Once at startup (bg fetch) | GitHub release update notice, `None` when up-to-date |
| `sessions.names` | Every 10s (background thread) | `session_id → session name` map; scanned by `session_poll_loop` in `app/workers.rs` so the TUI thread never blocks on filesystem I/O. Stays empty while `show_session_name` is off — neither the thread nor the startup scan runs |
| `show_session_name` | Once at startup | Whether pane rows are titled with the agent's session name instead of the agent label (from `@sidebar_show_session_name`, default `false`) |
| `sessions.dirty` | On session map refresh / application tick | Marks the session map as changed so the per-pane session label walk only runs when needed. Bypassed while `show_session_name` is on: `apply_session_snapshot` rebuilds each `PaneInfo` with an empty `session_name` every tick, so a dirty-gated walk would leave the row title flipping between the session name and the agent label once per poll |

---

## Update Cycle Summary

```
┌─────────────────────────────────────────────────────────────┐
│  Every frame (~200ms)                                       │
│  layout.* (rebuilt by ui::draw), spinner + pet animation     │
├─────────────────────────────────────────────────────────────┤
│  Every 1s (refresh cycle)                                   │
│  repo_groups, focus_state.focused_pane_id,                  │
│  layout.pane_row_targets, activity.entries,                 │
│  pane_states.map[..].task_progress,                         │
│  Codex/OpenCode shell-fallback liveness (debounced)         │
├─────────────────────────────────────────────────────────────┤
│  Every 10s (port scan, background)                          │
│  pane_states.map[..].ports, agent liveness cleanup          │
│  (debounced — see pane_states.map[..].dead_scan_misses)     │
├─────────────────────────────────────────────────────────────┤
│  Every 10s (session_names background thread, opt-in)        │
│  sessions.names map populated by session_poll_loop          │
├─────────────────────────────────────────────────────────────┤
│  Once at startup                                             │
│  theme, bottom_panel_height, bottom_panel_enabled,          │
│  pet_enabled, show_session_name,                            │
│  notices.claude_plugin_*,                                   │
│  notices.claude_settings_has_residual_hooks,                │
│  notices.claude_plugin_notice, notices.missing_hook_groups  │
├─────────────────────────────────────────────────────────────┤
│  Every 2s (git background thread, Git tab only)             │
│  git (branch, diff, ahead/behind, PR)                       │
├─────────────────────────────────────────────────────────────┤
│  On SIGUSR1 (tmux focus change)                             │
│  GlobalState reloaded from tmux variables                   │
├─────────────────────────────────────────────────────────────┤
│  Event-driven (agent hooks)                                 │
│  @pane_* tmux options, activity log files                   │
├─────────────────────────────────────────────────────────────┤
│  On user input                                              │
│  focus_state.focus, scrolls.*, activity.scroll, bottom_tab, │
│  GlobalState fields, popup (PopupState enum),               │
│  timers.last_filter_click,                                  │
│  immediate selection / active-pane redraw                   │
├─────────────────────────────────────────────────────────────┤
│  Every frame (render)                                       │
│  layout.line_to_row, popup.area (Repo/Notices variants),    │
│  notices.button_col, notices.copy_targets,                  │
│  layout.hyperlink_overlays                                  │
└─────────────────────────────────────────────────────────────┘
```

---

## Data Flow

```
Agent hooks (hook.sh)
  → CLI `hook` subcommand (cli/hook.rs)
    → resolve_adapter() (event.rs) → adapter.parse() → AgentEvent
    → handle_event() writes @pane_* tmux options + /tmp activity log files
                        ↓
TUI main loop (app::run in app.rs; submodules app/{setup,workers,input,render})
  → startup plugin-state reads (cli/plugin_state.rs)
    → installed_plugins.json / ~/.claude/settings.json
    → initializes Claude notices state once
                        ↓
  → refresh() every 1s → RefreshOutcome { window_active, self_close }
    → get_sidebar_pane_info() (tmux/panes.rs) ← one `display-message` on our own
                                       pane: focus, geometry, and the window /
                                       session counts the self-close check needs
    → self_close ⇒ `kill-pane` on ourselves, main loop returns (see below)
    → query_sessions() (tmux.rs)     ← reads @pane_* via `tmux list-panes -a`
    → group_panes_by_repo() (group.rs)
    → rebuild_row_targets()          ← applies GlobalState filters
    → refresh_activity_data()        ← reads /tmp activity logs
    → refresh_task_progress()        ← updates PaneRuntimeState.task_progress
    → refresh_port_data()            ← updates PaneRuntimeState.ports
    → scan_session_process_snapshot() ← detects dead panes and clears stale tmux metadata
                        ↓
  → git_rx.try_recv()                ← receives GitData from background thread
  → notices popup render/copy state  ← derived from AppState plugin fields
                        ↓
  → ui::draw() renders frame         ← reads all AppState fields
```

---

## Sidebar Auto-Close

When the last non-sidebar pane leaves a window, the sidebar must go with it —
otherwise it lingers alone in an empty window. Gated by `@sidebar_auto_close`
(default on).

Two mechanisms cooperate, because no single one covers every path:

| Pane leaves via | `pane-exited` | Handled by |
| --- | --- | --- |
| shell `exit`, Ctrl-D, process killed | fires | `pane-exited` hook → `auto-close` subcommand → `kill-window` |
| `kill-pane` (prefix + x) | **does not fire** | sidebar's own refresh tick |
| `break-pane` | **does not fire** | sidebar's own refresh tick |
| `move-pane` / `join-pane` out | **does not fire** | sidebar's own refresh tick |

tmux only notifies `pane-exited` from `server_destroy_pane()`, the
process-exit path — `kill-pane` goes through `server_kill_pane()` and notifies
nothing. There is no `after-break-pane` or `after-move-pane` hook to bind at
all (they are not valid hook names), and `window-layout-changed` also fires on
every drag-resize, so it is unusable as a trigger. Hence the sidebar checks for
itself.

**Hook path** (`agent-sidebar.conf` → `cli/toggle.rs::cmd_auto_close`) — fast,
sub-tick, but only reaches the process-exit case.

**Self-close path** (`state/refresh.rs` → `app.rs`) — covers everything.
`get_sidebar_pane_info()` already runs one `display-message` on the sidebar's
own pane each second, so `#{window_panes}`, `#{session_windows}` and
`#{session_attached}` ride along for free. `window_panes == 1` means "nothing
but us" (the sidebar always counts itself), and no `@pane_role` parsing is
needed. On a confirmed hit the sidebar runs `kill-pane` on itself and
`app::run` returns `Ok(())`; `kill-pane` rather than a bare process exit so
`remain-on-exit on` does not leave a dead-pane husk. `after-kill-pane` is
wired to the same SIGUSR1 wakeup as the focus hooks, so prefix + x is noticed
immediately rather than up to 1s late.

**Why there is no debounce** — `RefreshOutcome::self_close` is acted on the
first time it comes back true. An earlier revision required the condition to
hold for a 250ms grace period first, to protect a "kill a pane, then create a
replacement" sequence: two separate `tmux` invocations, tens of milliseconds
apart, whose midpoint looks exactly like an abandoned window. Killing the
sidebar there destroys the window and the follow-up `split-window` fails, so
the user loses the whole workspace.

It was dropped because the cost was disproportionate to what it bought. What
decides the race is simply *when the sidebar pulls the trigger*, since a
respawn that lands before that is safe and one that lands after is not:

| | trigger fires at | close latency (`prefix + x` → window gone) |
| --- | --- | --- |
| 250ms grace | kill + ~265ms | ~320ms |
| no grace | kill + one refresh | ~50ms, ~150ms on the 10s port-scan tick |

The grace period never made the race *correct* — a respawn slower than 265ms
lost the window anyway. It bought a wider bet, not a guarantee, and charged
every ordinary close 250ms for it. Measured respawn gaps on the current build
(5 runs each): ≤15ms always intact, ≥100ms always loses the window, 20–60ms is
the jitter band where the outcome depends on how much work that refresh tick
had to do. So the safe gap dropped from ~265ms to ~15ms, not to zero, but it
is genuinely small — do not read "no debounce" as "there is still some slack".

Nothing in this repository does kill-then-respawn (`cli/toggle.rs` kills
sidebars, `app.rs` kills only itself), so the exposure is limited to external
scripts that rebuild a pane within ~15ms of destroying the last one. tmux's own
`respawn-pane` does not destroy the pane and never enters this path.

What did survive the removal is a **re-read**, which is not the same thing as a
wait. `refresh()` samples the pane counts in its first call and then spends the
rest of the tick on `list-panes -a`, the process scan and the activity logs, so
`RefreshOutcome::self_close` is tens to hundreds of ms out of date by the time
the event loop sees it. Acting on that stale sample lets a pane created
mid-refresh lose its sidebar (the window itself survives — `kill-pane` on
ourselves only destroys the window when we really are the last pane). One
`display-message` before the kill moves the decision from the start of the
refresh to its end; with a respawn landing in between (35ms gap, 6 runs) that
took the fully-intact outcome from 1/6 to 5/6. It narrows the window rather
than closing it — at a 50ms gap, where the respawn and the re-read collide,
the two builds are indistinguishable. Costs ~5ms, only on a tick that already
wants to close.

If the trade-off ever needs revisiting, reinstate the wait as an absolute
deadline (`alone_since + GRACE`), not a relative one. The original made the
event loop's next-refresh countdown relative to `last_refresh`, so any SIGUSR1
arriving inside the grace window pushed the confirming tick out by a further
full grace period — a wakeup at 240ms delayed the close to ~550ms, and the
focus hooks fire exactly such wakeups.

Both paths share `tmux::session_safe_to_close()`: tearing down the last window
of a session destroys the session and drops every attached client, so that is
only allowed with at most one client attached. Any query returning `None` (pane
gone, tmux busy) reads as "cannot prove this is safe" and preserves the sidebar
— a lingering sidebar is always better than a mass-disconnect.

Note the consequence of that "at most one client" rule: in a **single-window
session** the auto-close *does* end the session and disconnect the one attached
client, exactly as tmux itself does when the last pane of the last window
exits. That is intentional — the alternative is stranding a sidebar in a window
with nothing to monitor — but it means `prefix + x` on the last agent pane can
close the whole session, not just a pane. `@sidebar_auto_close off` opts out.

**One parser for the option, read live.** `@sidebar_auto_close` is read only by
the binary, and truthiness has exactly one definition —
`tmux::parse_bool_option` (`on`/`true`/`1`/`yes`), shared by the global option
map (`ui::bool_option`) and by the format expansion below.
`agent-sidebar.conf` registers the `pane-exited` hook unconditionally and lets
`cmd_auto_close` decide, so the config file never reimplements what counts as
truthy — a value like `false` or `no` can't disable one half of the feature
while leaving the other running.

The sidebar's own check reads the option live rather than caching it at
startup: `#{@sidebar_auto_close}` is appended to the same `display-message`
that already runs each tick, so it costs nothing and toggling the option needs
neither a config reload nor a sidebar restart. That last field is the only
user-controlled one in the format, hence the `|` separator — an option value
containing spaces would break a space-separated parse. An empty expansion means
unset, which reads as the documented default (on).

---

## Key Types

```rust
enum Focus { Filter, Panes, ActivityLog }
enum StatusFilter { All, Running, Background, Waiting, Idle, Error }
enum RepoFilter { All, Repo(String) }
enum BottomTab { Activity, GitStatus }
enum PaneStatus { Running, Background, Waiting, Idle, Error, Unknown }
enum AgentType { Claude, Codex, OpenCode, Unknown }
enum PermissionMode { Default, Plan, AcceptEdits, Auto, DontAsk, BypassPermissions, Defer }

/// At-most-one popup state. The enum encodes both which popup is open
/// and its per-popup data so the invariant is checked by the type system.
enum PopupState {
    None,
    Repo { selected: usize, area: Option<Rect> },
    Notices { area: Option<Rect> },
    /// Modal text input shown when the user spawns a new worktree.
    SpawnInput {
        input: String,
        target_repo: String,
        target_repo_root: String,
        /// Editor command, seeded from `@sidebar_editor`. Non-empty ⇒
        /// the new window is split (editor left, agent right).
        editor: String,
        pick: AgentPick,
        field: SpawnField,   // Task | Editor | Agent | Mode
        anchor_y: Option<u16>,
        error: Option<String>,
        area: Option<Rect>,
    },
    /// Confirmation prompt shown when the user removes a sidebar-spawned pane.
    RemoveConfirm {
        pane_id: String,
        branch: String,
        /// Worktree has uncommitted work, sampled when the modal opens.
        /// Greys out `[y]` and makes `confirm_remove` refuse it.
        dirty: bool,
        error: Option<String>,
        area: Option<Rect>,
    },
    /// Two-step modal for opening an EXISTING worktree (`o`): pick the
    /// worktree, then pick the agent + mode. One variant holds both
    /// steps so `rows` survives the transition and `Esc` can walk back
    /// without re-running `git worktree list`.
    OpenWorktree {
        target_repo_root: String,
        rows: Vec<OpenWorktreeRow>,
        selected: usize,
        scroll: usize,
        step: OpenStep,       // Pick | Configure
        editor: String,       // same contract as SpawnInput's
        pick: AgentPick,
        field: OpenField,     // Editor | Agent | Mode (default Editor)
        anchor_y: Option<u16>,
        error: Option<String>,
        area: Option<Rect>,
    },
}

/// Agent + permission-mode selection shared by `SpawnInput` and
/// `OpenWorktree`. Owns the cycle rules (agent wrap resets the mode,
/// mode wraps against `worktree::modes_for(agent)`).
struct AgentPick {
    agent_idx: usize,
    mode_idx: usize,
}

/// One row of the open-worktree picker. `branch` feeds the window's
/// branch marker; `label` is the display string (`(detached <sha7>)`
/// when there is no branch). `in_use` is derived from `repo_groups`
/// pane paths, so it costs no extra tmux or git call.
struct OpenWorktreeRow {
    path: String,
    branch: String,
    label: String,
    in_use: bool,
}

struct ScrollState {
    offset: usize,
    total_lines: usize,
    visible_height: usize,
}

struct HyperlinkOverlay {
    x: u16,
    y: u16,
    text: String,
    url: String,
}

struct PaneRuntimeState {
    ports: Vec<u16>,
    command: Option<String>,
    task_progress: Option<TaskProgress>,
    task_dismissed_total: Option<usize>,
    inactive_since: Option<u64>,
    tab_pref: Option<BottomTab>,
    task_progress_log_mtime: Option<SystemTime>,
}

/// Wraps `PaneRuntimeState` per pane plus the set of pane IDs that
/// have been seen as agents. Methods delegate to the underlying
/// `HashMap`; `seen` is read/written alongside `map` during refresh.
struct PaneRuntimeMap {
    map: HashMap<String, PaneRuntimeState>,
    seen: HashSet<String>,
}

/// Focus-related fields grouped so UI code can pass them as a single
/// sub-struct rather than juggling four flat fields.
struct FocusState {
    sidebar_focused: bool,
    focus: Focus,
    focused_pane_id: Option<String>,
    prev_focused_pane_id: Option<String>,
}

/// Non-activity scrolls (the agent list and the git bottom panel).
/// Activity's scroll lives inside `ActivityState` because it pairs
/// with the activity entries buffer.
struct ScrollStates {
    panes: ScrollState,
    git: ScrollState,
}

/// Activity-log snapshot for the focused pane plus cache metadata so
/// the polling tick can skip redundant file reads.
struct ActivityState {
    entries: Vec<ActivityEntry>,
    scroll: ScrollState,
    max_entries: usize,
    log_cache: Option<(String, SystemTime)>,
}

/// Session-name map scanned by a background thread so the TUI thread
/// never blocks on `~/.claude/sessions/*.json` reads.
struct SessionNamesState {
    names: HashMap<String, String>,
    dirty: bool,
}

/// Frame-scoped render output cached for click hit-testing. Rewritten
/// every frame by the UI layer; consumed by mouse/keyboard handlers
/// before the next render.
struct FrameLayout {
    pane_row_targets: Vec<RowTarget>,
    line_to_row: Vec<Option<usize>>,
    repo_button_col: Option<u16>,
    repo_spawn_targets: Vec<RepoSpawnTarget>,
    spawn_remove_targets: Vec<SpawnRemoveTarget>,
    hyperlink_overlays: Vec<HyperlinkOverlay>,
}

/// Periodic-refresh bookkeeping. session_names refresh is intentionally
/// NOT here — it lives in a dedicated background thread so the TUI
/// thread never performs blocking filesystem I/O.
struct RefreshTimers {
    last_filter_click: Instant,
    last_port_refresh: Instant,
    port_scan_initialized: bool,
}

/// All fields for the ⓘ notices button and its popup.
struct NoticesState {
    button_col: Option<u16>,
    missing_hook_groups: Vec<NoticesMissingHookGroup>,
    claude_plugin_status: ClaudePluginStatus,
    claude_settings_has_residual_hooks: bool,
    claude_plugin_notice: Option<ClaudePluginNotice>,
    copy_targets: Vec<NoticesCopyTarget>,
    copied_at: Option<(String, Instant)>,
}
```

---

## State Invariants

1. `selected_pane_row` is always < `layout.pane_row_targets.len()` — clamped in `rebuild_row_targets()`
2. `activity.entries` contains only the focused pane's entries — cleared on focus change
3. Tab preferences persist per pane in `PaneRuntimeState.tab_pref` and are restored on focus change. They vanish together with the rest of `PaneRuntimeState` when the pane is pruned, so a relaunched agent starts on the default tab
4. Git fetching respects the `git_tab_active` flag — stops when tab is hidden
5. Task progress has a 3-second debounce — prevents flicker when agent briefly pauses
6. Global state syncs via tmux variables — enables coordination across sidebar instances
7. Scroll positions are independent per panel — agents, activity, git each have their own `ScrollState`
8. `layout.line_to_row` is rebuilt every frame — ensures accurate click routing
9. Pane runtime state is pruned when the pane disappears — prevents stale per-pane ports, task progress, and tab preferences from surviving after the agent is gone
10. At most one popup is open at a time — enforced structurally by the `PopupState` enum, not by parallel boolean flags
10. Hook-based cleanup wins when available; pid-based cleanup is a slower fallback that removes panes when the agent process is gone but the hook did not fire
