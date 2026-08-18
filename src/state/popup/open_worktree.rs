//! `AppState` methods driving the two-step "open an existing worktree"
//! modal (`o`). Split out of `popup.rs` so that file does not double in
//! size, matching the `worktree.rs` + `worktree/` and `ui/panes.rs` +
//! `ui/panes/` pattern used elsewhere in this repo.

use std::path::Path;

use ratatui::layout::Rect;

use super::{OpenField, OpenStep, OpenWorktreeRow, PopupState};
use crate::state::AppState;

/// Border rows the picker's popup spends on its frame. Used to derive
/// the visible row count from the rendered area when scrolling.
const PICKER_BORDER_ROWS: u16 = 2;

/// Whether `pane_path` sits inside `worktree` (the directory itself, or
/// anything below it). Compares against `worktree/` rather than
/// `worktree` so a sibling like `/repo/.worktrees/login-2` does not
/// count as being inside `/repo/.worktrees/login`.
fn worktree_contains(worktree: &str, pane_path: &str) -> bool {
    pane_path == worktree || pane_path.starts_with(&format!("{worktree}/"))
}

/// Turn a `git worktree list` result into picker rows. Pure so the
/// filtering rules (drop bare / prunable / vanished worktrees) and the
/// detached-HEAD label are testable without a repo.
///
/// `pane_paths` are the working directories of every pane the sidebar
/// currently knows about. `exists` is injected so tests do not have to
/// touch disk.
fn build_rows(
    entries: Vec<crate::git::WorktreeEntry>,
    pane_paths: &[String],
    exists: &dyn Fn(&str) -> bool,
) -> Vec<OpenWorktreeRow> {
    // A bare entry cannot host a checkout, and `tmux new-window -c` on a
    // prunable / missing directory fails with an opaque error — better
    // to not offer the row at all.
    let kept: Vec<crate::git::WorktreeEntry> = entries
        .into_iter()
        .filter(|e| !e.bare && !e.prunable && !e.path.is_empty() && exists(&e.path))
        .collect();

    // Linked worktrees usually live *under* the main checkout
    // (`<repo>/.worktrees/<slug>` by default), so plain containment
    // would light up the main row for every pane in every worktree.
    // Each pane belongs to the LONGEST worktree path containing it —
    // the one git itself would resolve — and only that row is in use.
    let owns = |worktree: &str, pane_path: &str| {
        kept.iter()
            .filter(|w| worktree_contains(&w.path, pane_path))
            .max_by_key(|w| w.path.len())
            .is_some_and(|w| w.path == worktree)
    };

    kept.iter()
        .map(|e| OpenWorktreeRow {
            in_use: pane_paths.iter().any(|p| owns(&e.path, p)),
            label: if e.branch.is_empty() {
                format!("(detached {})", &e.head[..7.min(e.head.len())])
            } else {
                e.branch.clone()
            },
            branch: e.branch.clone(),
            path: e.path.clone(),
        })
        .collect()
}

/// Largest offset that still fills the visible window. Recomputed on
/// every interaction because the popup can grow or shrink between
/// frames (terminal resize, bottom-panel toggle); a stale offset would
/// make the renderer and the click hit-test disagree about which row
/// sits on which screen line.
pub(crate) fn clamp_scroll(scroll: usize, total: usize, visible: usize) -> usize {
    if visible == 0 {
        return 0;
    }
    scroll.min(total.saturating_sub(visible))
}

/// Keep `selected` inside the visible window, returning the new scroll
/// offset. Mirrors the agents-list auto-scroll rule: scroll up when the
/// selection moves above the window, down when it moves below.
fn follow_selection(selected: usize, scroll: usize, visible: usize) -> usize {
    if visible == 0 {
        return 0;
    }
    if selected < scroll {
        selected
    } else if selected >= scroll + visible {
        selected + 1 - visible
    } else {
        scroll
    }
}

impl AppState {
    // ─── Accessors ───────────────────────────────────────────────────

    pub fn is_open_worktree_open(&self) -> bool {
        matches!(self.popup, PopupState::OpenWorktree { .. })
    }

    pub fn open_worktree_popup_area(&self) -> Option<Rect> {
        match &self.popup {
            PopupState::OpenWorktree { area, .. } => *area,
            _ => None,
        }
    }

    pub fn open_worktree_step(&self) -> Option<OpenStep> {
        match &self.popup {
            PopupState::OpenWorktree { step, .. } => Some(*step),
            _ => None,
        }
    }

    pub fn open_worktree_selected(&self) -> usize {
        match &self.popup {
            PopupState::OpenWorktree { selected, .. } => *selected,
            _ => 0,
        }
    }

    pub fn open_worktree_scroll(&self) -> usize {
        match &self.popup {
            PopupState::OpenWorktree { scroll, .. } => *scroll,
            _ => 0,
        }
    }

    // ─── Entry point (`o` key) ───────────────────────────────────────

    /// Resolve the repo from the current selection, list its worktrees,
    /// and open the picker. Every failure path flashes and leaves the
    /// popup closed — an empty modal would be worse than a message.
    pub fn open_worktree_from_selection(&mut self) {
        let Some(pane) = self.selected_pane() else {
            self.set_flash("open: no pane selected");
            return;
        };
        let pane_id = pane.pane_id.clone();
        let Some(group) = self
            .repo_groups
            .iter()
            .find(|g| g.panes.iter().any(|(p, _)| p.pane_id == pane_id))
        else {
            self.set_flash("open: could not find repo group for selection");
            return;
        };
        let Some(root) = group
            .panes
            .iter()
            .find_map(|(_, git)| git.repo_root.clone())
        else {
            self.set_flash("open: selected pane is not in a git repo");
            return;
        };
        let name = group.name.clone();

        let entries = match crate::git::worktree_list(&root) {
            Ok(entries) => entries,
            Err(e) => {
                self.set_flash(format!("open: git: {e}"));
                return;
            }
        };
        // Every pane path the sidebar knows about, so the picker can mark
        // worktrees that already have an agent in them. Costs no extra
        // tmux or git calls.
        let pane_paths: Vec<String> = self
            .repo_groups
            .iter()
            .flat_map(|g| g.panes.iter().map(|(p, _)| p.path.clone()))
            .collect();
        let rows = build_rows(entries, &pane_paths, &|p| Path::new(p).exists());
        if rows.is_empty() {
            self.set_flash("open: no worktrees found");
            return;
        }

        // Anchor below the repo header row, same as the spawn modal, so
        // `n` and `o` popups appear in the same place.
        let anchor_y = self
            .layout
            .repo_spawn_targets
            .iter()
            .find(|t| t.repo_name == name)
            .map(|t| t.rect.y);
        self.popup = PopupState::OpenWorktree {
            target_repo_root: root,
            rows,
            selected: 0,
            scroll: 0,
            step: OpenStep::Pick,
            pick: super::AgentPick::default(),
            field: OpenField::default(),
            anchor_y,
            error: None,
            area: None,
        };
    }

    // ─── Navigation ──────────────────────────────────────────────────

    pub fn close_open_worktree(&mut self) {
        if self.is_open_worktree_open() {
            self.popup = PopupState::None;
        }
    }

    /// Single navigation entry point for `j`/`k`/`Tab`/arrows: moves the
    /// list selection in `Pick`, the focused field in `Configure`. Keeps
    /// the key table flat and lets `j`/`k` work in both steps (there is
    /// no text field to shadow them).
    pub fn open_worktree_nav(&mut self, delta: isize) {
        match self.open_worktree_step() {
            Some(OpenStep::Pick) => self.open_worktree_move(delta),
            Some(OpenStep::Configure) => {
                if delta >= 0 {
                    self.open_worktree_next_field();
                } else {
                    self.open_worktree_prev_field();
                }
            }
            None => {}
        }
    }

    /// Move the picker selection, clamped at both ends, and follow it
    /// with the scroll offset. No-op outside the `Pick` step.
    pub fn open_worktree_move(&mut self, delta: isize) {
        let visible = self.open_worktree_visible_rows();
        let PopupState::OpenWorktree {
            rows,
            selected,
            scroll,
            step,
            error,
            ..
        } = &mut self.popup
        else {
            return;
        };
        if *step != OpenStep::Pick || rows.is_empty() {
            return;
        }
        let last = rows.len() - 1;
        let next = (*selected as isize + delta).clamp(0, last as isize) as usize;
        if next != *selected {
            *selected = next;
            *error = None;
        }
        let start = clamp_scroll(*scroll, rows.len(), visible);
        *scroll = follow_selection(*selected, start, visible);
    }

    pub fn open_worktree_next_field(&mut self) {
        if let PopupState::OpenWorktree {
            field, step, error, ..
        } = &mut self.popup
            && *step == OpenStep::Configure
        {
            *field = field.next();
            *error = None;
        }
    }

    pub fn open_worktree_prev_field(&mut self) {
        if let PopupState::OpenWorktree {
            field, step, error, ..
        } = &mut self.popup
            && *step == OpenStep::Configure
        {
            *field = field.prev();
            *error = None;
        }
    }

    /// Cycle the value under the focused field. No-op in the `Pick`
    /// step, where left/right have nothing to act on.
    pub fn open_worktree_cycle(&mut self, delta: isize) {
        let PopupState::OpenWorktree {
            step,
            field,
            pick,
            error,
            ..
        } = &mut self.popup
        else {
            return;
        };
        if *step != OpenStep::Configure {
            return;
        }
        match *field {
            OpenField::Agent => pick.cycle_agent(delta),
            OpenField::Mode => pick.cycle_mode(delta),
        }
        *error = None;
    }

    /// Mouse: select the row at `idx` in the picker. Out-of-range
    /// indices (clicks on padding below the last row) are ignored.
    pub fn open_worktree_select_row(&mut self, idx: usize) {
        let visible = self.open_worktree_visible_rows();
        let PopupState::OpenWorktree {
            rows,
            selected,
            scroll,
            step,
            error,
            ..
        } = &mut self.popup
        else {
            return;
        };
        if *step != OpenStep::Pick {
            return;
        }
        // `idx` is relative to the first visible row, not the first row,
        // and the renderer derives that row from the *clamped* offset —
        // so the same clamp has to happen here or a click after a resize
        // maps to the wrong worktree.
        let start = clamp_scroll(*scroll, rows.len(), visible);
        let Some(target) = idx.checked_add(start).filter(|i| *i < rows.len()) else {
            return;
        };
        *selected = target;
        *error = None;
        *scroll = follow_selection(*selected, start, visible);
    }

    // ─── Confirm / back ──────────────────────────────────────────────

    /// `Enter`: advance `Pick` → `Configure`, or run the open flow from
    /// `Configure`. Errors stay inline so the modal survives for a retry.
    pub fn confirm_open_worktree(&mut self) {
        let PopupState::OpenWorktree {
            target_repo_root,
            rows,
            selected,
            step,
            pick,
            ..
        } = &self.popup
        else {
            return;
        };
        let Some(row) = rows.get(*selected).cloned() else {
            self.set_open_worktree_error("no worktree selected");
            return;
        };
        let step = *step;
        let pick = *pick;
        let repo_root = target_repo_root.clone();

        if step == OpenStep::Pick {
            if let PopupState::OpenWorktree {
                step, pick, field, ..
            } = &mut self.popup
            {
                *step = OpenStep::Configure;
                *pick = super::AgentPick::default();
                *field = OpenField::default();
            }
            return;
        }

        let Some(session) = crate::tmux::pane_session_name(&self.tmux_pane) else {
            self.set_open_worktree_error("could not resolve tmux session");
            return;
        };
        let agent = pick.agent_or_default();
        let mode = pick.mode_or_default(&agent);
        let req = crate::worktree::OpenRequest {
            repo_root: std::path::PathBuf::from(repo_root),
            worktree_path: std::path::PathBuf::from(row.path),
            branch: row.branch,
            session,
            agent,
            mode,
        };

        match crate::worktree::open(&req) {
            Ok(()) => self.popup = PopupState::None,
            Err(e) => self.set_open_worktree_error(e),
        }
    }

    /// `Esc`: step back from `Configure` to `Pick` (keeping the picked
    /// row), or close the modal from `Pick`.
    pub fn open_worktree_back(&mut self) {
        match &mut self.popup {
            PopupState::OpenWorktree { step, error, .. } if *step == OpenStep::Configure => {
                *step = OpenStep::Pick;
                *error = None;
            }
            PopupState::OpenWorktree { .. } => self.popup = PopupState::None,
            _ => {}
        }
    }

    fn set_open_worktree_error(&mut self, msg: impl Into<String>) {
        if let PopupState::OpenWorktree { error, .. } = &mut self.popup {
            *error = Some(msg.into());
        }
    }

    /// How many picker rows fit inside the popup as last rendered. The
    /// renderer sizes the popup to the row count, so before the first
    /// frame (or when the area is unset) every row is treated as
    /// visible and no scrolling happens.
    fn open_worktree_visible_rows(&self) -> usize {
        match &self.popup {
            PopupState::OpenWorktree { rows, area, .. } => match area {
                Some(rect) => rect.height.saturating_sub(PICKER_BORDER_ROWS) as usize,
                None => rows.len(),
            },
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::AgentPick;
    use super::*;
    use crate::git::WorktreeEntry;
    use crate::group::{PaneGitInfo, RepoGroup};
    use crate::tmux::{AgentType, PaneInfo, PaneStatus, PermissionMode, WorktreeMetadata};

    fn test_pane(id: &str, path: &str) -> PaneInfo {
        PaneInfo {
            pane_id: id.into(),
            pane_active: false,
            status: PaneStatus::Running,
            attention: false,
            agent: AgentType::Claude,
            path: path.into(),
            current_command: String::new(),
            prompt: String::new(),
            prompt_is_response: false,
            started_at: None,
            wait_reason: String::new(),
            permission_mode: PermissionMode::Default,
            subagents: vec![],
            pane_pid: None,
            worktree: WorktreeMetadata::default(),
            session_id: None,
            session_name: String::new(),
            sidebar_spawned: false,
            bg_shell_cmd: None,
        }
    }

    fn entry(path: &str, branch: &str) -> WorktreeEntry {
        WorktreeEntry {
            path: path.into(),
            branch: branch.into(),
            head: "abc1234def".into(),
            ..WorktreeEntry::default()
        }
    }

    fn row(path: &str, branch: &str) -> OpenWorktreeRow {
        OpenWorktreeRow {
            path: path.into(),
            branch: branch.into(),
            label: branch.into(),
            in_use: false,
        }
    }

    /// Open the picker directly, bypassing the git call.
    fn state_with_picker(rows: Vec<OpenWorktreeRow>) -> AppState {
        let mut state = AppState::new("%99".into());
        state.popup = PopupState::OpenWorktree {
            target_repo_root: "/repo".into(),
            rows,
            selected: 0,
            scroll: 0,
            step: OpenStep::Pick,
            pick: AgentPick::default(),
            field: OpenField::default(),
            anchor_y: None,
            error: None,
            area: None,
        };
        state
    }

    fn popup_error(state: &AppState) -> Option<String> {
        match &state.popup {
            PopupState::OpenWorktree { error, .. } => error.clone(),
            _ => None,
        }
    }

    // ─── build_rows ──────────────────────────────────────────────────

    #[test]
    fn build_rows_keeps_main_and_linked_worktrees() {
        let rows = build_rows(
            vec![
                entry("/repo", "main"),
                entry("/repo/.wt/login", "agent/login"),
            ],
            &[],
            &|_| true,
        );
        assert_eq!(
            rows,
            vec![row("/repo", "main"), row("/repo/.wt/login", "agent/login")]
        );
    }

    #[test]
    fn build_rows_drops_bare_prunable_and_missing() {
        let entries = vec![
            WorktreeEntry {
                path: "/repo.git".into(),
                bare: true,
                ..WorktreeEntry::default()
            },
            WorktreeEntry {
                path: "/repo/.wt/stale".into(),
                branch: "agent/stale".into(),
                prunable: true,
                ..WorktreeEntry::default()
            },
            entry("/repo/.wt/gone", "agent/gone"),
            entry("/repo", "main"),
        ];
        let rows = build_rows(entries, &[], &|p| p != "/repo/.wt/gone");
        assert_eq!(
            rows,
            vec![row("/repo", "main")],
            "bare, prunable and vanished worktrees must not be offered — \
             `tmux new-window -c` on them fails opaquely"
        );
    }

    #[test]
    fn build_rows_labels_detached_head_with_short_sha() {
        let rows = build_rows(
            vec![WorktreeEntry {
                path: "/repo/.wt/spike".into(),
                head: "deadbeefcafe1234".into(),
                detached: true,
                ..WorktreeEntry::default()
            }],
            &[],
            &|_| true,
        );
        assert_eq!(rows[0].label, "(detached deadbee)");
        assert!(
            rows[0].branch.is_empty(),
            "the display label must not leak into the branch marker"
        );
    }

    #[test]
    fn build_rows_short_head_does_not_panic() {
        let rows = build_rows(
            vec![WorktreeEntry {
                path: "/repo/.wt/spike".into(),
                head: "abc".into(),
                detached: true,
                ..WorktreeEntry::default()
            }],
            &[],
            &|_| true,
        );
        assert_eq!(rows[0].label, "(detached abc)");
    }

    #[test]
    fn build_rows_marks_in_use_for_exact_and_nested_pane_paths() {
        let paths = vec![
            "/repo/.wt/login".to_string(),
            "/repo/src/deep".to_string(),
            "/other/repo".to_string(),
        ];
        let rows = build_rows(
            vec![
                entry("/repo", "main"),
                entry("/repo/.wt/login", "agent/login"),
                entry("/repo/.wt/db", "agent/db"),
            ],
            &paths,
            &|_| true,
        );
        assert!(rows[0].in_use, "a pane in a subdirectory counts");
        assert!(rows[1].in_use, "an exact pane path counts");
        assert!(!rows[2].in_use);
    }

    #[test]
    fn build_rows_prefix_match_respects_path_boundary() {
        // `/repo/.wt/login-2` must not mark `/repo/.wt/login` in use.
        let rows = build_rows(
            vec![entry("/repo/.wt/login", "agent/login")],
            &["/repo/.wt/login-2".to_string()],
            &|_| true,
        );
        assert!(!rows[0].in_use);
    }

    #[test]
    fn build_rows_nested_worktree_pane_does_not_mark_the_main_checkout() {
        // Regression: linked worktrees live under the main checkout by
        // default (`<repo>/.worktrees/<slug>`), so plain containment lit
        // up the main row for every pane in every worktree — the `●`
        // would have been permanently on for any repo using the default
        // worktree directory.
        let rows = build_rows(
            vec![
                entry("/repo", "main"),
                entry("/repo/.worktrees/login", "agent/login"),
            ],
            &["/repo/.worktrees/login".to_string()],
            &|_| true,
        );
        assert!(
            !rows[0].in_use,
            "nothing runs in the main checkout — only the linked worktree has a pane"
        );
        assert!(rows[1].in_use);
    }

    #[test]
    fn build_rows_assigns_each_pane_to_its_longest_matching_worktree() {
        let rows = build_rows(
            vec![
                entry("/repo", "main"),
                entry("/repo/.worktrees/login", "agent/login"),
                entry("/repo/.worktrees/db", "agent/db"),
            ],
            &[
                // Deep inside the login worktree → login only.
                "/repo/.worktrees/login/src/api".to_string(),
                // Inside the main checkout but not any worktree → main.
                "/repo/src".to_string(),
            ],
            &|_| true,
        );
        assert!(rows[0].in_use, "the /repo/src pane belongs to main");
        assert!(rows[1].in_use, "the nested pane belongs to login");
        assert!(!rows[2].in_use, "nothing runs in the db worktree");
    }

    // ─── follow_selection ────────────────────────────────────────────

    #[test]
    fn follow_selection_scrolls_only_when_out_of_window() {
        assert_eq!(follow_selection(1, 0, 3), 0, "inside the window: no move");
        assert_eq!(follow_selection(3, 0, 3), 1, "past the bottom: scroll down");
        assert_eq!(follow_selection(0, 2, 3), 0, "above the top: scroll up");
        assert_eq!(follow_selection(5, 0, 0), 0, "zero-height window is inert");
    }

    #[test]
    fn clamp_scroll_pins_offset_to_the_last_full_window() {
        assert_eq!(clamp_scroll(4, 8, 4), 4, "already valid");
        assert_eq!(clamp_scroll(4, 8, 8), 0, "popup grew to fit everything");
        assert_eq!(clamp_scroll(7, 8, 3), 5, "offset past the last full window");
        assert_eq!(clamp_scroll(3, 0, 4), 0, "no rows");
        assert_eq!(clamp_scroll(3, 8, 0), 0, "zero-height window");
    }

    // ─── open_worktree_from_selection failure paths ──────────────────

    #[test]
    fn open_worktree_from_selection_without_selection_flashes() {
        let mut state = AppState::new("%99".into());
        state.open_worktree_from_selection();
        assert_eq!(
            state.take_flash().as_deref(),
            Some("open: no pane selected")
        );
        assert!(!state.is_open_worktree_open());
    }

    #[test]
    fn open_worktree_from_selection_without_repo_root_flashes() {
        let mut state = AppState::new("%99".into());
        state.repo_groups = vec![RepoGroup {
            name: "proj".into(),
            has_focus: true,
            panes: vec![(test_pane("%1", "/tmp"), PaneGitInfo::default())],
        }];
        state.rebuild_row_targets();
        state.open_worktree_from_selection();
        assert_eq!(
            state.take_flash().as_deref(),
            Some("open: selected pane is not in a git repo")
        );
        assert!(!state.is_open_worktree_open());
    }

    #[test]
    fn open_worktree_from_selection_with_stale_row_target_flashes() {
        // A row target pointing at a pane no longer in any repo group.
        // `selected_pane` resolves through `repo_groups`, so this lands
        // on the "no pane selected" guard rather than the group lookup —
        // either way the popup must stay closed.
        let mut state = AppState::new("%99".into());
        state.layout.pane_row_targets = vec![crate::state::RowTarget {
            pane_id: "%1".into(),
        }];
        state.global.selected_pane_row = 0;
        state.open_worktree_from_selection();
        assert!(state.take_flash().is_some());
        assert!(!state.is_open_worktree_open());
    }

    // ─── Navigation ──────────────────────────────────────────────────

    #[test]
    fn open_worktree_move_clamps_at_both_ends() {
        let mut state = state_with_picker(vec![
            row("/a", "main"),
            row("/b", "agent/x"),
            row("/c", "agent/y"),
        ]);
        state.open_worktree_move(-1);
        assert_eq!(state.open_worktree_selected(), 0, "clamped at the top");
        state.open_worktree_move(1);
        state.open_worktree_move(1);
        assert_eq!(state.open_worktree_selected(), 2);
        state.open_worktree_move(1);
        assert_eq!(state.open_worktree_selected(), 2, "clamped at the bottom");
    }

    #[test]
    fn open_worktree_move_scrolls_to_follow_selection() {
        let rows: Vec<_> = (0..6)
            .map(|i| row(&format!("/w{i}"), &format!("agent/{i}")))
            .collect();
        let mut state = state_with_picker(rows);
        // Simulate a rendered popup showing 3 rows (3 + 2 borders).
        state
            .popup
            .set_open_worktree_area(Some(Rect::new(0, 0, 20, 5)));

        for _ in 0..3 {
            state.open_worktree_move(1);
        }
        assert_eq!(state.open_worktree_selected(), 3);
        assert_eq!(state.open_worktree_scroll(), 1, "window follows downward");

        for _ in 0..3 {
            state.open_worktree_move(-1);
        }
        assert_eq!(state.open_worktree_selected(), 0);
        assert_eq!(state.open_worktree_scroll(), 0, "window follows upward");
    }

    #[test]
    fn open_worktree_move_is_noop_in_configure_step() {
        let mut state = state_with_picker(vec![row("/a", "main"), row("/b", "agent/x")]);
        state.confirm_open_worktree(); // Pick → Configure
        state.open_worktree_move(1);
        assert_eq!(state.open_worktree_selected(), 0);
    }

    #[test]
    fn open_worktree_nav_moves_list_in_pick_and_field_in_configure() {
        let mut state = state_with_picker(vec![row("/a", "main"), row("/b", "agent/x")]);
        state.open_worktree_nav(1);
        assert_eq!(state.open_worktree_selected(), 1);

        state.confirm_open_worktree();
        state.open_worktree_nav(1);
        assert!(matches!(
            state.popup,
            PopupState::OpenWorktree {
                field: OpenField::Mode,
                ..
            }
        ));
        state.open_worktree_nav(-1);
        assert!(matches!(
            state.popup,
            PopupState::OpenWorktree {
                field: OpenField::Agent,
                ..
            }
        ));
    }

    #[test]
    fn open_worktree_cycle_is_noop_in_pick_step() {
        let mut state = state_with_picker(vec![row("/a", "main")]);
        state.open_worktree_cycle(1);
        assert!(matches!(
            state.popup,
            PopupState::OpenWorktree {
                pick: AgentPick {
                    agent_idx: 0,
                    mode_idx: 0
                },
                ..
            }
        ));
    }

    #[test]
    fn open_worktree_cycle_moves_agent_then_mode_in_configure() {
        let mut state = state_with_picker(vec![row("/a", "main")]);
        state.confirm_open_worktree();
        state.open_worktree_cycle(1); // agent: claude → codex
        state.open_worktree_next_field();
        state.open_worktree_cycle(1); // mode: default → auto
        match &state.popup {
            PopupState::OpenWorktree { pick, .. } => {
                assert_eq!(pick.agent(), "codex");
                assert_eq!(pick.mode(), "auto");
            }
            _ => panic!("popup must stay open"),
        }
    }

    // ─── Step transitions ────────────────────────────────────────────

    #[test]
    fn confirm_in_pick_advances_to_configure_with_defaults() {
        let mut state = state_with_picker(vec![row("/a", "main"), row("/b", "agent/x")]);
        state.open_worktree_move(1);
        state.confirm_open_worktree();
        match &state.popup {
            PopupState::OpenWorktree {
                step,
                pick,
                field,
                selected,
                ..
            } => {
                assert_eq!(*step, OpenStep::Configure);
                assert_eq!(*pick, AgentPick::default());
                assert_eq!(*field, OpenField::Agent);
                assert_eq!(*selected, 1, "the picked row is preserved");
            }
            _ => panic!("popup must stay open"),
        }
    }

    #[test]
    fn back_from_configure_returns_to_pick_preserving_selection() {
        let mut state = state_with_picker(vec![row("/a", "main"), row("/b", "agent/x")]);
        state.open_worktree_move(1);
        state.confirm_open_worktree();
        state.open_worktree_back();
        assert_eq!(state.open_worktree_step(), Some(OpenStep::Pick));
        assert_eq!(state.open_worktree_selected(), 1);
    }

    #[test]
    fn back_from_pick_closes_the_popup() {
        let mut state = state_with_picker(vec![row("/a", "main")]);
        state.open_worktree_back();
        assert!(matches!(state.popup, PopupState::None));
    }

    #[test]
    fn close_open_worktree_leaves_other_popups_alone() {
        let mut state = AppState::new("%99".into());
        state.popup = PopupState::Notices { area: None };
        state.close_open_worktree();
        assert!(matches!(state.popup, PopupState::Notices { .. }));
    }

    #[test]
    fn confirm_with_empty_rows_sets_error_and_keeps_popup_open() {
        let mut state = state_with_picker(vec![]);
        state.confirm_open_worktree();
        assert!(state.is_open_worktree_open(), "popup must stay open");
        assert_eq!(popup_error(&state).as_deref(), Some("no worktree selected"));
    }

    // ─── Mouse ───────────────────────────────────────────────────────

    #[test]
    fn select_row_sets_selection_and_ignores_out_of_range() {
        let mut state = state_with_picker(vec![
            row("/a", "main"),
            row("/b", "agent/x"),
            row("/c", "agent/y"),
        ]);
        state.open_worktree_select_row(2);
        assert_eq!(state.open_worktree_selected(), 2);
        state.open_worktree_select_row(9);
        assert_eq!(
            state.open_worktree_selected(),
            2,
            "a click below the last row must not move the selection"
        );
    }

    #[test]
    fn select_row_is_relative_to_the_scroll_offset() {
        let rows: Vec<_> = (0..6)
            .map(|i| row(&format!("/w{i}"), &format!("agent/{i}")))
            .collect();
        let mut state = state_with_picker(rows);
        state
            .popup
            .set_open_worktree_area(Some(Rect::new(0, 0, 20, 5)));
        for _ in 0..3 {
            state.open_worktree_move(1);
        }
        assert_eq!(state.open_worktree_scroll(), 1);

        // Clicking the first visible row picks the row at `scroll`, not 0.
        state.open_worktree_select_row(0);
        assert_eq!(state.open_worktree_selected(), 1);
    }

    #[test]
    fn select_row_uses_the_clamped_offset_after_the_popup_grows() {
        // Regression: the renderer clamps the offset so a grown popup
        // draws from row 0, but the hit-test used to add the raw stored
        // offset — clicking the top visible row then selected a row
        // several entries down, and Enter would open an agent in the
        // wrong worktree.
        let rows: Vec<_> = (0..8)
            .map(|i| row(&format!("/w{i}"), &format!("agent/{i}")))
            .collect();
        let mut state = state_with_picker(rows);

        // Small popup: 4 visible rows. Scroll to the bottom.
        state
            .popup
            .set_open_worktree_area(Some(Rect::new(0, 0, 20, 6)));
        for _ in 0..7 {
            state.open_worktree_move(1);
        }
        assert_eq!(state.open_worktree_scroll(), 4);

        // Terminal grows — all 8 rows now fit, so the first visible row
        // is row 0 again.
        state
            .popup
            .set_open_worktree_area(Some(Rect::new(0, 0, 20, 10)));
        state.open_worktree_select_row(0);
        assert_eq!(
            state.open_worktree_selected(),
            0,
            "the first visible row is row 0 once the popup fits everything"
        );
        assert_eq!(state.open_worktree_scroll(), 0);
    }

    #[test]
    fn move_reclamps_a_stale_offset_after_the_popup_grows() {
        let rows: Vec<_> = (0..8)
            .map(|i| row(&format!("/w{i}"), &format!("agent/{i}")))
            .collect();
        let mut state = state_with_picker(rows);
        state
            .popup
            .set_open_worktree_area(Some(Rect::new(0, 0, 20, 6)));
        for _ in 0..7 {
            state.open_worktree_move(1);
        }
        assert_eq!(state.open_worktree_scroll(), 4);

        state
            .popup
            .set_open_worktree_area(Some(Rect::new(0, 0, 20, 10)));
        state.open_worktree_move(-1);
        assert_eq!(state.open_worktree_selected(), 6);
        assert_eq!(
            state.open_worktree_scroll(),
            0,
            "a window that fits every row must sit at offset 0"
        );
    }

    #[test]
    fn select_row_is_noop_in_configure_step() {
        let mut state = state_with_picker(vec![row("/a", "main"), row("/b", "agent/x")]);
        state.confirm_open_worktree();
        state.open_worktree_select_row(1);
        assert_eq!(state.open_worktree_selected(), 0);
    }
}
