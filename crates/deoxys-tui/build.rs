use std::process::Command;

fn main() -> std::io::Result<()> {
    let source = "substrate/mod.rs";
    let destination =
        "~/.cargo/git/checkouts/polkadot-sdk-3672e12173abcafd/a41673f/substrate/client/tracing/src/logging/";

    Command::new("cp").arg(source).arg(destination).status()?;

    Ok(())
}
