//! The `mntk movie` command.
//!
//! Searches a metadata source for the movie a file belongs to, then moves
//! the file to `<Title (Year)>/<Title (Year) [imdbid-ttXXXXXXX].<ext>`
//! under the current working directory.
//!
//! Interactive sessions get a dialog-style TUI: an input box for the search
//! term (never assumed from the file name unless `--guess` is passed), then
//! a menu of results. Non-interactive sessions (no TTY) require a search
//! term and never loop.

use std::io::{ErrorKind, IsTerminal};
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use mntk_core::config::{self, Config};
use mntk_core::naming::plan_rename;
use mntk_core::source::{MovieSearchResult, MovieSource};
use mntk_source_omdb::OmdbSource;
use thiserror::Error;

use crate::cli::{GlobalOpts, MovieOpts, Source};
use crate::tui;

/// A user interruption (Ctrl+C or Esc).
/// Mapped to the conventional exit code 130.
#[derive(Debug, Error)]
#[error("interrupted")]
pub struct Interrupted;

pub fn run(opts: MovieOpts, global: GlobalOpts) -> Result<()> {
    if !opts.file.is_file() {
        bail!("file not found: {}", opts.file.display());
    }

    let config = Config::load(global.config.as_deref())?;
    if config.source != global.source.as_str() {
        bail!(
            "config file requests source {:?}, but only \"{}\" is available",
            config.source,
            global.source.as_str()
        );
    }
    let source = build_source(&global, &config)?;

    let embed_imdb_id = config.embed_imdb_id && !opts.no_imdb_id;
    let base =
        std::env::current_dir().context("failed to determine the current working directory")?;

    let result = choose_result(&*source, &opts)?;
    let plan = plan_rename(&opts.file, &result, embed_imdb_id);
    let target = plan.target_file(&base);

    if opts.dry_run {
        println!("would move:");
        println!("  from: {}", opts.file.display());
        println!("  to:   {}", target.display());
        return Ok(());
    }

    if target.exists() && !opts.force {
        bail!(
            "destination already exists: {} (use --force to overwrite)",
            target.display()
        );
    }

    move_file(&opts.file, &target).with_context(|| {
        format!(
            "failed to move {} to {}",
            opts.file.display(),
            target.display()
        )
    })?;
    println!("renamed:");
    println!("  from: {}", opts.file.display());
    println!("  to:   {}", target.display());
    Ok(())
}

/// Composition root: the only place that knows which concrete source crate
/// backs a [`Source`].
fn build_source(global: &GlobalOpts, config: &Config) -> Result<Box<dyn MovieSource>> {
    let api_key = config::resolve_api_key(
        global.api_key.as_deref(),
        config::api_key_from_env().as_deref(),
        config.omdb.api_key.as_deref(),
    );
    match global.source {
        Source::Omdb => {
            let api_key = api_key.ok_or_else(|| {
                anyhow!(
                    "no OMDB API key: set the OMDB_API_KEY environment variable, pass --api-key, or add omdb.api_key to the config file"
                )
            })?;
            Ok(Box::new(OmdbSource::new(
                api_key,
                config.omdb.base_url.clone(),
            )))
        }
    }
}

fn choose_result(source: &dyn MovieSource, opts: &MovieOpts) -> Result<MovieSearchResult> {
    if is_interactive() {
        choose_interactive(source, opts)
    } else {
        choose_non_interactive(source, opts)
    }
}

/// A TTY on both ends is required: the TUI reads keys from stdin and draws
/// to stdout.
fn is_interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn choose_interactive(source: &dyn MovieSource, opts: &MovieOpts) -> Result<MovieSearchResult> {
    let file_name = opts.file.display().to_string();
    let search = |term: &str| -> Result<Vec<MovieSearchResult>> {
        source
            .search_movies(term)
            .with_context(|| format!("search for {term:?} failed"))
    };
    match tui::session(&file_name, initial_term(opts), search)? {
        Some(result) => Ok(result),
        None => Err(anyhow::Error::new(Interrupted)),
    }
}

/// The term to prefill the input box with: `--search` wins, `--guess`
/// derives a term from the file name, and otherwise the box starts empty —
/// the file name is never assumed.
fn initial_term(opts: &MovieOpts) -> Option<String> {
    if let Some(search) = opts
        .search
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(search.to_string());
    }
    if opts.guess {
        return derive_query(&opts.file);
    }
    None
}

fn choose_non_interactive(source: &dyn MovieSource, opts: &MovieOpts) -> Result<MovieSearchResult> {
    let query = initial_term(opts).ok_or_else(|| {
        anyhow!(
            "no TTY detected: pass --search <TITLE>, or --guess to derive a term from the file name"
        )
    })?;
    let results = source.search_movies(&query)?;
    if results.is_empty() {
        bail!("no results for {query:?}");
    }
    if let Some(selected) = opts.select {
        let result = selected
            .checked_sub(1)
            .and_then(|index| results.get(index as usize))
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "--select {selected} is out of range: {} results are available",
                    results.len()
                )
            })?;
        return Ok(result);
    }
    if results.len() == 1 {
        return Ok(results.into_iter().next().expect("len is 1"));
    }
    let mut message = format!(
        "{} results for {query:?}; pass --select N to pick one without a TTY:",
        results.len()
    );
    for (index, result) in results.iter().enumerate() {
        message.push_str(&format!("\n  {}. {}", index + 1, tui::label(result)));
    }
    bail!("{message}")
}

/// Moves `from` to `to`, creating the destination directory.
///
/// `rename(2)` cannot cross filesystems; in that case fall back to copy +
/// delete.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::CrossesDevices => {
            std::fs::copy(from, to)?;
            std::fs::remove_file(from)
        }
        Err(error) => Err(error),
    }
}

/// Derives a search term from a file name: bracketed segments (release tags,
/// imdbid markers) are dropped, the rest is split on non-alphanumerics, and
/// the tokens stop after the first year-like one, since whatever follows is
/// usually release junk.
fn derive_query(file: &Path) -> Option<String> {
    let stem = file
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())?;
    let stripped = strip_brackets(&stem);
    let tokens: Vec<&str> = stripped
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect();
    let end = tokens
        .iter()
        .position(|token| token.len() == 4 && token.bytes().all(|byte| byte.is_ascii_digit()))
        .map(|index| index + 1)
        .unwrap_or(tokens.len());
    let query = tokens[..end].join(" ");
    if query.is_empty() { None } else { Some(query) }
}

fn strip_brackets(stem: &str) -> String {
    let mut out = String::with_capacity(stem.len());
    let mut depth = 0usize;
    for character in stem.chars() {
        match character {
            '[' => depth += 1,
            ']' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(character),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mntk_core::source::{SourceError, SourceId};
    use std::path::PathBuf;

    struct StubSource(Vec<MovieSearchResult>);

    impl MovieSource for StubSource {
        fn name(&self) -> &'static str {
            "stub"
        }

        fn search_movies(&self, _query: &str) -> Result<Vec<MovieSearchResult>, SourceError> {
            Ok(self.0.clone())
        }
    }

    /// A stub that records the query it was asked for.
    struct RecordingSource {
        results: Vec<MovieSearchResult>,
        last_query: std::cell::RefCell<Option<String>>,
    }

    impl MovieSource for RecordingSource {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn search_movies(&self, query: &str) -> Result<Vec<MovieSearchResult>, SourceError> {
            *self.last_query.borrow_mut() = Some(query.to_string());
            Ok(self.results.clone())
        }
    }

    fn result(title: &str, id: &str, year: Option<u16>) -> MovieSearchResult {
        MovieSearchResult {
            id: SourceId::new(id),
            imdb_id: Some(id.to_string()),
            title: title.to_string(),
            year,
        }
    }

    fn opts(search: Option<&str>, select: Option<u32>) -> MovieOpts {
        MovieOpts {
            file: PathBuf::from("movie.mkv"),
            search: search.map(str::to_string),
            select,
            guess: false,
            dry_run: false,
            force: false,
            no_imdb_id: false,
        }
    }

    #[test]
    fn derive_query_stops_after_the_year() {
        let query = derive_query(Path::new("The.Matrix.1999.1080p.BluRay.mkv")).unwrap();
        assert_eq!(query, "The Matrix 1999");
    }

    #[test]
    fn derive_query_strips_bracketed_segments() {
        let query = derive_query(Path::new("the matrix (1999) [imdbid-tt0133093].mkv")).unwrap();
        assert_eq!(query, "the matrix 1999");
    }

    #[test]
    fn derive_query_without_a_year_keeps_all_tokens() {
        let query = derive_query(Path::new("Interstellar.2160p.WEB-DL.mkv")).unwrap();
        assert_eq!(query, "Interstellar 2160p WEB DL");
    }

    #[test]
    fn derive_query_without_alphanumerics_is_none() {
        assert_eq!(derive_query(Path::new("!!!.???")), None);
    }

    #[test]
    fn move_file_creates_the_destination_directory() {
        let dir = std::env::temp_dir().join(format!("mntk-move-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let from = dir.join("movie.mkv");
        std::fs::write(&from, b"data").unwrap();
        let to = dir.join("The Matrix (1999)").join("The Matrix (1999).mkv");

        move_file(&from, &to).unwrap();

        assert!(!from.exists());
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "data");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_interactive_requires_a_search_term() {
        let source = StubSource(vec![]);
        let error = choose_non_interactive(&source, &opts(None, None)).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("no TTY detected"), "{message}");
        assert!(message.contains("--search"), "{message}");
        assert!(message.contains("--guess"), "{message}");
    }

    #[test]
    fn non_interactive_guess_sends_the_derived_term() {
        let source = RecordingSource {
            results: vec![result("The Matrix", "tt0133093", Some(1999))],
            last_query: std::cell::RefCell::new(None),
        };
        let mut opts = opts(None, None);
        opts.guess = true;
        opts.file = PathBuf::from("The.Matrix.1999.1080p.mkv");

        let chosen = choose_non_interactive(&source, &opts).unwrap();
        assert_eq!(chosen.id.as_str(), "tt0133093");
        assert_eq!(
            source.last_query.borrow().as_deref(),
            Some("The Matrix 1999")
        );
    }

    #[test]
    fn initial_term_is_none_without_flags() {
        assert_eq!(initial_term(&opts(None, None)), None);
    }

    #[test]
    fn initial_term_uses_the_search_flag() {
        assert_eq!(
            initial_term(&opts(Some("the matrix"), None)).as_deref(),
            Some("the matrix")
        );
    }

    #[test]
    fn initial_term_guesses_from_the_file_name() {
        let mut opts = opts(None, None);
        opts.guess = true;
        opts.file = PathBuf::from("The.Matrix.1999.1080p.mkv");
        assert_eq!(initial_term(&opts).as_deref(), Some("The Matrix 1999"));
    }

    #[test]
    fn initial_term_guess_is_none_when_the_file_name_is_useless() {
        let mut opts = opts(None, None);
        opts.guess = true;
        opts.file = PathBuf::from("!!!.???");
        assert_eq!(initial_term(&opts), None);
    }

    #[test]
    fn non_interactive_single_result_is_auto_selected() {
        let source = StubSource(vec![result("The Matrix", "tt0133093", Some(1999))]);
        let chosen = choose_non_interactive(&source, &opts(Some("the matrix"), None)).unwrap();
        assert_eq!(chosen.id.as_str(), "tt0133093");
    }

    #[test]
    fn non_interactive_select_picks_by_index() {
        let source = StubSource(vec![
            result("The Matrix", "tt0133093", Some(1999)),
            result("The Matrix Revisited", "tt11000618", Some(2023)),
        ]);
        let chosen = choose_non_interactive(&source, &opts(Some("the matrix"), Some(2))).unwrap();
        assert_eq!(chosen.id.as_str(), "tt11000618");
    }

    #[test]
    fn non_interactive_select_out_of_range_is_an_error() {
        let source = StubSource(vec![result("The Matrix", "tt0133093", Some(1999))]);
        let error =
            choose_non_interactive(&source, &opts(Some("the matrix"), Some(3))).unwrap_err();
        assert!(error.to_string().contains("out of range"), "{error}");
    }

    #[test]
    fn non_interactive_multiple_results_list_candidates() {
        let source = StubSource(vec![
            result("The Matrix", "tt0133093", Some(1999)),
            result("The Matrix Revisited", "tt11000618", Some(2023)),
        ]);
        let error = choose_non_interactive(&source, &opts(Some("the matrix"), None)).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("2 results"), "{message}");
        assert!(message.contains("tt0133093"), "{message}");
        assert!(message.contains("tt11000618"), "{message}");
        assert!(message.contains("--select"), "{message}");
    }

    #[test]
    fn non_interactive_empty_results_are_an_error() {
        let source = StubSource(vec![]);
        let error = choose_non_interactive(&source, &opts(Some("nope"), None)).unwrap_err();
        assert!(error.to_string().contains("no results"), "{error}");
    }

    #[test]
    fn interrupted_is_detected_as_its_own_error() {
        let error = anyhow::Error::new(Interrupted);
        assert!(error.is::<Interrupted>());
        assert_eq!(error.to_string(), "interrupted");
    }
}
