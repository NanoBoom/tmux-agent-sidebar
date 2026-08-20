use crate::cli::plugin_state;
use crate::session;
use crate::state::AppState;
use crate::ui;

/// Construct and prime the initial [`AppState`] before the event loop starts.
///
/// Equivalent to the original `run_app` prelude in `src/main.rs`: installs the
/// color theme/icons from tmux options, loads global filter state, resolves
/// the Claude plugin install version once at startup, seeds session names
/// synchronously so `/rename` labels render on the first frame, and performs
/// the first refresh pass.
pub(super) fn init_state(tmux_pane: String) -> AppState {
    let mut state = AppState::new(tmux_pane);
    // One `show-options -g` for every startup-scoped option. Each of these
    // readers has a `*_from_tmux()` twin that fetches the map itself; going
    // through the map here keeps startup at a single tmux round-trip rather
    // than one per option.
    let opts = crate::tmux::get_all_global_options();
    state.theme = ui::colors::ColorTheme::from_options(&opts);
    state.icons = ui::icons::StatusIcons::from_options(&opts);
    state.bottom_panel_height = ui::bottom_panel_height_from_options(&opts);
    state.bottom_panel_enabled = ui::bottom_enabled_from_options(&opts);
    state.pet_enabled = ui::pet_enabled_from_options(&opts);
    state.show_session_name = ui::show_session_name_from_options(&opts);
    state.global.load_from_tmux();
    state.refresh();

    // Git data only ever reaches the Git tab, so skip the (blocking)
    // startup fetch when the bottom panel is off.
    if state.bottom_panel_visible() {
        super::render::refresh_git_for_focused_pane(&mut state);
    }

    // Resolve the installed Claude Code plugin status once at startup,
    // matching the version_notice pattern. Restart the sidebar after a
    // /plugin install, /plugin uninstall, or /plugin update to pick up
    // the new state.
    state.notices.claude_plugin_status = plugin_state::installed_plugin_status();
    // Likewise resolve whether the user still has legacy
    // tmux-agent-sidebar/hook.sh entries in ~/.claude/settings.json so
    // the notices popup can warn about duplicate hook execution.
    state.notices.claude_settings_has_residual_hooks =
        plugin_state::claude_settings_has_residual_hooks();
    // Notice inputs are static after the two lines above, so compute
    // them once here instead of from the per-tick refresh loop. This
    // also decouples the ⓘ badge from `focused_pane_id`, so killing
    // the last agent pane no longer drops outstanding setup warnings.
    state.refresh_notices();
    // Populate session names synchronously before the first draw so
    // `/rename`-assigned labels show up without waiting for the first
    // background scan tick. Skipped entirely when the labels are off —
    // an empty map is exactly what `refresh_session_names` wants then,
    // and it saves a `~/.claude/sessions/` walk on the startup path.
    if state.show_session_name {
        state.sessions.names = session::scan_session_names();
        state.sessions.dirty = true;
    }
    state.refresh();

    state
}
