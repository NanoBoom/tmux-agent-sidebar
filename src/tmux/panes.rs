use super::commands::{display_message, run_tmux};

/// Everything the sidebar needs to know about itself, its window and its
/// session, resolved from a single `display-message` on its own pane id.
/// Keeping it one query is the point: the sidebar re-reads this every
/// second, so extra fields must not cost extra tmux round-trips.
///
/// [`Default`] is the "tmux told us nothing" value: both flags `false`
/// and every count `None`, which every close decision reads as "cannot
/// prove this is safe".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SidebarPaneInfo {
    /// The sidebar pane itself holds tmux focus.
    pub pane_active: bool,
    /// The sidebar's window is the active window of its session.
    pub window_active: bool,
    /// Panes in the sidebar's own window, *including* the sidebar.
    /// `None` when tmux did not answer — the pane is already gone or the
    /// server was too busy to reply.
    pub window_panes: Option<u32>,
    /// Windows in the sidebar's session, or `None` if tmux did not answer.
    pub session_windows: Option<u32>,
    /// Clients attached to the sidebar's session, or `None` if tmux did
    /// not answer.
    pub session_attached: Option<u32>,
    /// `@sidebar_auto_close`, resolved live rather than cached at
    /// startup: it rides along in the same `display-message`, so reading
    /// it every tick is free and a user who turns it off does not have
    /// to restart the sidebar for that to take effect.
    pub auto_close_enabled: bool,
}

impl SidebarPaneInfo {
    /// The sidebar is the only pane left in its window. The sidebar is
    /// always one of the panes counted, so `1` means "nothing but us".
    pub fn is_alone_in_window(&self) -> bool {
        self.window_panes == Some(1)
    }

    /// The sidebar should tear itself down: `@sidebar_auto_close` is on,
    /// it is alone in its window, and destroying that window will not
    /// strand other tmux clients.
    pub fn should_self_close(&self) -> bool {
        self.auto_close_enabled
            && self.is_alone_in_window()
            && session_safe_to_close(self.session_windows, self.session_attached)
    }
}

/// Pipe-separated because the last field is a user-set option value that
/// may contain spaces; everything before it is a tmux-generated number
/// or flag that never can.
const SIDEBAR_PANE_INFO_FORMAT: &str = "#{pane_active}|#{window_active}|#{window_panes}\
     |#{session_windows}|#{session_attached}|#{@sidebar_auto_close}";

pub fn get_sidebar_pane_info(tmux_pane: &str) -> SidebarPaneInfo {
    parse_sidebar_pane_info(&display_message(tmux_pane, SIDEBAR_PANE_INFO_FORMAT))
}

/// Pure parser for [`SIDEBAR_PANE_INFO_FORMAT`] output. A truncated
/// answer degrades field by field rather than wholesale, but anything
/// missing stays `None` so a half-answer can never read as "safe to
/// close".
fn parse_sidebar_pane_info(out: &str) -> SidebarPaneInfo {
    let fields: Vec<&str> = out.split('|').collect();
    if fields.len() < 2 {
        return SidebarPaneInfo::default();
    }
    let count = |index: usize| -> Option<u32> {
        fields
            .get(index)
            .and_then(|field| field.trim().parse().ok())
    };
    SidebarPaneInfo {
        pane_active: fields[0].trim() == "1",
        window_active: fields[1].trim() == "1",
        window_panes: count(2),
        session_windows: count(3),
        session_attached: count(4),
        // Unset (or absent, on an older sidebar's format) means on: the
        // default, and what `agent-sidebar.conf` seeds. Only an explicit
        // non-truthy value turns the feature off.
        auto_close_enabled: fields
            .get(5)
            .map(|field| field.trim())
            .filter(|field| !field.is_empty())
            .is_none_or(super::options::parse_bool_option),
    }
}

/// Whether tearing down the sidebar's window is safe for the session it
/// lives in.
///
/// Closing the last pane of a window destroys the window, and destroying
/// the last window of a session destroys the session — dropping every
/// client attached to it. One attached client is fine: that is exactly
/// what tmux already does when the last pane of a session exits, so the
/// user expects the sidebar to go with it. Two or more means a shared
/// session (e.g. several terminal tabs attached to `main`) where we
/// cannot tell which clients are wanted, so the sidebar stays instead.
/// A `None` anywhere means the query failed and the close cannot be
/// proven safe — err on the side of preservation.
pub(crate) fn session_safe_to_close(
    session_windows: Option<u32>,
    session_attached: Option<u32>,
) -> bool {
    match session_windows {
        None | Some(0) => false,
        Some(1) => matches!(session_attached, Some(n) if n <= 1),
        Some(_) => true,
    }
}

pub fn get_pane_path(pane_id: &str) -> Option<String> {
    Some(display_message(pane_id, "#{pane_current_path}")).filter(|s| !s.is_empty())
}

/// Query tmux for all panes in the active window, returning (pane_id, pane_active, path).
/// This queries tmux directly and is NOT filtered by agent type, so it includes
/// all panes (shell, editor, etc.) — not just agent panes.
pub fn query_active_window_panes() -> Vec<(String, bool, String)> {
    // List panes in the current (active) window across all sessions
    let output = match run_tmux(&[
        "list-panes",
        "-F",
        "#{pane_id}|#{pane_active}|#{pane_current_path}",
    ]) {
        Some(s) => s,
        None => return vec![],
    };
    output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(3, '|').collect();
            if parts.len() < 3 {
                return None;
            }
            Some((parts[0].to_string(), parts[1] == "1", parts[2].to_string()))
        })
        .collect()
}

/// Find the focused (non-sidebar) pane ID and path by querying tmux directly.
/// Returns all panes regardless of agent type, so activity/git info can be shown
/// even for non-agent panes.
pub fn find_active_pane(sidebar_pane: &str) -> Option<(String, String)> {
    pick_active_pane(sidebar_pane, &query_active_window_panes())
}

/// Pure logic: pick the active non-sidebar pane from a list.
/// Returns the pane with pane_active=true (excluding sidebar) if one exists.
/// Returns None when the sidebar itself is active or no valid pane is found,
/// so callers can preserve the previously focused pane.
pub(crate) fn pick_active_pane(
    sidebar_pane: &str,
    panes: &[(String, bool, String)],
) -> Option<(String, String)> {
    let valid = |p: &&(String, bool, String)| p.0 != sidebar_pane && !p.2.is_empty();
    panes
        .iter()
        .find(|p| p.1 && valid(p))
        .map(|p| (p.0.clone(), p.2.clone()))
}

/// Find the focused pane's working directory by querying tmux directly.
/// Used by the background git thread which doesn't have access to AppState.
/// Queries all panes (not just agent panes) so git info is available
/// even when the focused pane has no agent running.
pub fn focused_pane_path(sidebar_pane: &str) -> Option<String> {
    find_active_pane(sidebar_pane).map(|(_, path)| path)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── sidebar pane info parsing ───────────────────────────────────

    /// Live `display-message` output captured from tmux 3.7b for a
    /// sidebar sharing its window with one other pane.
    #[test]
    fn parse_sidebar_pane_info_reads_all_six_fields() {
        let info = parse_sidebar_pane_info("0|1|2|3|1|on");
        assert_eq!(
            info,
            SidebarPaneInfo {
                pane_active: false,
                window_active: true,
                window_panes: Some(2),
                session_windows: Some(3),
                session_attached: Some(1),
                auto_close_enabled: true,
            }
        );
    }

    #[test]
    fn parse_sidebar_pane_info_falls_back_when_tmux_says_nothing() {
        // `display_message` returns an empty string when the pane is
        // already gone. Every close-relevant field must stay unknown.
        for out in ["", "   "] {
            let info = parse_sidebar_pane_info(out);
            assert_eq!(info, SidebarPaneInfo::default(), "output {out:?}");
            assert!(!info.should_self_close(), "output {out:?}");
        }
    }

    #[test]
    fn parse_sidebar_pane_info_keeps_flags_when_counts_are_missing() {
        // A truncated answer still yields usable focus flags, but the
        // counts stay `None` so no close decision can be made from it.
        let info = parse_sidebar_pane_info("1|1");
        assert!(info.pane_active && info.window_active);
        assert_eq!(info.window_panes, None);
        assert!(!info.should_self_close());
    }

    #[test]
    fn parse_sidebar_pane_info_ignores_unparseable_counts() {
        let info = parse_sidebar_pane_info("1|1|x|y|z|on");
        assert_eq!(info.window_panes, None);
        assert_eq!(info.session_windows, None);
        assert_eq!(info.session_attached, None);
        assert!(!info.should_self_close());
    }

    #[test]
    fn parse_sidebar_pane_info_reads_auto_close_with_the_shared_parser() {
        for (value, expected) in [
            ("on", true),
            ("ON", true),
            ("true", true),
            ("1", true),
            ("yes", true),
            (" on ", true),
            ("off", false),
            ("OFF", false),
            ("false", false),
            ("0", false),
            ("no", false),
            ("bogus", false),
        ] {
            let info = parse_sidebar_pane_info(&format!("1|1|1|2|1|{value}"));
            assert_eq!(info.auto_close_enabled, expected, "value {value:?}");
            assert_eq!(info.should_self_close(), expected, "value {value:?}");
        }
    }

    #[test]
    fn parse_sidebar_pane_info_treats_an_unset_auto_close_as_on() {
        // `@sidebar_auto_close` expands to the empty string when nobody
        // set it — e.g. the binary run without `agent-sidebar.conf`.
        // The documented default is on, so only an explicit non-truthy
        // value may disable the feature.
        for out in ["1|1|1|2|1|", "1|1|1|2|1|   ", "1|1|1|2|1"] {
            let info = parse_sidebar_pane_info(out);
            assert!(info.auto_close_enabled, "output {out:?}");
            assert!(info.should_self_close(), "output {out:?}");
        }
    }

    #[test]
    fn parse_sidebar_pane_info_survives_a_spaced_option_value() {
        // The option value is the only user-controlled field, hence the
        // pipe separator: a space-separated format would mis-parse it.
        let info = parse_sidebar_pane_info("1|1|1|2|1|not a bool");
        assert_eq!(info.window_panes, Some(1));
        assert!(!info.auto_close_enabled);
        assert!(!info.should_self_close());
    }

    // ─── self-close decision ─────────────────────────────────────────

    fn info(window_panes: u32, session_windows: u32, session_attached: u32) -> SidebarPaneInfo {
        SidebarPaneInfo {
            window_panes: Some(window_panes),
            session_windows: Some(session_windows),
            session_attached: Some(session_attached),
            auto_close_enabled: true,
            ..SidebarPaneInfo::default()
        }
    }

    #[test]
    fn should_not_self_close_when_the_option_is_off() {
        assert!(
            !SidebarPaneInfo {
                auto_close_enabled: false,
                ..info(1, 2, 1)
            }
            .should_self_close()
        );
    }

    #[test]
    fn should_self_close_when_alone_and_session_has_other_windows() {
        assert!(info(1, 2, 1).should_self_close());
        assert!(info(1, 2, 5).should_self_close());
    }

    #[test]
    fn should_not_self_close_while_a_neighbour_remains() {
        // The sidebar counts itself, so 2 means one real pane is left.
        assert!(!info(2, 2, 1).should_self_close());
        assert!(!info(7, 2, 1).should_self_close());
    }

    #[test]
    fn should_self_close_in_last_window_only_for_a_single_client() {
        // Closing the last window ends the session. One (or zero)
        // attached client matches tmux's own `exit`-on-last-pane
        // behaviour; two or more would mass-disconnect a shared session.
        assert!(info(1, 1, 0).should_self_close());
        assert!(info(1, 1, 1).should_self_close());
        assert!(!info(1, 1, 2).should_self_close());
        assert!(!info(1, 1, 9).should_self_close());
    }

    #[test]
    fn should_not_self_close_when_a_count_is_unknown() {
        // Cannot prove the teardown is safe → keep the sidebar.
        let unknown_windows = SidebarPaneInfo {
            window_panes: Some(1),
            session_windows: None,
            session_attached: Some(1),
            ..SidebarPaneInfo::default()
        };
        assert!(!unknown_windows.should_self_close());

        let unknown_attached = SidebarPaneInfo {
            window_panes: Some(1),
            session_windows: Some(1),
            session_attached: None,
            ..SidebarPaneInfo::default()
        };
        assert!(!unknown_attached.should_self_close());

        assert!(!info(1, 0, 1).should_self_close());
    }

    #[test]
    fn session_safe_to_close_matches_should_kill_window_semantics() {
        // Shared with `cli::toggle::should_kill_window`; the two paths
        // must never disagree about when a teardown is acceptable.
        assert!(session_safe_to_close(Some(2), None));
        assert!(session_safe_to_close(Some(1), Some(1)));
        assert!(!session_safe_to_close(Some(1), Some(2)));
        assert!(!session_safe_to_close(Some(1), None));
        assert!(!session_safe_to_close(None, Some(1)));
        assert!(!session_safe_to_close(Some(0), Some(1)));
    }

    // ─── active pane selection ───────────────────────────────────────

    #[test]
    fn pick_active_pane_returns_active_non_sidebar() {
        let panes = vec![
            ("%1".into(), false, "/home".into()),
            ("%2".into(), true, "/work".into()),
            ("%3".into(), false, "/tmp".into()),
        ];
        assert_eq!(
            pick_active_pane("%99", &panes),
            Some(("%2".into(), "/work".into()))
        );
    }

    #[test]
    fn pick_active_pane_skips_sidebar_even_when_marked_active() {
        let panes = vec![("%99".into(), true, "/a".into())];
        assert!(pick_active_pane("%99", &panes).is_none());
    }

    #[test]
    fn pick_active_pane_skips_panes_with_empty_path() {
        let panes = vec![
            ("%1".into(), true, "".into()),
            ("%2".into(), true, "/ok".into()),
        ];
        assert_eq!(
            pick_active_pane("%99", &panes),
            Some(("%2".into(), "/ok".into()))
        );
    }

    #[test]
    fn pick_active_pane_returns_none_for_empty_list() {
        assert!(pick_active_pane("%99", &[]).is_none());
    }

    #[test]
    fn pick_active_pane_returns_none_when_no_active() {
        let panes = vec![
            ("%1".into(), false, "/x".into()),
            ("%2".into(), false, "/y".into()),
        ];
        assert!(pick_active_pane("%99", &panes).is_none());
    }
}
