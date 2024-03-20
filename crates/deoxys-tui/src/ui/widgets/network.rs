use ratatui::layout::{Margin, Rect};
use ratatui::prelude::Frame;
use ratatui::style::{Color, Style, Stylize};
use ratatui::symbols::Marker;
use ratatui::widgets::{Axis, Chart, Dataset};

use super::utils::{continuous, smooth_serie};
use crate::app::App;

pub fn render_network_graph(frame: &mut Frame, app: &App, area: Rect) {
    let rx_serie = continuous(smooth_serie(&app.data.rx_flow, 5));
    let tx_serie = continuous(smooth_serie(&app.data.tx_flow, 5));
    let rx_max = rx_serie.iter().map(|(_, elm)| elm).max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap();
    let tx_max = tx_serie.iter().map(|(_, elm)| elm).max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap();
    let ymax = std::cmp::max_by(rx_max, tx_max, |a, b| a.partial_cmp(b).unwrap());
    let y_labels = (0..3)
        .map(|i| 0. + ymax * i as f64 / 2.)
        .map(|elm| (((elm * 100.).round() / 100.).to_string() + " Mb/s").bold())
        .collect();
    let rx_dataset = Dataset::default()
        .name("Receiving")
        .marker(Marker::Braille)
        .style(Style::default().fg(Color::Green))
        .data(&rx_serie);
    let tx_dataset = Dataset::default()
        .name("Sending")
        .marker(Marker::Braille)
        .style(Style::default().fg(Color::LightRed))
        .data(&tx_serie);
    let chart = Chart::new(vec![tx_dataset, rx_dataset])
        .x_axis(Axis::default().title("t").style(Style::default().fg(Color::Gray)).labels(vec![]).bounds([0., 100.]))
        .y_axis(
            Axis::default()
                .style(Style::default().fg(Color::Gray))
                .labels(y_labels)
                .bounds([0., if *ymax != 0. { *ymax } else { 0.1 }]),
        );
    frame.render_widget(chart, area.inner(&Margin::new(1, 1)));
}
