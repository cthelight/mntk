//! The metadata source abstraction.
//!
//! A [`MovieSource`] is anything that can answer "which movies match this
//! title?". Concrete providers (OMDB today, TVDB/TMDB in the future) live in
//! their own crates and implement this trait; the rest of mntk only ever
//! deals in [`MovieSearchResult`] values.

use thiserror::Error;

/// A movie search result in provider-agnostic terms.
#[derive(Debug, Clone)]
pub struct MovieSearchResult {
    /// The source's canonical id for this movie (e.g. `"tt0133093"` on OMDB).
    pub id: SourceId,
    /// The IMDB id, when the provider knows it.
    ///
    /// Used to embed a Jellyfin-recognizable `[imdbid-ttXXXXXXX]` tag in file
    /// names.
    pub imdb_id: Option<String>,
    /// The movie title as reported by the source.
    pub title: String,
    /// The release year, when the source reports one.
    pub year: Option<u16>,
}

/// A provider-native movie id.
///
/// A newtype so that ids from different providers can never be confused with
/// each other (or with, say, IMDB ids) at the type level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceId(String);

impl SourceId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A metadata source that can search for movies.
pub trait MovieSource {
    /// A stable, human-readable name for this source (e.g. `"omdb"`).
    fn name(&self) -> &'static str;

    /// Search for movies whose title matches `query`.
    ///
    /// Implementations should make as few network requests as necessary and
    /// return results in the source's own ranking order.
    fn search_movies(&self, query: &str) -> Result<Vec<MovieSearchResult>, SourceError>;
}

/// Errors returned by a metadata source.
#[derive(Debug, Error)]
pub enum SourceError {
    /// A transport-level failure (DNS, connection refused, timeout, TLS...).
    #[error("network error while contacting the metadata source: {0}")]
    Transport(String),

    /// The source answered with a non-2xx HTTP status.
    #[error("metadata source returned HTTP status {0}")]
    Http(u16),

    /// The source's response could not be parsed.
    #[error("could not parse metadata source response: {0}")]
    Parse(String),

    /// The search completed but found nothing.
    ///
    /// `detail` carries the source's own explanation when it has one (e.g.
    /// OMDB's `"Invalid API key!"`).
    #[error("metadata source found no results: {0}")]
    NotFound(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubSource;

    impl MovieSource for StubSource {
        fn name(&self) -> &'static str {
            "stub"
        }

        fn search_movies(&self, _query: &str) -> Result<Vec<MovieSearchResult>, SourceError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn trait_is_object_safe() {
        let source: Box<dyn MovieSource> = Box::new(StubSource);
        assert_eq!(source.name(), "stub");
        assert!(source.search_movies("anything").unwrap().is_empty());
    }

    #[test]
    fn source_id_round_trips() {
        let id = SourceId::new("tt0133093");
        assert_eq!(id.as_str(), "tt0133093");
        assert_eq!(id.to_string(), "tt0133093");
        assert_eq!(id, SourceId::new("tt0133093"));
    }

    #[test]
    fn source_error_display() {
        let err = SourceError::NotFound("Invalid API key!".to_string());
        assert_eq!(err.to_string(), "metadata source found no results: Invalid API key!");

        let err = SourceError::Http(404);
        assert_eq!(err.to_string(), "metadata source returned HTTP status 404");
    }
}
