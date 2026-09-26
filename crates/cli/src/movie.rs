//! The `mntk movie` command.
//!
//! Searches a metadata source for the movie a file belongs to, then moves
//! the file to `<Title (Year)>/<Title (Year) [imdbid-ttXXXXXXX]>.<ext>`
//! under the current working directory.
//!
//! Interactive sessions get a re-entrant dialog: pick a result or ask for
//! another search. Non-interactive sessions (no TTY) require `--search`
//! and never loop.

use std::io::{ErrorKind, IsTerminal};
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use dialoguer::{Input, Select};
use mntk_core::config::{self, Config};
use mntk_core::naming::plan_rename;
use mntk_core::source::{MovieSearchResult, MovieSource};
use mntk_source_omdb::OmdbSource;
use thiserror::Error;

use crate::cli::{GlobalOpts, MovieOpts, Source};

/// A user interruption (Ctrl+C). Mapped to the conventional exit code 130.
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
    if std::io::stdin().is_terminal() {
        choose_interactive(source, opts)
    } else {
        choose_non_interactive(source, opts)
    }
}

fn choose_interactive(source: &dyn MovieSource, opts: &MovieOpts) -> Result<MovieSearchResult> {
    let mut query = opts
        .search
        .clone()
        .map(|search| search.trim().to_string())
        .filter(|search| !search.is_empty())
        .or_else(|| derive_query(&opts.file))
        .unwrap_or_default();

    loop {
        let results = if query.is_empty() {
            query = ask_term(None)?;
            source.search_movies(&query)?
        } else {
            source.search_movies(&query)?
        };
        if results.is_empty() {
            println!("no results for {query:?}.");
            query = ask_term(Some(query.clone()))?;
            continue;
        }
        match pick_from_menu(&results)? {
            Some(index) => {
                return Ok(results
                    .into_iter()
                    .nth(index)
                    .expect("menu index is in range"));
            }
            None => query = ask_term(Some(query.clone()))?,
        }
    }
}

fn choose_non_interactive(source: &dyn MovieSource, opts: &MovieOpts) -> Result<MovieSearchResult> {
    let query = opts
        .search
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .ok_or_else(|| {
            anyhow!("no TTY detected: pass --search <TITLE> to search non-interactively")
        })?;
    let results = source.search_movies(query)?;
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
        message.push_str(&format!("\n  {}. {}", index + 1, label(result)));
    }
    bail!("{message}")
}

fn ask_term(default: Option<String>) -> Result<String> {
    let prompt = Input::<String>::new().with_prompt("Search term");
    let prompt = match default {
        Some(default) => prompt.default(default),
        None => prompt,
    };
    prompt.interact_text().map_err(dialog_error)
}

/// `Some(index)` for a chosen result, `None` for "search again".
fn pick_from_menu(results: &[MovieSearchResult]) -> Result<Option<usize>> {
    let mut labels: Vec<String> = results.iter().map(label).collect();
    labels.push("Search again…".to_string());
    match Select::new()
        .with_prompt("Select the movie")
        .items(&labels)
        .default(0)
        .interact_opt()
    {
        Ok(None) => Err(anyhow::Error::new(Interrupted)),
        Ok(Some(index)) if index < results.len() => Ok(Some(index)),
        Ok(Some(_)) => Ok(None),
        Err(error) => Err(dialog_error(error)),
    }
}

fn label(result: &MovieSearchResult) -> String {
    let year = match result.year {
        Some(year) => year.to_string(),
        None => "N/A".to_string(),
    };
    format!("{} ({year}) - {}", result.title, result.id)
}

fn dialog_error(error: dialoguer::Error) -> anyhow::Error {
    match &error {
        dialoguer::Error::IO(io) if io.kind() == ErrorKind::Interrupted => {
            anyhow::Error::new(Interrupted)
        }
        _ => error.into(),
    }
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
    fn label_shows_title_year_and_id() {
        let formatted = label(&result("The Matrix", "tt0133093", Some(1999)));
        assert_eq!(formatted, "The Matrix (1999) - tt0133093");

        let formatted = label(&result("Mystery", "tt0000001", None));
        assert_eq!(formatted, "Mystery (N/A) - tt0000001");
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
        assert!(error.to_string().contains("no TTY detected"), "{error}");
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
