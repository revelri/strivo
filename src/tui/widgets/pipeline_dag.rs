//! ASCII DAG overlay for the active Pipeline(s). (X6.)
//!
//! Pure render — reads the [`PipelineRegistry`] and lays each pipeline
//! out as a left-to-right ASCII flow with per-node state glyphs. No
//! interaction in this MVP; future commits add cancel + retry verbs.

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Wrap},
    Frame,
};

use crate::pipeline::{Pipeline, PipelineRegistry, PipelineState, StageState};
use crate::tui::theme::Theme;

/// Render the DAG overlay over `area`. Caller centers the rect.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    registry: &PipelineRegistry,
    enter_progress: f32,
) {
    let h = area.height.saturating_mul(7) / 10;
    let h = h.min(28).max(12);
    let w = area.width.saturating_mul(7) / 10;
    let w = w.min(90).max(56);

    let [_, row, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(h),
        Constraint::Fill(1),
    ])
    .areas(area);
    let [_, center, _] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(w),
        Constraint::Fill(1),
    ])
    .areas(row);

    frame.render_widget(Clear, center);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border_ramp(enter_progress))
        .padding(Padding::horizontal(1))
        .title(" Pipelines ")
        .title_style(Theme::title());
    let inner = block.inner(center);
    frame.render_widget(block, center);

    if registry.is_empty() {
        let p = Paragraph::new("No pipelines submitted this session.")
            .style(Style::new().fg(Theme::muted()));
        frame.render_widget(p, inner);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    for pipe in registry.iter() {
        lines.extend(render_pipeline(pipe));
        lines.push(Line::raw(""));
    }

    let scroll = if lines.len() > inner.height as usize {
        lines.len().saturating_sub(inner.height as usize)
    } else {
        0
    };
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll as u16, 0)),
        inner,
    );
}

fn render_pipeline(pipe: &Pipeline) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();

    let state_chip = state_chip_text(&pipe.state);
    out.push(Line::from(vec![
        Span::styled(
            format!(" {state_chip} "),
            state_chip_style(&pipe.state),
        ),
        Span::raw("  "),
        Span::styled(
            pipe.name.clone(),
            Style::new().fg(Theme::fg()).add_modifier(Modifier::BOLD),
        ),
    ]));

    let cost = pipe.total_cost_cents();
    if cost > 0 {
        out.push(Line::from(Span::styled(
            format!("    est. {}", crate::edl::schema::EDL_VERSION).replace(
                &format!("{}", crate::edl::schema::EDL_VERSION),
                &format_cents(cost),
            ),
            Style::new().fg(Theme::muted()),
        )));
    }

    // Topological flow: render stages left-to-right by layering. A
    // simple BFS suffices for the MVP — the DAG widths we expect are
    // tiny (≤ 8 stages) so we don't worry about elaborate layouts.
    let mut layers: Vec<Vec<&crate::pipeline::Stage>> = Vec::new();
    let mut placed: std::collections::HashSet<crate::pipeline::StageId> =
        std::collections::HashSet::new();
    while placed.len() < pipe.stages.len() {
        let mut layer: Vec<&crate::pipeline::Stage> = Vec::new();
        for s in &pipe.stages {
            if placed.contains(&s.id) {
                continue;
            }
            if s.inputs.iter().all(|i| placed.contains(i)) {
                layer.push(s);
            }
        }
        if layer.is_empty() {
            // Shouldn't happen post-assert_acyclic, but guard against
            // walking forever.
            break;
        }
        for s in &layer {
            placed.insert(s.id);
        }
        layers.push(layer);
    }

    for (li, layer) in layers.iter().enumerate() {
        let mut spans: Vec<Span<'static>> = Vec::new();
        spans.push(Span::raw("    "));
        if li > 0 {
            spans.push(Span::styled(
                "─▶ ".to_string(),
                Style::new().fg(Theme::dim()),
            ));
        }
        for (si, s) in layer.iter().enumerate() {
            if si > 0 {
                spans.push(Span::styled(
                    " · ".to_string(),
                    Style::new().fg(Theme::dim()),
                ));
            }
            spans.push(Span::styled(
                stage_glyph(&s.state).to_string(),
                stage_glyph_style(&s.state),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                s.kind.label(),
                Style::new().fg(Theme::fg()),
            ));
            if s.attempts > 0 {
                spans.push(Span::styled(
                    format!(" ×{}", s.attempts + 1),
                    Style::new().fg(Theme::muted()),
                ));
            }
        }
        out.push(Line::from(spans));
    }

    out
}

fn state_chip_text(state: &PipelineState) -> &'static str {
    match state {
        PipelineState::Pending => "pending",
        PipelineState::Running => "running",
        PipelineState::Done => "done",
        PipelineState::Failed => "failed",
        PipelineState::Cancelled => "cancelled",
    }
}

fn state_chip_style(state: &PipelineState) -> Style {
    let bg = match state {
        PipelineState::Pending => Theme::muted(),
        PipelineState::Running => Theme::primary(),
        PipelineState::Done => Theme::green(),
        PipelineState::Failed => Theme::red(),
        PipelineState::Cancelled => Theme::dim(),
    };
    Style::new()
        .fg(Theme::bg())
        .bg(bg)
        .add_modifier(Modifier::BOLD)
}

fn stage_glyph(state: &StageState) -> &'static str {
    match state {
        StageState::Pending => "○",
        StageState::Running { .. } => "◐",
        StageState::Done => "✓",
        StageState::Failed { .. } => "✗",
        StageState::Exhausted { .. } => "✗",
        StageState::Cancelled => "□",
        StageState::Skipped => "·",
    }
}

fn stage_glyph_style(state: &StageState) -> Style {
    let fg = match state {
        StageState::Pending => Theme::muted(),
        StageState::Running { .. } => Theme::primary(),
        StageState::Done => Theme::green(),
        StageState::Failed { .. } => Theme::yellow(),
        StageState::Exhausted { .. } => Theme::red(),
        StageState::Cancelled => Theme::dim(),
        StageState::Skipped => Theme::dim(),
    };
    Style::new().fg(fg).add_modifier(Modifier::BOLD)
}

fn format_cents(c: u32) -> String {
    let d = c / 100;
    let r = c % 100;
    format!("${d}.{r:02}")
}
