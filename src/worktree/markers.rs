use crate::tmux;

pub const SPAWNED_OPTION: &str = "@agent-sidebar-spawned";
pub const SPAWNED_FROM_OPTION: &str = "@agent-sidebar-spawned-from";
pub const SPAWNED_WORKTREE_OPTION: &str = "@agent-sidebar-spawned-worktree";
pub const SPAWNED_BRANCH_OPTION: &str = "@agent-sidebar-spawned-branch";
/// Set by the `o` (open existing worktree) flow *in addition to*
/// [`SPAWNED_OPTION`], which `o` also sets so `x` treats the window
/// exactly like a spawned one. This marker is therefore the only
/// record that the sidebar created the window but **not** the git
/// state behind it — worth knowing before `x` offers to
/// `git worktree remove --force` it.
pub const OPENED_OPTION: &str = "@agent-sidebar-opened";

/// Build the tmux `display-message` template used by [`read_spawn_markers`]. One
/// call, six fields: the truthy flag, the owning repo, the worktree
/// path, the branch name, the window id, and the "opened, not spawned"
/// flag. Callers share this template so the remove confirmation popup
/// and the remove flow itself always read the same set of fields in the
/// same order. The `opened` flag is appended last so the original
/// five-field parse order stays untouched.
pub fn spawn_markers_template() -> String {
    [
        format!("#{{{SPAWNED_OPTION}}}"),
        format!("#{{{SPAWNED_FROM_OPTION}}}"),
        format!("#{{{SPAWNED_WORKTREE_OPTION}}}"),
        format!("#{{{SPAWNED_BRANCH_OPTION}}}"),
        "#{window_id}".to_string(),
        format!("#{{{OPENED_OPTION}}}"),
    ]
    .join("\n")
}

/// Parsed view of the window-scope markers the spawn/remove flow
/// depends on. All fields are always present because
/// `display-message` returns empty strings for missing keys —
/// [`SpawnMarkers::is_spawned`] is the canonical check. The remove
/// flow also requires `worktree_path`, `branch`, and `window_id` to
/// be populated and errors out otherwise; `spawn_with` always writes
/// all four markers atomically (with rollback on partial failure),
/// so a pane in the wild either has the full set or the remove flow
/// correctly refuses to touch it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpawnMarkers {
    pub spawned: bool,
    pub from_repo: String,
    pub worktree_path: String,
    pub branch: String,
    pub window_id: String,
    /// Window was opened on a pre-existing worktree by the `o` flow.
    /// Implies `spawned` in practice, since `o` sets both — see
    /// [`OPENED_OPTION`].
    pub opened: bool,
}

impl SpawnMarkers {
    pub fn is_spawned(&self) -> bool {
        self.spawned && !self.from_repo.is_empty()
    }

    /// `true` when the window was created by the open-existing-worktree
    /// flow. Narrower than [`Self::is_spawned`], which such a window
    /// also satisfies: this one additionally says the worktree and
    /// branch predate the sidebar.
    pub fn is_opened(&self) -> bool {
        self.opened && !self.from_repo.is_empty()
    }

    /// Parse the output of `tmux display-message -p -F spawn_markers_template()`.
    /// Missing / empty fields become `""` / `false` rather than errors.
    pub fn parse(raw: &str) -> Self {
        let mut lines = raw.lines();
        let spawned = lines.next().unwrap_or("") == "1";
        let from_repo = lines.next().unwrap_or("").to_string();
        let worktree_path = lines.next().unwrap_or("").to_string();
        let branch = lines.next().unwrap_or("").to_string();
        let window_id = lines.next().unwrap_or("").to_string();
        let opened = lines.next().unwrap_or("") == "1";
        Self {
            spawned,
            from_repo,
            worktree_path,
            branch,
            window_id,
            opened,
        }
    }
}

/// Read the spawn markers for `pane_id` through tmux `display-message`,
/// which falls through pane → window scope. The markers are stored at
/// window scope so sub panes (e.g. Claude Code subagents split from the
/// original) still resolve them; a pane-scope lookup would miss them.
pub fn read_spawn_markers(pane_id: &str) -> SpawnMarkers {
    SpawnMarkers::parse(&tmux::display_message(pane_id, &spawn_markers_template()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_all_fields_populated() {
        let raw = "1\n/repo\n/repo/.worktrees/foo\nagent/foo\n@42\n";
        let m = SpawnMarkers::parse(raw);
        assert!(m.spawned);
        assert_eq!(m.from_repo, "/repo");
        assert_eq!(m.worktree_path, "/repo/.worktrees/foo");
        assert_eq!(m.branch, "agent/foo");
        assert_eq!(m.window_id, "@42");
        assert!(m.is_spawned());
        assert!(!m.opened, "spawn markers must not read as opened");
    }

    #[test]
    fn parse_missing_trailing_fields_default_to_empty() {
        let m = SpawnMarkers::parse("1\n/repo\n");
        assert!(m.spawned);
        assert_eq!(m.from_repo, "/repo");
        assert!(m.worktree_path.is_empty());
        assert!(m.branch.is_empty());
        assert!(m.window_id.is_empty());
        assert!(!m.opened);
    }

    #[test]
    fn spawn_markers_template_appends_opened_flag_last() {
        let template = spawn_markers_template();
        let lines: Vec<&str> = template.lines().collect();
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[0], format!("#{{{SPAWNED_OPTION}}}"));
        assert_eq!(lines[4], "#{window_id}");
        assert_eq!(
            lines[5],
            format!("#{{{OPENED_OPTION}}}"),
            "opened must stay last so the five-field parse order is untouched"
        );
    }

    #[test]
    fn parse_opened_window_is_both_spawned_and_opened() {
        // What `open_with` actually writes: both flags set.
        let raw = "1\n/repo\n/repo/.worktrees/foo\nagent/foo\n@42\n1\n";
        let m = SpawnMarkers::parse(raw);
        assert!(m.is_opened());
        assert!(
            m.is_spawned(),
            "`x` keys off is_spawned, so an opened window must satisfy it"
        );
    }

    #[test]
    fn parse_reads_the_two_flags_independently() {
        // The parser itself does not couple the flags; only the open
        // flow happens to set both.
        let m = SpawnMarkers::parse("\n/repo\n/wt\nb\n@42\n1\n");
        assert!(m.is_opened());
        assert!(!m.is_spawned());
    }

    #[test]
    fn is_opened_requires_both_flag_and_repo() {
        let mut m = SpawnMarkers {
            opened: true,
            from_repo: String::new(),
            ..Default::default()
        };
        assert!(!m.is_opened(), "flag alone is insufficient");
        m.from_repo = "/repo".into();
        assert!(m.is_opened());
        m.opened = false;
        assert!(!m.is_opened(), "repo alone is insufficient");
    }

    #[test]
    fn parse_empty_input_yields_default() {
        let m = SpawnMarkers::parse("");
        assert_eq!(m, SpawnMarkers::default());
        assert!(!m.is_spawned());
    }

    #[test]
    fn is_spawned_requires_both_flag_and_repo() {
        let mut m = SpawnMarkers {
            spawned: true,
            from_repo: String::new(),
            ..Default::default()
        };
        assert!(!m.is_spawned(), "flag alone is insufficient");
        m.from_repo = "/repo".into();
        assert!(m.is_spawned());
        m.spawned = false;
        assert!(!m.is_spawned(), "repo alone is insufficient");
    }

    #[test]
    fn parse_non_one_flag_is_false() {
        // Only literal "1" counts as spawned; any other value (including
        // "true", "yes", "0") reads as false.
        assert!(!SpawnMarkers::parse("true\n/repo\n").spawned);
        assert!(!SpawnMarkers::parse("0\n/repo\n").spawned);
        assert!(!SpawnMarkers::parse("\n/repo\n").spawned);
    }
}
