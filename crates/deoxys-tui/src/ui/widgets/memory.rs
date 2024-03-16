use ratatui::layout::Rect;
use ratatui::prelude::Frame;
use ratatui::style::{Color, Style, Stylize};
use ratatui::symbols::Marker;
use ratatui::widgets::{Axis, Chart, Dataset};

use super::utils::{continuous, render_gauge, smooth_serie};
use crate::app::App;

pub fn render_memory_gauge(frame: &mut Frame, app: &App, area: Rect) {
    render_gauge(frame, area, *app.data.memory_usage.last().unwrap() as f64 / app.data.total_memory as f64, true);
}

pub fn render_memory_graph(frame: &mut Frame, app: &App, area: Rect) {
    let fserie: Vec<f64> = app.data.memory_usage.clone().into_iter().map(|elm| elm as f64 / 1000000.).collect();
    let serie = continuous(smooth_serie(&fserie, 5));
    let datasets = vec![
        Dataset::default().name("RAM").marker(Marker::Braille).style(Style::default().fg(Color::Magenta)).data(&serie),
    ];
    let chart = Chart::new(datasets)
        .x_axis(Axis::default().title("t").style(Style::default().fg(Color::Gray)).labels(vec![]).bounds([0., 100.]))
        .y_axis(
            Axis::default()
                .style(Style::default().fg(Color::Gray))
                .labels(vec!["0%".bold(), "50%".bold(), "100%".bold()])
                .bounds([0., app.data.total_memory as f64 / 1000000.]),
        );
    frame.render_widget(chart, area);
}
