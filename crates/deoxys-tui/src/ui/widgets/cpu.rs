use ratatui::layout::Rect;
use ratatui::prelude::Frame;
use ratatui::style::{Color, Style, Stylize};
use ratatui::symbols::Marker;
use ratatui::widgets::{Axis, Chart, Dataset};

use super::utils::{continuous, render_gauge, smooth_serie};
use crate::app::App;

pub fn render_cpu_graph(frame: &mut Frame, app: &App, area: Rect) {
    let serie = continuous(smooth_serie(&app.data.cpu_usage, 5));
    let datasets = vec![
        Dataset::default().name("CPU").marker(Marker::Braille).style(Style::default().fg(Color::Cyan)).data(&serie),
    ];
    let chart = Chart::new(datasets)
        .x_axis(Axis::default().title("t").style(Style::default().fg(Color::Gray)).labels(vec![]).bounds([0., 100.]))
        .y_axis(
            Axis::default()
                .style(Style::default().fg(Color::Gray))
                .labels(vec!["0%".bold(), "50%".bold(), "100%".bold()])
                .bounds([0., 101.]),
        );
    frame.render_widget(chart, area);
}

pub fn render_cpu_gauge(frame: &mut Frame, app: &App, area: Rect) {
    let serie = smooth_serie(&app.data.cpu_usage, 20);
    render_gauge(frame, area, serie.last().unwrap().1 / 100., true)
}
