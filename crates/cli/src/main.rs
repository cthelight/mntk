//! mntk: Media Naming Toolkit.
//!
//! Composition root: parses arguments, then dispatches to the command.

mod cli;
mod movie;
mod tui;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Command};
use crate::movie::Interrupted;

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        // Ctrl+C gets the conventional shell exit code (128 + SIGINT).
        Err(error) if error.is::<Interrupted>() => ExitCode::from(130),
        Err(error) => {
            eprintln!("mntk: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Movie(opts) => movie::run(opts, cli.global),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movie_missing_file_is_an_error() {
        let cli = Cli::parse_from(["mntk", "movie", "/nonexistent/nope.mkv"]);
        let error = run(cli).expect_err("missing file must fail");
        assert!(error.to_string().contains("file not found"), "{error}");
    }

    #[test]
    fn interrupted_maps_to_exit_code_130() {
        let error = anyhow::Error::new(Interrupted);
        assert!(error.is::<Interrupted>());
    }
}
