use anyhow::Context;
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

pub fn raise_fdlimit() {
    use fdlimit::Outcome;
    let recommended = 10000;
    match fdlimit::raise_fd_limit() {
        Ok(Outcome::LimitRaised { to, .. }) if to < recommended => {
            log::warn!(
                    "The file-descriptor limit for the current process is {to}, which is lower than the recommended {recommended}."
                );
        }
        Ok(Outcome::LimitRaised { to, .. }) => {
            log::debug!("File-descriptor limit was raised to {to}.");
        }
        Err(error) => {
            log::warn!(
                "Error while trying to raise the file-descriptor limit for the process: {error:#}. The recommended file-descriptor limit is {recommended}."
            );
        }
        Ok(Outcome::Unsupported) => {
            log::debug!("Unsupported platform for raising file-descriptor limit.");
        }
    }
}

// Todo: Setup tracing
pub fn setup_logging() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format(|fmt, record| {
            let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
            let style = fmt.default_level_style(record.level());
            let brackets = Style::new().fg_color(Some(Color::Ansi(AnsiColor::BrightBlack)));

            match record.level() {
                Level::Info if record.target() == "rpc_calls" => {
                    let status = record.key_values().get(Key::from("status")).expect("Mo status in record").to_i64().expect("Status is not an int");
                    let method = record.key_values().get(Key::from("method")).expect("Mo method in record");
                    let res_len = record.key_values().get(Key::from("res_len")).expect("No res_len in record");
                    let rpc_style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Magenta)));
                    let status_color = if status == 200 {
                        Style::new().fg_color(Some(Color::Ansi(AnsiColor::Green)))
                    } else {
                        Style::new().fg_color(Some(Color::Ansi(AnsiColor::Red)))
                    };
                    let response_time = Duration::from_micros(record.key_values().get(Key::from("response_time")).expect("No response time in record").to_u64().expect("Response time is not an int"));
                    let time_color = match response_time {
                        time if time <= Duration::from_millis(5) => {
                            Style::new()
                        },
                        // time if time <= Duration::from_millis(10) => {
                        _ => {
                            Style::new().fg_color(Some(Color::Ansi(AnsiColor::Yellow)))
                        },
                        // _ => {
                        //     Style::new().fg_color(Some(Color::Ansi(AnsiColor::Red)))
                        // }
                    };

                    writeln!(
                        fmt,
                        "{brackets}[{brackets:#}{ts} {rpc_style}HTTP{rpc_style:#}{brackets}]{brackets:#} 🌐 {method} {status_color}{status}{status_color:#} {res_len} bytes - {time_color}{response_time:?}{time_color:#}",
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
                        "{brackets}[{brackets:#}{} {style}{}{style:#} {}{brackets}]{brackets:#} ⚠️  {}",
                        ts,
                        record.level(),
                        record.target(),
                        record.args()
                    )
                }
                Level::Error if record.target() == "rpc_errors" => {
                    writeln!(
                        fmt,
                        "{brackets}[{brackets:#}{} {style}{}{style:#}{brackets}]{brackets:#} ❗ RPC Internal Server Error: {}",
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

/// Returns a random Pokémon name.
pub async fn get_random_pokemon_name() -> anyhow::Result<String> {
    use rand::{seq::SliceRandom, thread_rng};
    let res = reqwest::get("https://pokeapi.co/api/v2/pokemon/?limit=1000").await?;
    let body = res.text().await?;
    let json: serde_json::Value = serde_json::from_str(&body)?;

    let pokemon_array = json["results"].as_array().context("Getting result from returned json")?;
    let mut rng = thread_rng();
    let random_pokemon = pokemon_array.choose(&mut rng).context("Choosing a name")?;

    Ok(random_pokemon["name"].as_str().context("Getting name from pokemon object")?.to_string())
}
