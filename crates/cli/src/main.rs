//! mntk: Media Naming Toolkit.
//!
//! Composition root: parses arguments, resolves configuration, builds the
//! requested source, and dispatches to the command.

mod cli;

use std::process::ExitCode;

use anyhow::bail;
use clap::Parser;

use crate::cli::{Cli, Command};

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mntk: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Movie(_) => bail!("`mntk movie` is not implemented yet"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movie_command_is_not_implemented_yet() {
        let cli = Cli::parse_from(["mntk", "movie", "movie.mkv"]);
        match run(cli) {
            Err(error) => assert!(error.to_string().contains("not implemented")),
            Ok(()) => panic!("expected an error"),
        }
    }
}
