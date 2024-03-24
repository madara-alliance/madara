use ratatui::layout::Rect;
use ratatui::prelude::Frame;
use ratatui::style::{Color, Stylize};
use ratatui::widgets::{Block, Borders, Gauge};
use splines::{Interpolation, Key, Spline};

pub fn smooth_serie(series: &[f64], window_size: usize) -> Vec<(f64, f64)> {
    let mut smoothed_series = Vec::new();

    let ignore_count = window_size / 2;

    for i in ignore_count..series.len() - ignore_count {
        let window_average: f64 = series[i - ignore_count..=i + ignore_count].iter().sum::<f64>() / window_size as f64;
        smoothed_series.push(window_average);
    }
    let x: Vec<f64> = (0..=100).map(|x| x as f64).collect();
    let serie: Vec<(f64, f64)> = x.into_iter().zip(smoothed_series).collect();
    serie
}

pub fn render_zone(frame: &mut Frame, area: Rect, title: &str) {
    let outline = Block::new().borders(Borders::ALL).title(title);
    frame.render_widget(outline, area);
}

pub fn render_gauge(frame: &mut Frame, area: Rect, ratio: f64, alert_mode: bool) {
    let color;

    if alert_mode {
        if ratio <= 1. / 3. {
            color = Color::Green;
        } else if ratio <= 2. / 3. {
            color = Color::Rgb(255, 128, 0)
        } else {
            color = Color::Red;
        }
    } else {
        color = Color::Green
    }
    let gauge =
        Gauge::default().gauge_style(color).fg(Color::Rgb(20, 20, 20)).ratio(if ratio <= 1. { ratio } else { 1. });
    frame.render_widget(gauge, area);
}

pub fn continuous(points: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    let keys: Vec<Key<f64, f64>> = points.iter().map(|&(x, y)| Key::new(x, y, Interpolation::Linear)).collect();

    let spline = Spline::from_vec(keys);

    let mut total_distance = 0.0;
    for window in points.windows(2) {
        let (x0, y0) = window[0];
        let (x1, y1) = window[1];
        total_distance += ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
    }
    let target_distance = 0.01;
    let points_nb = (total_distance / target_distance).ceil() as usize;

    let start = points.first().unwrap().0;
    let end = points.last().unwrap().0;
    let ds = (end - start) / (points_nb as f64 - 1.0);

    let mut cnt: Vec<(f64, f64)> = Vec::new();
    for i in 0..points_nb {
        let x = start + i as f64 * ds;
        if let Some(y) = spline.sample(x) {
            cnt.push((x, y));
        }
    }
    cnt
}
