//! The command-line interface, as parsed by clap.
//!
//! Keeping the arg model in its own module lets tests exercise argument
//! parsing without running any command.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Media Naming Toolkit: name local media files after their metadata.
#[derive(Debug, Parser)]
#[command(name = "mntk", version, about, arg_required_else_help = true)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalOpts,

    #[command(subcommand)]
    pub command: Command,
}

/// Metadata sources.
///
/// Only OMDB ships in v1; the enum exists so adding TVDB/TMDB later is a
/// new variant plus a new source crate, not a breaking CLI change.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum Source {
    /// The OMDB API (http://www.omdbapi.com).
    #[default]
    Omdb,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Omdb => "omdb",
        }
    }
}

/// Options accepted at every level (global or per-subcommand).
#[derive(Debug, Clone, Args)]
pub struct GlobalOpts {
    /// Metadata source to use.
    #[arg(long, value_enum, default_value_t, global = true)]
    pub source: Source,

    /// Path to a config file (default: ~/.config/mntk/config.yaml).
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// API key for the metadata source.
    ///
    /// Overrides OMDB_API_KEY, OMDB_ID, and the config file.
    #[arg(long, global = true)]
    pub api_key: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Search for a movie and rename the file into "<Title (Year)>/".
    Movie(MovieOpts),
}

/// Options for `mntk movie`.
#[derive(Debug, Args)]
pub struct MovieOpts {
    /// The media file to rename.
    pub file: PathBuf,

    /// Search term. In a terminal this prefills the input box (still
    /// editable); outside a terminal it is used directly.
    #[arg(long, conflicts_with = "guess")]
    pub search: Option<String>,

    /// Guess the search term from the file name.
    ///
    /// Bracketed release tags are dropped and the tokens stop after the
    /// first year-like one, so `The.Matrix.1999.1080p.mkv` becomes
    /// `The Matrix 1999`. In a terminal this prefills the input box (still
    /// editable); outside a terminal the guess is used directly.
    #[arg(long)]
    pub guess: bool,

    /// Pick result N (1-based) without prompting.
    #[arg(long)]
    pub select: Option<u32>,

    /// Print the rename plan instead of moving the file.
    #[arg(long)]
    pub dry_run: bool,

    /// Overwrite an existing file at the destination.
    #[arg(long)]
    pub force: bool,

    /// Do not embed the "[imdbid-ttXXXXXXX]" tag in the file name.
    #[arg(long)]
    pub no_imdb_id: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movie_cli(args: &[&str]) -> (GlobalOpts, MovieOpts) {
        let cli = Cli::parse_from(std::iter::once("mntk").chain(args.iter().copied()));
        let Command::Movie(opts) = cli.command;
        (cli.global, opts)
    }

    #[test]
    fn movie_defaults() {
        let (global, opts) = movie_cli(&["movie", "the matrix (1999) [imdbid-tt0133093].mkv"]);

        assert_eq!(
            opts.file,
            PathBuf::from("the matrix (1999) [imdbid-tt0133093].mkv")
        );
        assert!(opts.search.is_none());
        assert!(opts.select.is_none());
        assert!(!opts.guess);
        assert!(!opts.dry_run);
        assert!(!opts.force);
        assert!(!opts.no_imdb_id);
        assert_eq!(global.source, Source::Omdb);
        assert!(global.config.is_none());
        assert!(global.api_key.is_none());
    }

    #[test]
    fn movie_guess_flag() {
        let (_global, opts) = movie_cli(&["movie", "The.Matrix.1999.1080p.mkv", "--guess"]);

        assert!(opts.guess);
        assert!(opts.search.is_none());
    }

    #[test]
    fn movie_search_and_guess_conflict() {
        let result = Cli::try_parse_from([
            "mntk",
            "movie",
            "m.mkv",
            "--search",
            "the matrix",
            "--guess",
        ]);
        assert!(
            result.is_err(),
            "--search and --guess must be mutually exclusive"
        );
    }

    #[test]
    fn movie_flags() {
        let (_global, opts) = movie_cli(&[
            "movie",
            "movie.mkv",
            "--search",
            "the matrix",
            "--select",
            "2",
            "--dry-run",
            "--force",
            "--no-imdb-id",
        ]);

        assert_eq!(opts.search.as_deref(), Some("the matrix"));
        assert_eq!(opts.select, Some(2));
        assert!(opts.dry_run);
        assert!(opts.force);
        assert!(opts.no_imdb_id);
    }

    #[test]
    fn global_flags_work_before_and_after_the_subcommand() {
        let (global, _) = movie_cli(&[
            "--api-key",
            "from-before",
            "movie",
            "movie.mkv",
            "--source",
            "omdb",
        ]);

        assert_eq!(global.api_key.as_deref(), Some("from-before"));
        assert_eq!(global.source, Source::Omdb);
    }

    #[test]
    fn unknown_source_is_rejected() {
        let result = Cli::try_parse_from(["mntk", "--source", "tmdb", "movie", "m.mkv"]);
        assert!(result.is_err(), "unknown sources must not parse");
    }

    #[test]
    fn movie_requires_a_file() {
        let result = Cli::try_parse_from(["mntk", "movie"]);
        assert!(result.is_err(), "the file argument is required");
    }

    #[test]
    fn source_round_trips_through_its_cli_name() {
        assert_eq!(Source::from_str("omdb", true).unwrap(), Source::Omdb);
        assert!(Source::from_str("tmdb", true).is_err());
    }
}
