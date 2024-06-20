use chrono::Local;
use clap::builder::styling::{AnsiColor, Color, Style};
use log::{kv::Key, Level};
use std::{io::Write, time::Duration};

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
        .format(|fmt, record| {
            let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
            let style = fmt.default_level_style(record.level());
            let brackets = Style::new().fg_color(Some(Color::Ansi(AnsiColor::BrightBlack)));

            match record.level() {
                Level::Info if record.target() == "rpc_calls" => {
                    let status = record.key_values().get(Key::from("status")).unwrap().to_i64().unwrap();
                    let method = record.key_values().get(Key::from("method")).unwrap();
                    let res_len = record.key_values().get(Key::from("res_len")).unwrap();
                    let rpc_style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Magenta)));
                    let status_color = if status == 200 {
                        Style::new().fg_color(Some(Color::Ansi(AnsiColor::Green)))
                    } else {
                        Style::new().fg_color(Some(Color::Ansi(AnsiColor::Red)))
                    };
                    let response_time = Duration::from_micros(record.key_values().get(Key::from("response_time")).unwrap().to_u64().unwrap());
                    let time_color = match response_time {
                        time if time <= Duration::from_millis(5) => {
                            Style::new()
                        },
                        time if time <= Duration::from_millis(10) => {
                            Style::new().fg_color(Some(Color::Ansi(AnsiColor::Yellow)))
                        },
                        _ => {
                            Style::new().fg_color(Some(Color::Ansi(AnsiColor::Red)))
                        }
                    };

                    writeln!(
                        fmt,
                        "{brackets}[{brackets:#}{ts} {rpc_style}HTTP{rpc_style:#}{brackets}]{brackets:#} {method} {status_color}{status}{status_color:#} {res_len} bytes - {time_color}{response_time:?}{time_color:#}",
                    )
                }
                Level::Info => {
                    writeln!(
                        fmt,
                        "{brackets}[{brackets:#}{} {style}{}{style:#}{brackets}]{brackets:#} {}",
                        ts,
                        record.level(),
                        record.args()
                    )
                }
                Level::Warn => {
                    writeln!(
                        fmt,
                        "{brackets}[{brackets:#}{} {style}{}{style:#}{brackets}]{brackets:#} ⚠️ {}",
                        ts,
                        record.level(),
                        record.args()
                    )
                }
                Level::Error if record.target() == "rpc_errors" => {
                    writeln!(
                        fmt,
                        "{brackets}[{brackets:#}{} {style}{}{style:#}{brackets}]{brackets:#} ❗ {}",
                        ts,
                        record.level(),
                        record.args()
                    )
                }
                Level::Error => {
                    writeln!(
                        fmt,
                        "{brackets}[{brackets:#}{} {style}{}{style:#} {}{brackets}]{brackets:#} ❗ {}",
                        ts,
                        record.level(),
                        record.target(),
                        record.args()
                    )
                }
                _ => {
                    writeln!(
                        fmt,
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
