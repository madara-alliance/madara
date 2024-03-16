use humansize::{format_size, BINARY};
use ratatui::layout::{Constraint, Direction, Layout, Margin, Rect};
use ratatui::prelude::Frame;
use ratatui::style::Color;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use super::utils::{render_gauge, render_zone};
use crate::app::App;

pub fn render_storage_data(frame: &mut Frame, app: &App, area: Rect) {
    let data = vec![
        Line::raw(format!("Total Disk Space: {}", format_size(app.data.disk_size, BINARY))).style(Color::Green),
        Line::raw(format!("Node Disk Usage: {}", format_size(app.data.disk_usage, BINARY))).style(Color::Green),
        Line::raw(format!("Available Space: {}", format_size(app.data.available_storage, BINARY))).style(Color::Green),
    ];
    frame.render_widget(Paragraph::new(data), area);
}

pub fn render_storage_gauge(frame: &mut Frame, app: &App, area: Rect) {
    let ratio = if app.data.available_storage == 0 {
        1.
    } else {
        app.data.disk_size as f64 / app.data.available_storage as f64
    };
    render_zone(frame, area, "Used");
    render_gauge(frame, area.inner(&Margin::new(1, 1)), ratio, true);
}

pub fn render_storage(frame: &mut Frame, app: &App, area: Rect) {
    let zones = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Ratio(3, 4), Constraint::Ratio(1, 4)])
        .split(area);
    render_storage_gauge(frame, app, zones[1]);
    render_storage_data(frame, app, zones[0]);
}
