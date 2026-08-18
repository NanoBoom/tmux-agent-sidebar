use super::AppState;

pub(crate) mod open_worktree;

/// Agent + permission-mode selection shared by the spawn and open
/// worktree modals. `agent_idx` indexes [`crate::worktree::AGENTS`];
/// `mode_idx` indexes [`crate::worktree::modes_for`] for that agent.
/// Extracted so the non-trivial cycle rules (agent wrap resets the mode,
/// mode wraps against the agent-specific list) have one source of truth.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AgentPick {
    pub agent_idx: usize,
    pub mode_idx: usize,
}

impl AgentPick {
    /// Selected agent, or `""` when `agent_idx` is out of range.
    pub fn agent(&self) -> &'static str {
        crate::worktree::AGENTS
            .get(self.agent_idx)
            .copied()
            .unwrap_or("")
    }

    /// Selected permission mode, or `""` when `mode_idx` is out of range.
    pub fn mode(&self) -> &'static str {
        crate::worktree::modes_for(self.agent())
            .get(self.mode_idx)
            .copied()
            .unwrap_or("")
    }

    /// Move `agent_idx` by `delta`, wrapping. The mode list is
    /// agent-specific, so the mode selection resets to the first entry.
    pub fn cycle_agent(&mut self, delta: isize) {
        let len = crate::worktree::AGENTS.len() as isize;
        if len == 0 {
            return;
        }
        self.agent_idx = ((self.agent_idx as isize + delta).rem_euclid(len)) as usize;
        self.mode_idx = 0;
    }

    /// Move `mode_idx` by `delta`, wrapping within the current agent's
    /// mode list. No-op when that list is empty.
    pub fn cycle_mode(&mut self, delta: isize) {
        let len = crate::worktree::modes_for(self.agent()).len() as isize;
        if len == 0 {
            return;
        }
        self.mode_idx = ((self.mode_idx as isize + delta).rem_euclid(len)) as usize;
    }

    /// Agent name for launching, falling back to the configured default
    /// rather than `""` so a corrupt index still produces a runnable
    /// command.
    pub fn agent_or_default(&self) -> String {
        crate::worktree::AGENTS
            .get(self.agent_idx)
            .copied()
            .unwrap_or(crate::worktree::DEFAULT_AGENT)
            .to_string()
    }

    /// Mode name for launching, falling back to the configured default.
    pub fn mode_or_default(&self, agent: &str) -> String {
        crate::worktree::modes_for(agent)
            .get(self.mode_idx)
            .copied()
            .unwrap_or(crate::worktree::DEFAULT_MODE)
            .to_string()
    }
}

/// Focus target inside the spawn input popup. Tab / Shift+Tab / arrow
/// keys cycle through these in order; only `Task` accepts text input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpawnField {
    #[default]
    Task,
    Agent,
    Mode,
}

/// Focus target inside the open-worktree modal's second step. There is
/// no text input there, so this is deliberately not [`SpawnField`] —
/// reusing that enum would make the unreachable `Task` variant
/// representable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OpenField {
    #[default]
    Agent,
    Mode,
}

impl OpenField {
    pub fn next(self) -> Self {
        match self {
            Self::Agent => Self::Mode,
            Self::Mode => Self::Agent,
        }
    }

    pub fn prev(self) -> Self {
        // Two variants, so prev and next coincide; spelled out
        // separately to keep the call sites symmetric with SpawnField.
        self.next()
    }
}

/// Which half of the open-worktree modal is showing. One popup variant
/// holds both so the worktree list survives the step transition —
/// `Esc` from `Configure` returns to `Pick` without re-running
/// `git worktree list`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenStep {
    Pick,
    Configure,
}

/// Row shown in the open-worktree picker: an existing worktree plus the
/// sidebar-local "is a pane already running here" flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenWorktreeRow {
    pub path: String,
    /// Short branch name. Empty for a detached checkout — kept separate
    /// from `label` so the branch marker written on the new window is
    /// never a display string.
    pub branch: String,
    /// What the picker renders: the branch, or `(detached <sha7>)`.
    pub label: String,
    pub in_use: bool,
}

impl SpawnField {
    pub fn next(self) -> Self {
        match self {
            Self::Task => Self::Agent,
            Self::Agent => Self::Mode,
            Self::Mode => Self::Task,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Task => Self::Mode,
            Self::Agent => Self::Task,
            Self::Mode => Self::Agent,
        }
    }
}

/// What the close-pane modal puts in its title. This is the string the
/// user reads before pressing `y`, and `y` runs `git branch -D` on
/// [`SpawnMarkers::branch`] — so it has to be that branch, not the
/// worktree directory name. For a spawned worktree the two only differ
/// by the `agent/` prefix, but an opened one can sit in a directory
/// whose name has nothing to do with its branch.
///
/// Falls back to the directory basename for a detached checkout, which
/// has no branch to name.
fn remove_confirm_label(markers: &crate::worktree::SpawnMarkers) -> String {
    if !markers.branch.is_empty() {
        return markers.branch.clone();
    }
    std::path::Path::new(&markers.worktree_path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// At-most-one popup state for the sidebar. The enum variant encodes
/// both which popup is open and its per-popup data, so the "only one
/// popup open at a time" invariant is checked by the type system.
#[derive(Debug, Clone, Default)]
pub enum PopupState {
    #[default]
    None,
    Repo {
        selected: usize,
        area: Option<ratatui::layout::Rect>,
    },
    Notices {
        area: Option<ratatui::layout::Rect>,
    },
    /// Modal text input shown when the user presses `n` (or clicks `+`)
    /// to spawn a new worktree. `target_repo` / `target_repo_root` pin
    /// the spawn target; `pick` holds the user's agent and
    /// permission-mode selection.
    SpawnInput {
        input: String,
        target_repo: String,
        target_repo_root: String,
        pick: AgentPick,
        field: SpawnField,
        /// Screen Y of the repo header row that owns the `+` button
        /// this modal was opened from. Renderer anchors the popup just
        /// below it; `None` falls back to a centered layout.
        anchor_y: Option<u16>,
        /// Inline error message rendered at the bottom of the popup
        /// so spawn failures stay visually attached to the input the
        /// user was editing. Cleared on the next edit / field change.
        error: Option<String>,
        area: Option<ratatui::layout::Rect>,
    },
    /// Confirmation prompt shown when the user presses `x` on a
    /// sidebar-created pane (`n` or `o`). `pane_id` feeds
    /// `worktree::remove`; `branch` is the modal title and names exactly
    /// what `[y]` will `git branch -D` — see `remove_confirm_label`.
    RemoveConfirm {
        pane_id: String,
        branch: String,
        /// The worktree has staged, unstaged or untracked changes, as
        /// of the moment the modal was opened. `[y]` runs
        /// `git worktree remove --force`, which would discard them, so
        /// when this is set the renderer greys the option out and
        /// `confirm_remove` refuses it. `[c]` is unaffected.
        dirty: bool,
        error: Option<String>,
        area: Option<ratatui::layout::Rect>,
    },
    /// Two-step modal shown when the user presses `o`: pick one of the
    /// repository's existing worktrees, then pick the agent and
    /// permission mode to launch in it. Both steps live in one variant
    /// so `rows` survives the transition (see [`OpenStep`]).
    OpenWorktree {
        target_repo_root: String,
        rows: Vec<OpenWorktreeRow>,
        selected: usize,
        /// First visible row — the list scrolls when it exceeds the
        /// popup's inner height.
        scroll: usize,
        step: OpenStep,
        pick: AgentPick,
        field: OpenField,
        /// Screen Y of the repo header row this modal was opened from,
        /// so `o` and `n` modals appear in the same place.
        anchor_y: Option<u16>,
        error: Option<String>,
        area: Option<ratatui::layout::Rect>,
    },
}

impl PopupState {
    pub fn set_repo_area(&mut self, rect: Option<ratatui::layout::Rect>) {
        if let Self::Repo { area, .. } = self {
            *area = rect;
        }
    }

    pub fn set_notices_area(&mut self, rect: Option<ratatui::layout::Rect>) {
        if let Self::Notices { area } = self {
            *area = rect;
        }
    }

    pub fn set_spawn_input_area(&mut self, rect: Option<ratatui::layout::Rect>) {
        if let Self::SpawnInput { area, .. } = self {
            *area = rect;
        }
    }

    pub fn set_remove_confirm_area(&mut self, rect: Option<ratatui::layout::Rect>) {
        if let Self::RemoveConfirm { area, .. } = self {
            *area = rect;
        }
    }

    pub fn set_open_worktree_area(&mut self, rect: Option<ratatui::layout::Rect>) {
        if let Self::OpenWorktree { area, .. } = self {
            *area = rect;
        }
    }
}

impl AppState {
    // ─── Repo popup ──────────────────────────────────────────────────────

    pub fn is_repo_popup_open(&self) -> bool {
        matches!(self.popup, PopupState::Repo { .. })
    }

    pub fn repo_popup_selected(&self) -> usize {
        match &self.popup {
            PopupState::Repo { selected, .. } => *selected,
            _ => 0,
        }
    }

    pub fn set_repo_popup_selected(&mut self, n: usize) {
        if let PopupState::Repo { selected, .. } = &mut self.popup {
            *selected = n;
        }
    }

    pub fn repo_popup_area(&self) -> Option<ratatui::layout::Rect> {
        match &self.popup {
            PopupState::Repo { area, .. } => *area,
            _ => None,
        }
    }

    pub fn toggle_repo_popup(&mut self) {
        if self.is_repo_popup_open() {
            self.close_repo_popup();
            return;
        }
        // Set selected to current filter position
        let names = self.repo_names();
        let selected = match &self.global.repo_filter {
            super::RepoFilter::All => 0,
            super::RepoFilter::Repo(name) => names.iter().position(|n| n == name).unwrap_or(0),
        };
        self.popup = PopupState::Repo {
            selected,
            area: None,
        };
    }

    pub fn confirm_repo_popup(&mut self) {
        let selected = self.repo_popup_selected();
        let names = self.repo_names();
        if let Some(name) = names.get(selected) {
            self.global.repo_filter = if selected == 0 {
                super::RepoFilter::All
            } else {
                super::RepoFilter::Repo(name.clone())
            };
        }
        self.popup = PopupState::None;
        self.global.save_repo_filter();
        self.rebuild_row_targets();
    }

    pub fn close_repo_popup(&mut self) {
        self.popup = PopupState::None;
    }

    // ─── Notices popup ───────────────────────────────────────────────────

    pub fn is_notices_popup_open(&self) -> bool {
        matches!(self.popup, PopupState::Notices { .. })
    }

    pub fn notices_popup_area(&self) -> Option<ratatui::layout::Rect> {
        match &self.popup {
            PopupState::Notices { area } => *area,
            _ => None,
        }
    }

    pub fn toggle_notices_popup(&mut self) {
        if self.is_notices_popup_open() {
            self.close_notices_popup();
        } else {
            self.popup = PopupState::Notices { area: None };
        }
    }

    pub fn close_notices_popup(&mut self) {
        self.popup = PopupState::None;
        self.notices.copy_targets.clear();
        self.notices.copied_at = None;
    }

    // ─── Spawn input popup (n key / + click) ─────────────────────────────

    pub fn is_spawn_input_open(&self) -> bool {
        matches!(self.popup, PopupState::SpawnInput { .. })
    }

    pub fn spawn_input_popup_area(&self) -> Option<ratatui::layout::Rect> {
        match &self.popup {
            PopupState::SpawnInput { area, .. } => *area,
            _ => None,
        }
    }

    pub fn open_spawn_input_for_repo(
        &mut self,
        repo_name: String,
        repo_root: String,
        anchor_y: Option<u16>,
    ) {
        self.popup = PopupState::SpawnInput {
            input: String::new(),
            target_repo: repo_name,
            target_repo_root: repo_root,
            pick: AgentPick::default(),
            field: SpawnField::Task,
            anchor_y,
            error: None,
            area: None,
        };
    }

    pub fn open_spawn_input_from_selection(&mut self) {
        let Some(pane) = self.selected_pane() else {
            self.set_flash("spawn: no pane selected");
            return;
        };
        let pane_id = pane.pane_id.clone();
        let Some(group) = self
            .repo_groups
            .iter()
            .find(|g| g.panes.iter().any(|(p, _)| p.pane_id == pane_id))
        else {
            self.set_flash("spawn: could not find repo group for selection");
            return;
        };
        let Some(root) = group
            .panes
            .iter()
            .find_map(|(_, git)| git.repo_root.clone())
        else {
            self.set_flash("spawn: selected pane is not in a git repo");
            return;
        };
        let name = group.name.clone();
        // Anchor the popup directly below the repo header row so it
        // matches what the mouse `+` click flow does.
        let anchor = self
            .layout
            .repo_spawn_targets
            .iter()
            .find(|t| t.repo_name == name)
            .map(|t| t.rect.y);
        self.open_spawn_input_for_repo(name, root, anchor);
    }

    pub fn close_spawn_input(&mut self) {
        if matches!(self.popup, PopupState::SpawnInput { .. }) {
            self.popup = PopupState::None;
        }
    }

    pub fn spawn_input_next_field(&mut self) {
        if let PopupState::SpawnInput { field, error, .. } = &mut self.popup {
            *field = field.next();
            *error = None;
        }
    }

    pub fn spawn_input_prev_field(&mut self) {
        if let PopupState::SpawnInput { field, error, .. } = &mut self.popup {
            *field = field.prev();
            *error = None;
        }
    }

    /// Cycle the value under the focused agent or mode field. No-op on
    /// the task input field so typing isn't interfered with.
    pub fn spawn_input_cycle(&mut self, delta: isize) {
        let PopupState::SpawnInput {
            field, pick, error, ..
        } = &mut self.popup
        else {
            return;
        };
        match *field {
            SpawnField::Agent => {
                pick.cycle_agent(delta);
                *error = None;
            }
            SpawnField::Mode => {
                pick.cycle_mode(delta);
                *error = None;
            }
            SpawnField::Task => {}
        }
    }

    pub fn spawn_input_push_char(&mut self, c: char) {
        if let PopupState::SpawnInput {
            input,
            field,
            error,
            ..
        } = &mut self.popup
            && *field == SpawnField::Task
        {
            input.push(c);
            *error = None;
        }
    }

    pub fn spawn_input_pop_char(&mut self) {
        if let PopupState::SpawnInput {
            input,
            field,
            error,
            ..
        } = &mut self.popup
            && *field == SpawnField::Task
        {
            input.pop();
            *error = None;
        }
    }

    fn set_spawn_error(&mut self, msg: impl Into<String>) {
        if let PopupState::SpawnInput { error, .. } = &mut self.popup {
            *error = Some(msg.into());
        }
    }

    fn set_remove_error(&mut self, msg: impl Into<String>) {
        if let PopupState::RemoveConfirm { error, .. } = &mut self.popup {
            *error = Some(msg.into());
        }
    }

    /// Run the spawn flow against the repo stored in the popup, using
    /// the agent / mode the user picked. On success the popup closes
    /// silently (the new window appearing in the sidebar is the
    /// feedback). On failure the error is surfaced inside the popup
    /// and the modal stays open so the user can retry.
    pub fn confirm_spawn_input(&mut self) {
        let PopupState::SpawnInput {
            input,
            target_repo_root,
            pick,
            ..
        } = &self.popup
        else {
            return;
        };
        let task_name = input.trim().to_string();
        if task_name.is_empty() {
            self.set_spawn_error("name is empty");
            return;
        }
        let agent = pick.agent_or_default();
        let mode = pick.mode_or_default(&agent);
        let repo_root = std::path::PathBuf::from(target_repo_root.clone());

        let Some(session) = crate::tmux::pane_session_name(&self.tmux_pane) else {
            self.set_spawn_error("could not resolve tmux session");
            return;
        };

        let req = crate::worktree::SpawnRequest {
            repo_root,
            task_name,
            session,
            agent,
            mode,
        };
        match crate::worktree::spawn(&req) {
            Ok(_) => self.popup = PopupState::None,
            Err(e) => self.set_spawn_error(e),
        }
    }

    // ─── Remove confirm popup (x key) ────────────────────────────────────

    pub fn is_remove_confirm_open(&self) -> bool {
        matches!(self.popup, PopupState::RemoveConfirm { .. })
    }

    pub fn remove_confirm_popup_area(&self) -> Option<ratatui::layout::Rect> {
        match &self.popup {
            PopupState::RemoveConfirm { area, .. } => *area,
            _ => None,
        }
    }

    pub fn close_remove_confirm(&mut self) {
        if matches!(self.popup, PopupState::RemoveConfirm { .. }) {
            self.popup = PopupState::None;
        }
    }

    /// Open the remove confirmation popup for the currently selected pane,
    /// but only if it was created by the sidebar's spawn flow. Otherwise
    /// flashes an error so the user knows nothing happened.
    pub fn open_remove_confirm(&mut self) {
        let Some(pane) = self.selected_pane() else {
            self.set_flash("remove: no pane selected");
            return;
        };
        self.open_remove_confirm_for_pane(pane.pane_id.clone());
    }

    pub fn open_remove_confirm_for_pane(&mut self, pane_id: String) {
        let markers = crate::worktree::read_spawn_markers(&pane_id);
        if !markers.is_spawned() {
            self.set_flash("remove: selected pane was not created by sidebar");
            return;
        }
        // Sampled once, here, rather than on every frame: the modal is
        // short-lived and `git status` is not free. The flow-level guard
        // in `remove_with` re-checks at the moment of deletion, so a
        // worktree that turns dirty while the modal is open is still
        // protected.
        let dirty = crate::git::worktree_is_dirty(&markers.worktree_path);
        self.popup = PopupState::RemoveConfirm {
            pane_id,
            branch: remove_confirm_label(&markers),
            dirty,
            error: None,
            area: None,
        };
    }

    /// Run the remove flow on the pane stored in the confirmation popup.
    /// Success silently closes the popup; failures are surfaced inside
    /// the popup so the user can retry.
    ///
    /// `[y]` is refused outright on a dirty worktree — `git worktree
    /// remove --force` would discard the uncommitted work with no undo,
    /// and `x` → `Enter` is a two-keystroke muscle-memory path straight
    /// out of the `o` flow. `[c]` touches no git and stays available.
    pub fn confirm_remove(&mut self, mode: crate::worktree::RemoveMode) {
        let (pane_id, dirty) = match &self.popup {
            PopupState::RemoveConfirm { pane_id, dirty, .. } => (pane_id.clone(), *dirty),
            _ => return,
        };
        if dirty && mode == crate::worktree::RemoveMode::WindowAndWorktree {
            self.set_remove_error("commit or stash first");
            return;
        }
        match crate::worktree::remove(&pane_id, mode) {
            Ok(_) => self.popup = PopupState::None,
            Err(e) => self.set_remove_error(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{NoticesCopyTarget, RepoFilter};
    use super::*;
    use crate::group::{PaneGitInfo, RepoGroup};
    use crate::tmux::{AgentType, PaneInfo, PaneStatus, PermissionMode, WorktreeMetadata};

    fn test_pane(id: &str) -> PaneInfo {
        PaneInfo {
            pane_id: id.into(),
            pane_active: false,
            status: PaneStatus::Running,
            attention: false,
            agent: AgentType::Claude,
            path: "/tmp".into(),
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

    // ─── SpawnField cycle ────────────────────────────────────────────

    #[test]
    fn spawn_field_next_and_prev_cycle() {
        assert_eq!(SpawnField::Task.next(), SpawnField::Agent);
        assert_eq!(SpawnField::Agent.next(), SpawnField::Mode);
        assert_eq!(SpawnField::Mode.next(), SpawnField::Task);
        assert_eq!(SpawnField::Task.prev(), SpawnField::Mode);
        assert_eq!(SpawnField::Agent.prev(), SpawnField::Task);
        assert_eq!(SpawnField::Mode.prev(), SpawnField::Agent);
    }

    // ─── OpenField cycle ─────────────────────────────────────────────

    #[test]
    fn open_field_next_and_prev_cycle() {
        assert_eq!(OpenField::default(), OpenField::Agent);
        assert_eq!(OpenField::Agent.next(), OpenField::Mode);
        assert_eq!(OpenField::Mode.next(), OpenField::Agent);
        assert_eq!(OpenField::Agent.prev(), OpenField::Mode);
        assert_eq!(OpenField::Mode.prev(), OpenField::Agent);
    }

    // ─── AgentPick ───────────────────────────────────────────────────

    #[test]
    fn agent_pick_defaults_to_first_agent_and_mode() {
        let pick = AgentPick::default();
        assert_eq!(pick.agent(), crate::worktree::AGENTS[0]);
        assert_eq!(pick.mode(), crate::worktree::modes_for(pick.agent())[0]);
    }

    #[test]
    fn agent_pick_cycle_agent_wraps_both_directions() {
        let len = crate::worktree::AGENTS.len();
        let mut pick = AgentPick::default();
        for i in 1..=len {
            pick.cycle_agent(1);
            assert_eq!(pick.agent_idx, i % len);
        }
        assert_eq!(pick.agent_idx, 0, "forward cycle wraps to the start");

        pick.cycle_agent(-1);
        assert_eq!(pick.agent_idx, len - 1, "backward cycle wraps to the end");
    }

    #[test]
    fn agent_pick_cycle_agent_resets_mode() {
        let mut pick = AgentPick {
            agent_idx: 0,
            mode_idx: 3,
        };
        pick.cycle_agent(1);
        assert_eq!(
            pick.mode_idx, 0,
            "the mode list is agent-specific, so a stale index must not survive"
        );
    }

    #[test]
    fn agent_pick_cycle_mode_wraps_within_current_agent() {
        // codex has fewer modes than claude — cycling past its end must
        // wrap against *its* list, not claude's.
        let codex_idx = crate::worktree::AGENTS
            .iter()
            .position(|a| *a == "codex")
            .expect("codex is a known agent");
        let mut pick = AgentPick {
            agent_idx: codex_idx,
            mode_idx: 0,
        };
        let len = crate::worktree::CODEX_MODES.len();
        for i in 1..=len {
            pick.cycle_mode(1);
            assert_eq!(pick.mode_idx, i % len);
        }
        assert_eq!(pick.mode_idx, 0);
        pick.cycle_mode(-1);
        assert_eq!(pick.mode_idx, len - 1);
        assert_eq!(pick.mode(), crate::worktree::CODEX_MODES[len - 1]);
    }

    #[test]
    fn agent_pick_out_of_range_indices_are_inert() {
        let pick = AgentPick {
            agent_idx: 99,
            mode_idx: 99,
        };
        assert_eq!(pick.agent(), "");
        assert_eq!(pick.mode(), "");
        // The launch path must still produce something runnable.
        assert_eq!(pick.agent_or_default(), crate::worktree::DEFAULT_AGENT);
        assert_eq!(
            pick.mode_or_default(crate::worktree::DEFAULT_AGENT),
            crate::worktree::DEFAULT_MODE
        );
    }

    // ─── PopupState::set_*_area ──────────────────────────────────────

    #[test]
    fn set_area_updates_only_matching_variant() {
        let mut popup = PopupState::Repo {
            selected: 0,
            area: None,
        };
        let rect = ratatui::layout::Rect::new(1, 2, 3, 4);
        popup.set_repo_area(Some(rect));
        popup.set_notices_area(Some(rect));
        popup.set_spawn_input_area(Some(rect));
        popup.set_remove_confirm_area(Some(rect));
        popup.set_open_worktree_area(Some(rect));
        match popup {
            PopupState::Repo { area, .. } => assert_eq!(area, Some(rect)),
            _ => panic!("variant must remain Repo"),
        }
    }

    // ─── Repo popup ──────────────────────────────────────────────────

    #[test]
    fn toggle_repo_popup_sets_selected_to_current() {
        let mut state = AppState::new("%99".into());
        state.repo_groups = vec![
            RepoGroup {
                name: "alpha".into(),
                has_focus: true,
                panes: vec![],
            },
            RepoGroup {
                name: "beta".into(),
                has_focus: false,
                panes: vec![],
            },
        ];

        state.toggle_repo_popup();
        assert!(state.is_repo_popup_open());
        assert_eq!(state.repo_popup_selected(), 0);

        state.close_repo_popup();
        state.global.repo_filter = RepoFilter::Repo("beta".into());
        state.toggle_repo_popup();
        assert_eq!(state.repo_popup_selected(), 2);
    }

    #[test]
    fn confirm_repo_popup_sets_filter() {
        let mut state = AppState::new("%99".into());
        state.repo_groups = vec![
            RepoGroup {
                name: "alpha".into(),
                has_focus: true,
                panes: vec![(test_pane("%1"), PaneGitInfo::default())],
            },
            RepoGroup {
                name: "beta".into(),
                has_focus: false,
                panes: vec![(test_pane("%2"), PaneGitInfo::default())],
            },
        ];
        state.popup = PopupState::Repo {
            selected: 2,
            area: None,
        };
        state.confirm_repo_popup();

        assert_eq!(state.global.repo_filter, RepoFilter::Repo("beta".into()));
        assert!(!state.is_repo_popup_open());
        assert_eq!(state.layout.pane_row_targets.len(), 1);
        assert_eq!(state.layout.pane_row_targets[0].pane_id, "%2");
    }

    #[test]
    fn confirm_repo_popup_all_resets_filter() {
        let mut state = AppState::new("%99".into());
        state.repo_groups = vec![RepoGroup {
            name: "app".into(),
            has_focus: true,
            panes: vec![(test_pane("%1"), PaneGitInfo::default())],
        }];
        state.global.repo_filter = RepoFilter::Repo("app".into());
        state.popup = PopupState::Repo {
            selected: 0,
            area: None,
        };
        state.confirm_repo_popup();

        assert_eq!(state.global.repo_filter, RepoFilter::All);
    }

    #[test]
    fn close_repo_popup_resets_popup_state() {
        let mut state = AppState::new("%99".into());
        state.popup = PopupState::Repo {
            selected: 5,
            area: Some(ratatui::layout::Rect::new(0, 0, 10, 5)),
        };
        assert!(state.is_repo_popup_open());
        state.close_repo_popup();
        assert!(matches!(state.popup, PopupState::None));
        assert!(state.repo_popup_area().is_none());
    }

    #[test]
    fn toggle_repo_popup_twice_closes() {
        let mut state = AppState::new("%99".into());
        state.toggle_repo_popup();
        assert!(state.is_repo_popup_open());
        state.toggle_repo_popup();
        assert!(!state.is_repo_popup_open());
    }

    #[test]
    fn set_repo_popup_selected_noop_when_other_variant() {
        let mut state = AppState::new("%99".into());
        state.popup = PopupState::Notices { area: None };
        state.set_repo_popup_selected(9);
        // Repo accessor returns default when not in Repo variant.
        assert_eq!(state.repo_popup_selected(), 0);
    }

    // ─── Notices popup ───────────────────────────────────────────────

    #[test]
    fn toggle_notices_popup_opens_then_closes() {
        let mut state = AppState::new("%99".into());
        state.toggle_notices_popup();
        assert!(state.is_notices_popup_open());
        state.toggle_notices_popup();
        assert!(!state.is_notices_popup_open());
    }

    #[test]
    fn close_notices_popup_clears_copy_state() {
        let mut state = AppState::new("%99".into());
        state.popup = PopupState::Notices { area: None };
        state.notices.copy_targets = vec![NoticesCopyTarget {
            area: ratatui::layout::Rect::new(0, 0, 5, 1),
            agent: "claude".into(),
        }];
        state.notices.copied_at = Some(("claude".into(), std::time::Instant::now()));

        state.close_notices_popup();

        assert!(!state.is_notices_popup_open());
        assert!(state.notices.copy_targets.is_empty());
        assert!(state.notices.copied_at.is_none());
    }

    #[test]
    fn notices_popup_area_none_when_closed() {
        let state = AppState::new("%99".into());
        assert!(state.notices_popup_area().is_none());
    }

    // ─── Spawn input popup ───────────────────────────────────────────

    #[test]
    fn open_spawn_input_for_repo_initializes_fields() {
        let mut state = AppState::new("%99".into());
        state.open_spawn_input_for_repo("alpha".into(), "/tmp/alpha".into(), Some(7));
        assert!(state.is_spawn_input_open());
        if let PopupState::SpawnInput {
            input,
            target_repo,
            target_repo_root,
            pick,
            field,
            anchor_y,
            error,
            area,
        } = &state.popup
        {
            assert!(input.is_empty());
            assert_eq!(target_repo, "alpha");
            assert_eq!(target_repo_root, "/tmp/alpha");
            assert_eq!(*pick, AgentPick::default());
            assert_eq!(*field, SpawnField::Task);
            assert_eq!(*anchor_y, Some(7));
            assert!(error.is_none());
            assert!(area.is_none());
        } else {
            panic!("expected SpawnInput, got {:?}", state.popup);
        }
    }

    #[test]
    fn close_spawn_input_noop_when_not_open() {
        let mut state = AppState::new("%99".into());
        state.popup = PopupState::Notices { area: None };
        state.close_spawn_input();
        // The other popup must not be touched.
        assert!(matches!(state.popup, PopupState::Notices { .. }));
    }

    #[test]
    fn spawn_input_next_prev_cycle_field() {
        let mut state = AppState::new("%99".into());
        state.open_spawn_input_for_repo("alpha".into(), "/tmp/alpha".into(), None);
        state.spawn_input_next_field();
        assert!(matches!(
            state.popup,
            PopupState::SpawnInput {
                field: SpawnField::Agent,
                ..
            }
        ));
        state.spawn_input_next_field();
        assert!(matches!(
            state.popup,
            PopupState::SpawnInput {
                field: SpawnField::Mode,
                ..
            }
        ));
        state.spawn_input_prev_field();
        assert!(matches!(
            state.popup,
            PopupState::SpawnInput {
                field: SpawnField::Agent,
                ..
            }
        ));
    }

    #[test]
    fn spawn_input_field_change_clears_error() {
        let mut state = AppState::new("%99".into());
        state.open_spawn_input_for_repo("alpha".into(), "/tmp/alpha".into(), None);
        if let PopupState::SpawnInput { error, .. } = &mut state.popup {
            *error = Some("boom".into());
        }
        state.spawn_input_next_field();
        if let PopupState::SpawnInput { error, .. } = &state.popup {
            assert!(error.is_none());
        }
    }

    #[test]
    fn spawn_input_cycle_agent_wraps_and_resets_mode() {
        let mut state = AppState::new("%99".into());
        state.open_spawn_input_for_repo("alpha".into(), "/tmp/alpha".into(), None);
        // Switch field to Agent, bump mode_idx artificially, then cycle to
        // verify mode_idx resets to 0 on agent change.
        if let PopupState::SpawnInput { field, pick, .. } = &mut state.popup {
            *field = SpawnField::Agent;
            pick.mode_idx = 1;
        }
        state.spawn_input_cycle(1);
        if let PopupState::SpawnInput { pick, .. } = &state.popup {
            assert_eq!(pick.agent_idx, 1 % crate::worktree::AGENTS.len());
            assert_eq!(pick.mode_idx, 0);
        }
    }

    #[test]
    fn spawn_input_cycle_task_field_is_noop() {
        let mut state = AppState::new("%99".into());
        state.open_spawn_input_for_repo("alpha".into(), "/tmp/alpha".into(), None);
        state.spawn_input_cycle(1);
        if let PopupState::SpawnInput { pick, .. } = &state.popup {
            assert_eq!(*pick, AgentPick::default());
        }
    }

    #[test]
    fn spawn_input_push_pop_char_only_on_task_field() {
        let mut state = AppState::new("%99".into());
        state.open_spawn_input_for_repo("alpha".into(), "/tmp/alpha".into(), None);

        state.spawn_input_push_char('h');
        state.spawn_input_push_char('i');
        if let PopupState::SpawnInput { input, .. } = &state.popup {
            assert_eq!(input, "hi");
        }

        // Switch field — push/pop must have no effect on input text.
        state.spawn_input_next_field();
        state.spawn_input_push_char('x');
        state.spawn_input_pop_char();
        if let PopupState::SpawnInput { input, .. } = &state.popup {
            assert_eq!(input, "hi");
        }
    }

    #[test]
    fn confirm_spawn_input_empty_sets_error() {
        let mut state = AppState::new("%99".into());
        state.open_spawn_input_for_repo("alpha".into(), "/tmp/alpha".into(), None);
        state.confirm_spawn_input();
        if let PopupState::SpawnInput { error, .. } = &state.popup {
            assert_eq!(error.as_deref(), Some("name is empty"));
        } else {
            panic!("popup must stay open on error");
        }
    }

    // ─── Remove confirm popup ────────────────────────────────────────

    #[test]
    fn remove_confirm_accessors_and_close() {
        let mut state = AppState::new("%99".into());
        state.popup = PopupState::RemoveConfirm {
            pane_id: "%1".into(),
            branch: "feature/x".into(),
            dirty: false,
            error: None,
            area: Some(ratatui::layout::Rect::new(0, 0, 20, 5)),
        };
        assert!(state.is_remove_confirm_open());
        assert_eq!(
            state.remove_confirm_popup_area(),
            Some(ratatui::layout::Rect::new(0, 0, 20, 5))
        );
        state.close_remove_confirm();
        assert!(!state.is_remove_confirm_open());
        assert!(state.remove_confirm_popup_area().is_none());
    }

    #[test]
    fn confirm_remove_refuses_worktree_deletion_on_a_dirty_worktree() {
        // `[y]` runs `git worktree remove --force`, which discards
        // uncommitted work with no undo. The guard must fire before
        // `worktree::remove` is reached — the popup stays open with an
        // inline reason instead.
        let mut state = AppState::new("%99".into());
        state.popup = PopupState::RemoveConfirm {
            pane_id: "%1".into(),
            branch: "feature/x".into(),
            dirty: true,
            error: None,
            area: None,
        };
        state.confirm_remove(crate::worktree::RemoveMode::WindowAndWorktree);
        match &state.popup {
            PopupState::RemoveConfirm { error, .. } => {
                assert_eq!(error.as_deref(), Some("commit or stash first"))
            }
            _ => panic!("popup must stay open so the user can pick [c] instead"),
        }
    }

    #[test]
    fn confirm_remove_still_allows_window_only_close_on_a_dirty_worktree() {
        // `[c]` touches no git, so uncommitted work is no reason to
        // block it — blocking it would leave no way to close the window
        // from the sidebar at all.
        let mut state = AppState::new("%99".into());
        state.popup = PopupState::RemoveConfirm {
            pane_id: "%1".into(),
            branch: "feature/x".into(),
            dirty: true,
            error: None,
            area: None,
        };
        state.confirm_remove(crate::worktree::RemoveMode::WindowOnly);
        // No tmux server in tests, so `remove` fails — but on the
        // *flow's* error, not the dirty guard's.
        match &state.popup {
            PopupState::RemoveConfirm { error, .. } => assert_ne!(
                error.as_deref(),
                Some("commit or stash first"),
                "the dirty guard must not apply to the window-only close"
            ),
            PopupState::None => {}
            _ => panic!("unexpected popup variant"),
        }
    }

    #[test]
    fn confirm_remove_noop_when_popup_absent() {
        // When no RemoveConfirm popup is open, `confirm_remove` must early-return
        // without touching state. We verify nothing blew up and the popup
        // variant is preserved.
        let mut state = AppState::new("%99".into());
        state.popup = PopupState::Notices { area: None };
        state.confirm_remove(crate::worktree::RemoveMode::WindowOnly);
        assert!(matches!(state.popup, PopupState::Notices { .. }));
    }

    #[test]
    fn remove_confirm_label_names_the_branch_that_gets_deleted() {
        // The title is what the user reads before pressing `y`, and `y`
        // runs `git branch -D` on the branch marker. An opened worktree
        // can live in a directory named nothing like its branch, so the
        // directory basename would confirm a deletion of the wrong name.
        let markers = crate::worktree::SpawnMarkers {
            spawned: true,
            from_repo: "/repo".into(),
            worktree_path: "/home/u/wt/hotfix".into(),
            branch: "feature/JIRA-4821-payment-retry".into(),
            window_id: "@1".into(),
            opened: true,
        };
        assert_eq!(
            remove_confirm_label(&markers),
            "feature/JIRA-4821-payment-retry"
        );
    }

    #[test]
    fn remove_confirm_label_falls_back_to_dir_name_when_detached() {
        let markers = crate::worktree::SpawnMarkers {
            spawned: true,
            from_repo: "/repo".into(),
            worktree_path: "/repo/.worktrees/spike".into(),
            branch: String::new(),
            window_id: "@1".into(),
            opened: true,
        };
        assert_eq!(remove_confirm_label(&markers), "spike");
    }

    #[test]
    fn remove_confirm_label_is_empty_when_nothing_identifies_the_target() {
        assert_eq!(
            remove_confirm_label(&crate::worktree::SpawnMarkers::default()),
            ""
        );
    }

    #[test]
    fn open_remove_confirm_without_selection_flashes() {
        let mut state = AppState::new("%99".into());
        state.open_remove_confirm();
        assert!(state.take_flash().is_some());
        assert!(!state.is_remove_confirm_open());
    }
}
