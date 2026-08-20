//! Rendering for the two-step "open an existing worktree" modal (`o`).
//! Step 1 is a scrollable worktree picker; step 2 reuses the spawn
//! modal's expanded/compact split with a read-only BRANCH row in place
//! of the editable NAME field.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use super::{
    COMPACT_AGENT_Y, COMPACT_CONTENT_ROWS, COMPACT_EDITOR_Y, COMPACT_ERROR_Y, COMPACT_MODE_Y,
    COMPACT_TASK_Y, EXP_AGENT_LABEL_Y, EXP_AGENT_VALUE_Y, EXP_EDITOR_LABEL_Y, EXP_EDITOR_VALUE_Y,
    EXP_ERROR_Y, EXP_MODE_LABEL_Y, EXP_MODE_VALUE_Y, EXP_TASK_LABEL_Y, EXP_TASK_VALUE_Y,
    EXPANDED_CONTENT_ROWS, POPUP_BORDER_ROWS, POPUP_ERROR_ROWS, SPAWN_MODAL_EXPANDED_MIN_HEIGHT,
    anchor_below, center_popup, tail_fit,
};
use crate::state::popup::open_worktree::clamp_scroll;
use crate::state::{AppState, OpenField, OpenStep, OpenWorktreeRow, PopupState};
use crate::ui::text::{display_width, truncate_to_width};

/// Marker drawn on rows whose worktree already hosts a sidebar-visible
/// pane. Informational only — `Enter` still opens a new window.
const IN_USE_MARKER: &str = "●";

pub(super) fn render_open_worktree_popup(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let PopupState::OpenWorktree { step, .. } = &state.popup else {
        return;
    };
    match *step {
        OpenStep::Pick => render_pick_step(frame, state, area),
        OpenStep::Configure => render_configure_step(frame, state, area),
    }
}

fn render_pick_step(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let PopupState::OpenWorktree {
        rows,
        selected,
        scroll,
        anchor_y,
        ..
    } = &state.popup
    else {
        return;
    };
    let rows = rows.clone();
    let selected = *selected;
    let scroll = *scroll;
    let anchor_y = *anchor_y;
    let theme = &state.theme;

    let popup_width = area.width.min(32).max(area.width.min(14));
    let popup_height = (rows.len() as u16).saturating_add(POPUP_BORDER_ROWS);
    let popup_rect = match anchor_y {
        Some(y) => anchor_below(area, y, popup_width, popup_height),
        None => center_popup(area, popup_width, popup_height),
    };
    state.popup.set_open_worktree_area(Some(popup_rect));

    frame.render_widget(Clear, popup_rect);
    let title = truncate_to_width(
        " Open worktree ",
        popup_rect.width.saturating_sub(2) as usize,
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(
            title,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(popup_rect);
    frame.render_widget(block, popup_rect);

    let inner_width = inner.width as usize;
    // Same clamp the click hit-test applies, via the same function: the
    // two must agree on which row sits on which screen line or a click
    // after a resize picks the wrong worktree.
    let visible = inner.height as usize;
    let start = clamp_scroll(scroll, rows.len(), visible);
    for (i, row) in rows.iter().skip(start).take(visible).enumerate() {
        let marker = if row.in_use { IN_USE_MARKER } else { " " };
        let label = truncate_to_width(&row.label, inner_width.saturating_sub(3));
        let text = format!(" {marker} {label}");
        let padding = " ".repeat(inner_width.saturating_sub(display_width(&text)));
        let style = if start + i == selected {
            Style::default()
                .fg(theme.text_active)
                .bg(theme.selection_bg)
        } else {
            Style::default().fg(theme.text_muted)
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(format!("{text}{padding}"), style))),
            Rect::new(inner.x, inner.y + i as u16, inner.width, 1),
        );
    }
}

fn render_configure_step(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let PopupState::OpenWorktree {
        rows,
        selected,
        editor,
        pick,
        field,
        anchor_y,
        error,
        ..
    } = &state.popup
    else {
        return;
    };
    let branch = rows
        .get(*selected)
        .map(|r: &OpenWorktreeRow| r.label.clone())
        .unwrap_or_default();
    let editor = editor.clone();
    let field = *field;
    let anchor_y = *anchor_y;
    let error = error.clone();
    let agent = pick.agent();
    let mode = pick.mode();
    let theme = &state.theme;

    let popup_width = area.width.min(32).max(area.width.min(14));
    let compact = area.height < SPAWN_MODAL_EXPANDED_MIN_HEIGHT;
    let content_rows: u16 = if compact {
        COMPACT_CONTENT_ROWS
    } else {
        EXPANDED_CONTENT_ROWS
    };
    let error_rows: u16 = if error.is_some() { POPUP_ERROR_ROWS } else { 0 };
    let popup_height = content_rows + error_rows + POPUP_BORDER_ROWS;
    let popup_rect = match anchor_y {
        Some(y) => anchor_below(area, y, popup_width, popup_height),
        None => center_popup(area, popup_width, popup_height),
    };
    state.popup.set_open_worktree_area(Some(popup_rect));

    frame.render_widget(Clear, popup_rect);
    let title = truncate_to_width(
        " Open worktree ",
        popup_rect.width.saturating_sub(2) as usize,
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(
            title,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(popup_rect);
    frame.render_widget(block, popup_rect);

    let render_at = |frame: &mut Frame, y_offset: u16, spans: Vec<Span<'_>>| {
        if y_offset < inner.height {
            let row = Rect::new(
                inner.x + 1,
                inner.y + y_offset,
                inner.width.saturating_sub(2),
                1,
            );
            frame.render_widget(Paragraph::new(Line::from(spans)), row);
        }
    };

    let label_style = |target: OpenField| {
        let base = Style::default().add_modifier(Modifier::BOLD);
        if field == target {
            base.fg(theme.accent)
        } else {
            base.fg(theme.text_muted)
        }
    };
    let value_style = |target: OpenField| {
        if field == target {
            Style::default().fg(theme.text_active)
        } else {
            Style::default().fg(theme.text_muted)
        }
    };

    let content_width = inner.width.saturating_sub(2) as usize;
    // BRANCH is read-only — it is what the user already picked in step
    // 1, shown for confirmation — so it never takes the focus style.
    let branch_value = truncate_to_width(&branch, content_width);
    let agent_value = truncate_to_width(agent, content_width);
    let mode_value = truncate_to_width(mode, content_width);
    let muted = Style::default().fg(theme.text_muted);
    // Same tail-fit + block cursor the spawn modal's text fields use.
    let editor_spans = {
        let mut spans = vec![Span::styled(
            tail_fit(&editor, content_width.saturating_sub(1)),
            value_style(OpenField::Editor),
        )];
        if field == OpenField::Editor {
            spans.push(Span::styled("█", Style::default().fg(theme.accent)));
        }
        spans
    };
    let error_spans = error.as_ref().map(|err| {
        vec![Span::styled(
            truncate_to_width(err, content_width),
            Style::default().fg(theme.status_error),
        )]
    });

    if compact {
        render_at(
            frame,
            COMPACT_TASK_Y,
            vec![Span::styled(branch_value, muted)],
        );
        render_at(frame, COMPACT_EDITOR_Y, editor_spans);
        render_at(
            frame,
            COMPACT_AGENT_Y,
            vec![Span::styled(agent_value, value_style(OpenField::Agent))],
        );
        render_at(
            frame,
            COMPACT_MODE_Y,
            vec![Span::styled(mode_value, value_style(OpenField::Mode))],
        );
        if let Some(err) = error_spans {
            render_at(frame, COMPACT_ERROR_Y, err);
        }
    } else {
        render_at(
            frame,
            EXP_TASK_LABEL_Y,
            vec![Span::styled("BRANCH", muted.add_modifier(Modifier::BOLD))],
        );
        render_at(
            frame,
            EXP_TASK_VALUE_Y,
            vec![Span::styled(branch_value, muted)],
        );
        render_at(
            frame,
            EXP_EDITOR_LABEL_Y,
            vec![Span::styled("EDITOR", label_style(OpenField::Editor))],
        );
        render_at(frame, EXP_EDITOR_VALUE_Y, editor_spans);
        render_at(
            frame,
            EXP_AGENT_LABEL_Y,
            vec![Span::styled("AGENT", label_style(OpenField::Agent))],
        );
        render_at(
            frame,
            EXP_AGENT_VALUE_Y,
            vec![Span::styled(agent_value, value_style(OpenField::Agent))],
        );
        render_at(
            frame,
            EXP_MODE_LABEL_Y,
            vec![Span::styled("MODE", label_style(OpenField::Mode))],
        );
        render_at(
            frame,
            EXP_MODE_VALUE_Y,
            vec![Span::styled(mode_value, value_style(OpenField::Mode))],
        );
        if let Some(err) = error_spans {
            render_at(frame, EXP_ERROR_Y, err);
        }
    }
}
