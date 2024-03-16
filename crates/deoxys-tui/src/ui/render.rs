use anyhow::Result;
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin};
use ratatui::prelude::Frame;
use ratatui::style::Color;
use ratatui::widgets::{Block, Borders};

use crate::app::App;
use crate::ui::widgets::cpu::*;
use crate::ui::widgets::logs::*;
use crate::ui::widgets::memory::*;
use crate::ui::widgets::network::*;
use crate::ui::widgets::storage::*;
use crate::ui::widgets::utils::render_zone;

pub fn ui(app: &App, frame: &mut Frame) {
    let outline = Block::new()
        .borders(Borders::ALL)
        .title(" Deoxys-TUI v0.1.0 (Press q to quit) ")
        .title_style(Color::Magenta)
        .title_alignment(Alignment::Center);
    frame.render_widget(outline, frame.size());

    let node0 = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(frame.size().inner(&Margin::new(2, 1)));

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Percentage(40), Constraint::Percentage(30), Constraint::Percentage(30)])
        .split(node0[0]);
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Ratio(1, 3), Constraint::Ratio(1, 3), Constraint::Ratio(1, 3)])
        .split(node0[1]);

    render_zone(frame, right[0], "CPU");
    render_cpu(frame, app, right[0].inner(&Margin::new(1, 1)));

    render_zone(frame, right[1], "Memory");
    render_memory(frame, app, right[1].inner(&Margin::new(1, 1)));

    render_zone(frame, right[2], "Storage");
    render_storage(frame, app, right[2].inner(&Margin::new(1, 1)));

    render_zone(frame, left[2], "Network");
    render_network_graph(frame, app, left[2]);
    render_l2_logs(frame, app, left[0]);
    render_l1_logs(frame, app, left[1]);
}

pub fn startup() -> Result<()> {
    enable_raw_mode()?;
    execute!(std::io::stdout(), EnterAlternateScreen)?;
    Ok(())
}

pub fn shutdown() -> Result<()> {
    execute!(std::io::stdout(), LeaveAlternateScreen)?;
    disable_raw_mode()?;
    std::env::set_var("RUST_LOG", "TRACE");
    Ok(())
}
