use std::io::stdout;

use crate::{command::get_matches, config::Config, list::Lister};

mod command;
mod config;
mod list;

fn main() {
    let matches = get_matches();
    let config = Config::from(&matches);
    let mut lister = Lister::new(&config, stdout().lock());

    if lister.list().is_err() {
        std::process::exit(1);
    }
}
