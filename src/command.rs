use std::path::PathBuf;

use clap::parser::ValueSource;

use clap::{Arg, ArgAction, ArgMatches, Command};

use crate::config::{Config, Format, HiddenMode};

pub mod options {
    pub mod format {
        pub const ONE_LINE: &str = "1";
    }

    pub mod hidden_mode {
        pub const ALL: &str = "all";
        pub const ALMOST_ALL: &str = "almost-all";
        pub const UNSORTED_ALL: &str = "f";
    }

    pub const DIRECTORY: &str = "directory";
    pub const REVERSE: &str = "reverse";
    pub const RECURSIVE: &str = "recursive";
    pub const PATH: &str = "path";
}

pub fn get_matches() -> ArgMatches {
    Command::new("ls")
        .arg(
            Arg::new(options::format::ONE_LINE)
                .short('1')
                .help("list one file per line")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(options::hidden_mode::ALL)
                .short('a')
                .long(options::hidden_mode::ALL)
                .overrides_with_all([options::hidden_mode::ALL, options::hidden_mode::ALMOST_ALL])
                .help("do not ignore entries starting with .")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(options::hidden_mode::ALMOST_ALL)
                .short('A')
                .long(options::hidden_mode::ALMOST_ALL)
                .overrides_with_all([options::hidden_mode::ALL, options::hidden_mode::ALMOST_ALL])
                .help("do not list implied . and ..")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(options::hidden_mode::UNSORTED_ALL)
                .short('f')
                .overrides_with_all([options::hidden_mode::ALL, options::hidden_mode::ALMOST_ALL])
                .help("same as -a -U")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(options::REVERSE)
                .short('r')
                .long(options::REVERSE)
                .help("reverse order while sorting")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(options::DIRECTORY)
                .short('d')
                .long(options::DIRECTORY)
                .help("list directories themselves, not their contents")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(options::RECURSIVE)
                .short('R')
                .long(options::RECURSIVE)
                .help("list subdirectories recursively")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(options::PATH)
                .action(ArgAction::Append)
                .default_value("."),
        )
        .get_matches()
}

fn extract_format(options: &ArgMatches) -> Format {
    let get_idx = |flag: &str| {
        if options.value_source(flag) == Some(ValueSource::CommandLine) {
            options.index_of(flag).unwrap_or(0)
        } else {
            0
        }
    };

    let one_line_idx = get_idx(options::format::ONE_LINE);

    let max_idx = one_line_idx;

    if max_idx == 0 {
        Format::Default
    } else {
        Format::OneLine
    }
}

fn extract_hidden_mode(options: &ArgMatches) -> HiddenMode {
    let get_idx = |flag: &str| {
        if options.value_source(flag) == Some(ValueSource::CommandLine) {
            options.index_of(flag).unwrap_or(0)
        } else {
            0
        }
    };

    let all_idx = get_idx(options::hidden_mode::ALL);
    let almost_all_idx = get_idx(options::hidden_mode::ALMOST_ALL);
    let unsorted_all_idx = get_idx(options::hidden_mode::UNSORTED_ALL);

    let max_idx = all_idx.max(almost_all_idx).max(unsorted_all_idx);

    if max_idx == 0 {
        HiddenMode::Default
    } else if max_idx == almost_all_idx {
        HiddenMode::AlmostAll
    } else {
        HiddenMode::All
    }
}

fn extract_paths(options: &ArgMatches) -> Vec<PathBuf> {
    options
        .get_many::<String>(options::PATH)
        .unwrap_or_default()
        .map(PathBuf::from)
        .collect()
}

impl From<&ArgMatches> for Config {
    fn from(options: &ArgMatches) -> Self {
        let format = extract_format(options);
        let hidden_mode = extract_hidden_mode(options);
        let paths = extract_paths(options);

        Self {
            format,
            hidden_mode,
            reverse: options.get_flag(options::REVERSE),
            list_dir: options.get_flag(options::DIRECTORY),
            recursive: options.get_flag(options::RECURSIVE),
            paths,
        }
    }
}
