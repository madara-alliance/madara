use chrono::Local;
use clap::builder::styling::{AnsiColor, Color, Style};
use log::Level;
use std::io::Write;

pub fn setup_rayon_threadpool() -> anyhow::Result<()> {
    let available_parallelism = std::thread::available_parallelism()?;
    rayon::ThreadPoolBuilder::new()
        .thread_name(|thread_index| format!("rayon-{}", thread_index))
        .num_threads(available_parallelism.get())
        .build_global()?;
    Ok(())
}

pub fn setup_logging() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format(|buf, record| {
            let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
            let style = buf.default_level_style(record.level());
            let brackets = Style::new().fg_color(Some(Color::Ansi(AnsiColor::BrightBlack)));
            match record.level() {
                Level::Info => {
                    writeln!(
                        buf,
                        "{brackets}[{brackets:#}{} {style}{}{style:#}{brackets}]{brackets:#} {}",
                        ts,
                        record.level(),
                        record.args()
                    )
                }
                _ => {
                    writeln!(
                        buf,
                        "{brackets}[{brackets:#}{} {style}{}{style:#} {}{brackets}]{brackets:#} {}",
                        ts,
                        record.level(),
                        record.target(),
                        record.args()
                    )
                }
            }
        })
        .init();

    Ok(())
}
