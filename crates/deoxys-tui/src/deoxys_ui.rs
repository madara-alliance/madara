use std::time::Duration;

use anyhow::{Ok, Result};
use crossterm::event::Event::Key;
use crossterm::event::KeyCode::Char;
use crossterm::event::{self};
use ratatui::prelude::{CrosstermBackend, Terminal};
use tokio::sync::mpsc;

use crate::app::App;
use crate::ui::render;

pub async fn run(storage_path: &str, logs_rx: mpsc::Receiver<String>) -> Result<()> {
    let mut t = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
    let mut app = App::new(storage_path, logs_rx).unwrap();

    render::startup()?;
    loop {
        update(&mut app).await?;
        t.draw(|f| {
            render::ui(&app, f);
        })?;
        if app.should_quit {
            break;
        }
    }
    render::shutdown()?;
    Ok(())
}

#[allow(clippy::single_match)]
async fn update(app: &mut App) -> Result<()> {
    app.update_metrics().await;
    if event::poll(Duration::from_millis(50))? {
        if let Key(key) = event::read()? {
            if key.kind == event::KeyEventKind::Press {
                match key.code {
                    Char('q') => app.should_quit = true,
                    _ => {}
                }
            }
        }
    }
    Ok(())
}
