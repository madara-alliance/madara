//! Deoxys node command line.
#![warn(missing_docs)]

#[macro_use]
mod service;
mod benchmarking;
mod chain_spec;
mod cli;
mod command;
mod commands;
mod configs;
mod genesis_block;
mod rpc;
mod util;

fn main() -> anyhow::Result<()> {
    command::run()
}
