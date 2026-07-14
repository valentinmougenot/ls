use std::{
    io::{BufWriter, stdout},
    process::ExitCode,
};

use crate::{command::get_matches, config::Config, list::Lister};

mod command;
mod config;
mod list;
mod output;

fn main() -> ExitCode {
    let matches = get_matches();
    let config = Config::from(&matches);
    let mut lister = Lister::new(&config, BufWriter::new(stdout().lock()));

    if lister.list().is_err() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
