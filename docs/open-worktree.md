# Open Worktree — Implementation Plan

Companion feature to the existing `n` (spawn worktree) flow: instead of
creating a *new* worktree, `o` lets the user pick an **existing** git
worktree of the selected pane's repository and launch an agent in it.

## 1. UX flow

```
sidebar (Focus::Panes)
        │  press `o`
        ▼
┌──────────────────────────┐   Esc → close
│ Step 1 — Pick worktree   │
│  ● agent/login-fix       │   j/k/↑/↓/Ctrl-n/Ctrl-p → move
│    agent/refactor-db     │   Enter → step 2
│    main                  │
└──────────────────────────┘
        │  Enter
        ▼
┌──────────────────────────┐   Esc → back to step 1
│ Step 2 — Agent + Mode    │
│  BRANCH  agent/login-fix │   Tab/↑/↓ → move field
│  AGENT   claude          │   ←/→ → cycle value
│  MODE    plan            │   Enter → open
└──────────────────────────┘
        │  Enter
        ▼
tmux new-window -c <worktree path> → markers → send agent command
```

Trigger conditions mirror `n`: only when `state.focus_state.focus ==
Focus::Panes` and a pane is selected. The repo is resolved from the
selected pane's `RepoGroup` — the same lookup
`open_spawn_input_from_selection` already performs, so a pane that is
itself inside a worktree still resolves to the main repo root (see
`group::resolve_pane_git_info`, which keys groups off `--git-common-dir`).

The `●` marker on a row means "a tmux pane is already running in this
worktree". It is informational only — `Enter` still opens a new window.
Computed from `state.repo_groups` pane paths, so it costs no extra tmux
or git calls.

## 2. Design decisions

| Decision | Choice | Why |
| --- | --- | --- |
| One popup or two? | **One** `PopupState::OpenWorktree` with a `step` field | The worktree list must survive the step transition (step 2 renders the picked branch, `Esc` returns to step 1). Two variants would force re-running `git worktree list` or duplicating the list into both. |
| Field enum | New `OpenField { Agent, Mode }` | Step 2 has no text input; reusing `SpawnField` would make the unreachable `Task` variant representable. |
| Agent/mode duplication | Extract `AgentPick { agent_idx, mode_idx }` used by both `SpawnInput` and `OpenWorktree` | The cycle logic (agent wrap + mode reset, mode wrap against `modes_for(agent)`) is non-trivial and already tested; sharing it keeps one source of truth. |
| tmux markers | ~~Set everything **except** `@agent-sidebar-spawned`~~ → **superseded**: set the full spawn marker set **plus** `@agent-sidebar-opened=1` | Original plan withheld `@agent-sidebar-spawned` so `x` could not delete a user-created worktree. In practice that also withheld `[c] close window only`, which touches no git — leaving no way to close the window from the sidebar at all. `x` now behaves identically for opened and spawned windows; `@agent-sidebar-opened` remains as the provenance record. See §8.1. |
| git call threading | Synchronous, on the key-handler path | `worktree::spawn`, `git::repo_root` and `group::resolve_pane_git_info` already run sync git from the UI thread. `git worktree list --porcelain` is a single cheap local read; adding a worker thread here would be inconsistent with the surrounding code for no measurable gain. |
| Already-open worktrees | Show `●`, still create a new window | Matches the requested flow. "Jump to the existing pane instead" is a viable alternative but changes what `Enter` means depending on state; left as a follow-up. |
| Prunable / missing worktrees | Filtered out of the list | `tmux new-window -c <missing dir>` fails with an opaque error; better to not offer the row. |
| Bare worktree entry | Filtered out | Cannot host a checkout. |
| Main worktree | **Included**, labelled by its branch | Launching an agent on `main` is a legitimate target and the natural "open the repo itself" entry. |

## 3. Module map

```
src/git.rs                          + WorktreeEntry, worktree_list(), parse_worktree_list()
src/worktree/config.rs              (unchanged — AGENTS / modes_for / agent_command reused)
src/worktree/flow.rs                + OpenRequest, open(), open_with()
src/worktree/markers.rs             + OPENED_OPTION, SpawnMarkers::opened
src/worktree.rs                     + re-exports
src/state/popup.rs                  + PopupState::OpenWorktree, AgentPick, OpenField
src/state/popup/open_worktree.rs    NEW — all AppState methods for the flow
src/state/layout.rs                 (unchanged)
src/state.rs                        + pub use popup::{AgentPick, OpenField}
src/app/input.rs                    + `o` binding, + popup key routing
src/state/layout.rs::handle_mouse_click  + popup click routing
src/ui/panes/open_worktree.rs       NEW — render_open_worktree_popup
src/ui/panes.rs                     + pub(super) helpers reuse, mod decl
src/ui/panes/popups.rs              + dispatch arm
```

## 4. New code, concretely

### 4.1 `src/git.rs` — list worktrees

```rust
/// One entry of `git worktree list --porcelain`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: String,
    /// Short branch name (`refs/heads/` stripped). Empty when detached.
    pub branch: String,
    pub head: String,
    pub bare: bool,
    pub detached: bool,
    pub locked: bool,
    pub prunable: bool,
}

/// `git worktree list --porcelain` from inside `repo`.
pub fn worktree_list(repo: &str) -> Result<Vec<WorktreeEntry>, String> {
    run_git_capture(repo, &["worktree", "list", "--porcelain"]).map(|out| parse_worktree_list(&out))
}

pub(crate) fn parse_worktree_list(text: &str) -> Vec<WorktreeEntry> { /* … */ }
```

Parser contract (records separated by blank lines; keys are
space-separated, value optional):

| Line | Effect |
| --- | --- |
| `worktree <path>` | starts a new record |
| `HEAD <sha>` | `head` |
| `branch refs/heads/<name>` | `branch` = `<name>` |
| `detached` | `detached = true` |
| `bare` | `bare = true` |
| `locked` / `locked <reason>` | `locked = true` |
| `prunable` / `prunable <reason>` | `prunable = true` |

Split out as a pure function so it is unit-testable without a repo —
same pattern as `parse_status_short` / `parse_diff_stat`.

### 4.2 `src/worktree/markers.rs`

```rust
pub const OPENED_OPTION: &str = "@agent-sidebar-opened";
```

Append `#{@agent-sidebar-opened}` as a **sixth** line of
`spawn_markers_template()` and a matching `pub opened: bool` field on
`SpawnMarkers`. Order matters: append last so the existing five-field
parse order (and its tests) is untouched. `is_spawned()` keeps its
current meaning; add `is_opened()` alongside it.

### 4.3 `src/worktree/flow.rs`

```rust
#[derive(Debug, Clone)]
pub struct OpenRequest {
    pub repo_root: PathBuf,
    pub worktree_path: PathBuf,
    pub branch: String,
    pub session: String,
    pub agent: String,
    pub mode: String,
}

/// Open an EXISTING worktree in a new tmux window and launch the agent.
/// Unlike `spawn` this creates no git state, so the failure path only
/// has to kill the window it created — there is nothing to roll back
/// on disk.
pub fn open(req: &OpenRequest) -> Result<(), String> { open_with(&RealEnv, req) }

pub(crate) fn open_with<E: SpawnEnv>(env: &E, req: &OpenRequest) -> Result<(), String>
```

Sequence, mirroring `spawn_with` minus the git steps:

1. Validate `worktree_path` is UTF-8 and non-empty.
2. `env.new_window(&req.session, worktree, window_name)` where
   `window_name` = worktree directory basename.
3. Set window options: `SPAWNED_OPTION=1`, `OPENED_OPTION=1`,
   `SPAWNED_FROM_OPTION=repo`, `SPAWNED_WORKTREE_OPTION=worktree`,
   `SPAWNED_BRANCH_OPTION=branch`. On failure →
   `env.kill_window(window_id)`, return composed error.
   (`SPAWNED_OPTION` was added after the fact — see §2 and §8.1.)
4. `env.send_command(&pane_id, &agent_command(&req.agent, &req.mode))`.
   On failure → `env.kill_window(window_id)`, return composed error.

No new `SpawnEnv` methods are required — `new_window`,
`set_window_option`, `send_command` and `kill_window` all already exist,
so the existing `FakeEnv` in `flow.rs` drives the new tests unchanged.
Reuse `compose_spawn_error` for the "window kill also failed" message.

### 4.4 `src/state/popup.rs` — shared agent/mode picker

```rust
/// Agent + permission-mode selection shared by the spawn and open
/// worktree modals. `agent_idx` indexes `worktree::AGENTS`; `mode_idx`
/// indexes `worktree::modes_for(agent)`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AgentPick {
    pub agent_idx: usize,
    pub mode_idx: usize,
}

impl AgentPick {
    pub fn agent(&self) -> &'static str;      // AGENTS[idx], "" when OOB
    pub fn mode(&self) -> &'static str;       // modes_for(agent)[idx], "" when OOB
    pub fn cycle_agent(&mut self, delta: isize);  // wraps, resets mode_idx = 0
    pub fn cycle_mode(&mut self, delta: isize);   // wraps within modes_for(agent)
}
```

`PopupState::SpawnInput` swaps its `agent_idx` / `mode_idx` fields for a
single `pick: AgentPick`, and `spawn_input_cycle` delegates. The
existing spawn tests are updated to read `pick.agent_idx` — behaviour is
unchanged.

### 4.5 `src/state/popup.rs` — new variant

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OpenField {
    #[default]
    Agent,
    Mode,
}
impl OpenField { pub fn next(self) -> Self; pub fn prev(self) -> Self; }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenStep { Pick, Configure }

/// Row shown in the picker: an existing worktree plus the sidebar-local
/// "is a pane already running here" flag.
#[derive(Debug, Clone)]
pub struct OpenWorktreeRow {
    pub path: String,
    pub branch: String,  // short branch, empty when detached
    pub label: String,   // branch, or "(detached <sha7>)"
    pub in_use: bool,
}

// added to PopupState
OpenWorktree {
    target_repo_root: String,
    rows: Vec<OpenWorktreeRow>,
    selected: usize,
    /// First visible row — the list scrolls when it exceeds the popup.
    scroll: usize,
    step: OpenStep,
    pick: AgentPick,
    field: OpenField,
    anchor_y: Option<u16>,
    error: Option<String>,
    area: Option<ratatui::layout::Rect>,
},
```

Plus `PopupState::set_open_worktree_area`, matching the four existing
`set_*_area` helpers.

### 4.6 `src/state/popup/open_worktree.rs` — AppState methods

`src/state/popup.rs` becomes `popup.rs` + a `popup/` directory, matching
the `worktree.rs` + `worktree/` and `ui/panes.rs` + `ui/panes/` pattern
already used in this repo. Keeps the ~870-line `popup.rs` from doubling.

```rust
impl AppState {
    pub fn is_open_worktree_open(&self) -> bool;
    pub fn open_worktree_popup_area(&self) -> Option<Rect>;

    /// `o` entry point. Resolves the repo from the selection, runs
    /// `git worktree list`, filters, and opens the picker. Flashes and
    /// returns without opening a popup on every failure path.
    pub fn open_worktree_from_selection(&mut self);

    pub fn close_open_worktree(&mut self);
    pub fn open_worktree_move(&mut self, delta: isize);       // clamped, updates scroll
    pub fn open_worktree_next_field(&mut self);
    pub fn open_worktree_prev_field(&mut self);
    pub fn open_worktree_cycle(&mut self, delta: isize);
    /// Enter: Pick → Configure, or Configure → run the open flow.
    pub fn confirm_open_worktree(&mut self);
    /// Esc: Configure → Pick, or Pick → close.
    pub fn open_worktree_back(&mut self);
    pub fn open_worktree_select_row(&mut self, idx: usize);   // mouse
}
```

`open_worktree_from_selection` failure flashes (same wording style as
the spawn path):

| Condition | Flash |
| --- | --- |
| no pane selected | `open: no pane selected` |
| no repo group for selection | `open: could not find repo group for selection` |
| group has no `repo_root` | `open: selected pane is not in a git repo` |
| `git worktree list` errors | `open: git: <stderr>` |
| all entries filtered out | `open: no worktrees found` |

Row construction:

```rust
let kept = entries
    .into_iter()
    .filter(|e| !e.bare && !e.prunable && !e.path.is_empty() && Path::new(&e.path).exists())
    .collect::<Vec<_>>();

// NOT plain containment: linked worktrees live *under* the main
// checkout (`<repo>/.worktrees/<slug>`), so `starts_with` alone marks
// the main row in use for every pane in every worktree. Each pane
// belongs to the LONGEST worktree path containing it.
let owns = |worktree: &str, pane_path: &str| {
    kept.iter()
        .filter(|w| worktree_contains(&w.path, pane_path))
        .max_by_key(|w| w.path.len())
        .is_some_and(|w| w.path == worktree)
};

rows = kept.iter()
    .map(|e| OpenWorktreeRow {
        in_use: pane_paths.iter().any(|p| owns(&e.path, p)),
        label: if e.branch.is_empty() {
            format!("(detached {})", &e.head[..7.min(e.head.len())])
        } else { e.branch.clone() },
        branch: e.branch.clone(),
        path: e.path.clone(),
    })
    .collect();
```

`pane_paths` comes from `state.repo_groups` (already in memory).

`confirm_open_worktree` in `Configure` step builds the `OpenRequest`
exactly like `confirm_spawn_input` does — resolving the session via
`crate::tmux::pane_session_name(&self.tmux_pane)` and surfacing errors
inline via `error` so the modal stays open for a retry.

### 4.7 `src/app/input.rs`

Insert a routing block **before** the `is_remove_confirm_open` block
(order among the popup guards is arbitrary — they are mutually
exclusive — but keeping picker-style popups adjacent reads better):

```rust
if state.is_open_worktree_open() {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc                                  => state.open_worktree_back(),
        KeyCode::Enter                                => state.confirm_open_worktree(),
        // Step 1 navigation / step 2 field movement — both handled by
        // the state methods, which no-op for the wrong step.
        KeyCode::Char('j') | KeyCode::Down            => state.open_worktree_nav(1),
        KeyCode::Char('n') if ctrl                    => state.open_worktree_nav(1),
        KeyCode::Char('k') | KeyCode::Up              => state.open_worktree_nav(-1),
        KeyCode::Char('p') if ctrl                    => state.open_worktree_nav(-1),
        KeyCode::Tab                                  => state.open_worktree_nav(1),
        KeyCode::BackTab                              => state.open_worktree_nav(-1),
        KeyCode::Left                                 => state.open_worktree_cycle(-1),
        KeyCode::Right                                => state.open_worktree_cycle(1),
        _ => {}
    }
    return true;
}
```

`open_worktree_nav(delta)` dispatches on `step`: `Pick` → move the list
selection, `Configure` → `next_field` / `prev_field`. Single entry point
keeps the key table flat and means `j`/`k` work in step 2 as well (there
is no text field to shadow them).

New binding in the main key table:

```rust
KeyCode::Char('o') => {
    if state.focus_state.focus == Focus::Panes {
        state.open_worktree_from_selection();
    }
}
```

### 4.8 `src/state/layout.rs::handle_mouse_click`

Add a guard alongside the existing four. **Required**, not optional —
without it a click while the picker is open falls through to the pane
rows and activates a pane behind the modal.

```rust
if state.is_open_worktree_open() {
    if let Some(area) = self.open_worktree_popup_area()
        && point_in_rect(row, col, area)
    {
        // Step 1 only: map the row to a list index the same way the
        // repo popup does, skipping the top border/title row.
        if row > area.y { self.open_worktree_select_row((row - area.y - 1) as usize); }
        return;
    }
    self.close_open_worktree();
    return;
}
```

### 4.9 `src/ui/panes/open_worktree.rs`

`pub(super) fn render_open_worktree_popup(frame, state, area)`, dispatched
from `popups.rs::render_if_open`. Reuses `center_popup`, `anchor_below`,
`truncate_to_width`, `display_width` and `tail_fit` from `ui/panes.rs`
(bump them to `pub(super)`).

**Step `Pick`** — bordered list, title `" Open worktree "`:

- width `area.width.min(32)`, height `rows.len().min(visible) + borders`,
  clamped to `area` (`center_popup` already clamps; the picker also has
  to clamp `visible` against `area.height`).
- one row per worktree: `"● branch"` / `"  branch"`, truncated to the
  inner width; selected row uses `theme.selection_bg` + `theme.text_active`,
  others `theme.text_muted`, matching `render_repo_popup`.
- `scroll` from popup state drives the visible slice.

**Step `Configure`** — same expanded/compact split as the spawn modal
(`SPAWN_MODAL_EXPANDED_MIN_HEIGHT`), with rows `BRANCH` (read-only,
`theme.text_muted`) / `AGENT` / `MODE`, plus the optional error row in
`theme.status_error`.

Anchoring uses the same `anchor_y` the spawn modal does (the repo header
row from `layout.repo_spawn_targets`), so `n` and `o` modals appear in
the same place.

## 5. Implementation order

1. `git::WorktreeEntry` + `parse_worktree_list` + `worktree_list` **+ tests**.
2. `markers.rs`: `OPENED_OPTION`, sixth template line, `opened` field,
   `is_opened()` **+ tests**.
3. `flow.rs`: `OpenRequest` + `open_with` **+ FakeEnv tests**.
4. `state/popup.rs`: `AgentPick` extraction, spawn migrated to it **+
   existing spawn tests updated** (must stay green before moving on).
5. `state/popup/open_worktree.rs`: variant + all `AppState` methods **+ tests**.
6. `app/input.rs` binding + routing **+ tests**.
7. `state/layout.rs` mouse guard **+ test**.
8. `ui/panes/open_worktree.rs` + `popups.rs` dispatch.
9. `tests/ui_snapshot.rs` inline snapshots.
10. Docs (§7).
11. `cargo fmt && cargo clippy && cargo test && cargo build --release`.

Steps 1–3 are independent of 4–5 and can land as a separate commit; the
feature is only user-visible from step 6 on.

## 6. Test plan

Per `CLAUDE.md`, **every test that renders a frame uses
`insta::assert_snapshot!(output, @"…")`** — no `contains` assertions on
rendered output.

**`src/git.rs`** (unit)
- porcelain with main + two worktrees → 3 entries, branches stripped of `refs/heads/`
- detached record → `detached == true`, `branch` empty
- bare record → `bare == true`
- `locked <reason>` / `prunable <reason>` with and without a reason
- empty string, trailing newlines, `\n\n\n` runs → no panic, no empty records

**`src/worktree/flow.rs`** (FakeEnv)
- `open_happy_path_sets_markers_then_sends_command` — asserts the call
  order `new_window` → 4× `set_window_option` → `send_command`
- `open_never_touches_git` — no `worktree_add` / `worktree_remove` /
  `branch_delete` in `calls()` on any path (this is the safety property
  that separates `open` from `spawn`)
- `open_kills_window_when_marker_fails` (at index 0 and at index 2)
- `open_kills_window_when_send_command_fails`
- `open_surfaces_rollback_failure_when_kill_window_also_fails`
- `open_rejects_empty_worktree_path`

**`src/state/popup.rs`** (`AgentPick`)
- `cycle_agent` wraps both directions and resets `mode_idx`
- `cycle_mode` wraps within `modes_for(agent)` and is a no-op on an
  empty mode list
- existing spawn popup tests still pass against the new field

**`src/state/popup/open_worktree.rs`**
- `open_worktree_from_selection` with no selection → flash, no popup
- ... with a selection whose group has no `repo_root` → flash, no popup
- nav clamps at both ends; `scroll` follows the selection past the
  visible window
- `Enter` in `Pick` → `step == Configure`, `field` at default (`pick` is
  left alone — it only starts at `AgentPick::default()` when the modal
  opens)
- `Esc` in `Configure` → back to `Pick`, selection preserved
- `Pick` → `Configure` → `Esc` → `Configure` preserves the agent/mode
  pick as well as the row
- `Esc` in `Pick` → `PopupState::None`
- `open_worktree_cycle` is a no-op in `Pick`
- `confirm_open_worktree` on an empty `rows` → error set, popup stays open
- `open_worktree_select_row` clamps out-of-range indices, including a
  click on the popup's bottom border (`idx == visible`, which is inside
  the popup *area* but past the last drawn row)

**`src/app/input.rs`**
- `o` in `Focus::Panes` calls the entry point (assert on the flash, since
  a bare `AppState` has no repo groups — mirrors the existing
  `bare_n_does_not_move_selection` test)
- `o` in `Focus::Filter` is inert
- while the picker is open, `j`/`k`/`Ctrl-n`/`Ctrl-p` move the selection
  and do **not** leak to pane navigation

**`tests/ui_snapshot.rs`** (inline insta)
- picker default state (3 rows, first selected)
- picker with `●` in-use marker
- picker with the selection moved down
- picker scrolled (rows > visible height)
- configure step, expanded layout
- configure step, compact layout (short agents area)
- configure step with an inline error row
- narrow sidebar width (the 14-column clamp path)

## 7. Documentation updates

- `website/src/content/docs/reference/keybindings.md`
  - Worktree table: `| o | Open an existing worktree with an agent |`
  - New "Open worktree modal" section covering both steps.
- `docs/state-management.md`
  - line 90: add `OpenWorktree { … }` to the `PopupState` enum summary.
  - the `enum PopupState` snippet around line 200: add the variant.
- `README.md` — extend the worktree bullet (line 20) to mention opening
  existing worktrees.
- `website/src/content/docs/reference/tmux-options.md` — document
  `@agent-sidebar-opened` if that page lists marker options.

## 8. Out of scope / follow-ups

1. ~~**`x` on an opened window.**~~ **Done, differently than planned.**
   The original deferral assumed the choice was "refuse" vs "window-only
   modal". Refusing turned out to be untenable: it left no way to close
   the window from the sidebar, and the message ("not spawned by
   sidebar") was false from the user's point of view — the sidebar did
   create that window. `open_with` now writes `@agent-sidebar-spawned`
   too, so `x`, the `×` click marker and `remove_with` all treat opened
   and spawned windows identically, `[y]` included. Two consequences
   handled alongside:
   - `remove_with`'s `worktree_path` / `branch` emptiness checks moved
     into the `WindowAndWorktree` arm; a worktree opened on a detached
     HEAD has no branch, and requiring one blocked even `[c]`.
     `is_opened()` distinguishes that legitimate case from a corrupt
     spawn marker set.
   - The remove modal's title now comes from the branch marker, not the
     worktree directory basename. They coincide for spawned worktrees
     but not for opened ones, and `[y]` must name what it deletes.
   - **`[y]` is refused on a dirty worktree.** Naming the branch tells
     the user *what* gets deleted but not *what is in it*, and
     `git worktree remove --force` discards staged, unstaged and
     untracked work with no undo. `PopupState::RemoveConfirm` carries a
     `dirty` flag (sampled from `git status --porcelain` when the modal
     opens); the renderer replaces `[y] remove worktree` with a muted
     `[y] remove — blocked` under an `! uncommitted changes` warning,
     `confirm_remove` answers `commit or stash first` instead of
     running, and `remove_with` re-checks through
     `SpawnEnv::worktree_is_dirty` so the guarantee does not rest on the
     UI having refreshed. The rule is deliberately not restricted to
     `o`-opened worktrees — losing an agent's uncommitted output from a
     spawned worktree is the same loss. `[c]` touches no git and stays
     available throughout.
2. **Jump-instead-of-open** when `in_use` is true.
3. **`open` CLI subcommand**, symmetric with `spawn`.
4. **Async worktree listing** if `git worktree list` ever shows up as a
   frame hitch on very large repos.
