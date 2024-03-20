use std::io;
use std::marker::Send;
use std::result::Result::Ok;

use tokio::sync::mpsc;
use tracing::subscriber::set_global_default;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;

use std::path::Path;

struct MpscWriter {
    tx: mpsc::Sender<String>,
}

impl io::Write for MpscWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let msg = String::from_utf8_lossy(buf);
        if let Err(_e) = self.tx.try_send(msg.to_string()) {
            return Err(io::Error::new(io::ErrorKind::Other, "Failed to send message"));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn set_subscriber(tx: mpsc::Sender<String>) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let subscriber = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(move || Box::new(MpscWriter { tx: tx.clone() }) as Box<dyn io::Write + Send>));
    set_global_default(subscriber).expect("Failed to set subscriber");
    std::env::set_var("SUBSCRIBER_DEFINED", "TRUE");
    std::env::set_var("RUST_LOG", "INFO");
}


pub fn modify_substrate_sources() {
    let crate_dir = env!("CARGO_MANIFEST_DIR");
    let flag_file_path = format!("{}/substrate/ok.flag", crate_dir); // Construisez le chemin complet du fichier flag

    if !Path::new(&flag_file_path).exists() {
        std::process::Command::new("bash")
        .arg(format!("{}/substrate/modifier.sh", crate_dir))
        .current_dir(crate_dir)
        .output()
        .expect("Failed to setup TUI");
        println!("TUI configuration finished, you can now re-build Deoxys");
        std::process::exit(0);
    }
}