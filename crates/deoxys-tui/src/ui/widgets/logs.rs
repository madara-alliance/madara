use ratatui::layout::{Margin, Rect};
use ratatui::prelude::Frame;
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::utils::render_zone;
use crate::app::App;

pub fn render_l1_logs(frame: &mut Frame, app: &App, area: Rect) {
    render_zone(frame, area, "L1 Logs");
    let raw: Vec<String> = app.data.l1_logs.clone().into_iter().flatten().collect();
    if !raw.is_empty() {
        let logs: Vec<Line<'_>> = raw.into_iter().map(Span::from).map(Line::from).collect();
        let ll = logs.len();
        if ll > area.height as usize {
            let l: Vec<Line> = logs.into_iter().skip(ll - area.height as usize).collect();
            frame.render_widget(Paragraph::new(l), area.inner(&Margin::new(1, 1)));
        } else {
            frame.render_widget(Paragraph::new(logs), area.inner(&Margin::new(1, 1)));
        }
    }
}

pub fn render_l2_logs(frame: &mut Frame, app: &App, area: Rect) {
    render_zone(frame, area, "L2 Logs");
    let raw: Vec<String> = app.data.l2_logs.clone().into_iter().flatten().collect();
    if !raw.is_empty() {
        let logs: Vec<Line<'_>> = raw.into_iter().map(Span::from).map(Line::from).collect();
        let ll = logs.len();
        if ll > area.height as usize {
            let l: Vec<Line> = logs.into_iter().skip(ll - area.height as usize).collect();
            frame.render_widget(Paragraph::new(l), area.inner(&Margin::new(1, 1)));
        } else {
            frame.render_widget(Paragraph::new(logs), area.inner(&Margin::new(1, 1)));
        }
    }
}

fn _render_nolog_error(frame: &mut Frame, area: Rect, message: &str) {
    let bad_popup = Paragraph::new(message).wrap(Wrap { trim: true }).style(Style::new().yellow()).block(
        Block::new()
            .title("No logs received")
            .title_style(Style::new().white().bold())
            .borders(Borders::ALL)
            .border_style(Style::new().red()),
    );
    frame.render_widget(
        bad_popup.style(Style::new().yellow().add_modifier(Modifier::RAPID_BLINK)),
        area,
    );
}
