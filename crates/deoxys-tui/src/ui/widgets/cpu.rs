use ratatui::layout::{Flex, Layout, Rect};
use ratatui::prelude::Frame;
use ratatui::prelude::Margin;
use ratatui::prelude::Direction;
use ratatui::prelude::Constraint;
use ratatui::style::{Color, Style, Stylize};
use ratatui::symbols::Marker;
use ratatui::widgets::{Axis, Chart, Dataset};

use super::utils::{continuous, render_gauge, smooth_serie, render_zone};
use crate::app::App;

pub fn render_cpu(frame: &mut Frame, app: &App, area: Rect) {
    let layout = Layout::default()
    .direction(Direction::Vertical)
    .constraints(vec![Constraint::Percentage(80), Constraint::Percentage(20)])
    .flex(Flex::Center)
    .margin(0)
    .split(area);
    render_zone(frame, layout[1], "Used");
    render_cpu_gauge(frame, app, layout[1].inner(&Margin::new(1, 1)));
    render_cpu_graph(frame, app, layout[0]);
}

fn render_cpu_graph(frame: &mut Frame, app: &App, area: Rect) {
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

fn render_cpu_gauge(frame: &mut Frame, app: &App, area: Rect) {
    let serie = smooth_serie(&app.data.cpu_usage, 20);
    render_gauge(frame, area, serie.last().unwrap().1 / 100., true)
}
