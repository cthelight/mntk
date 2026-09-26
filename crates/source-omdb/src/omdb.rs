//! The OMDB provider.

use mntk_core::source::{MovieSearchResult, MovieSource, SourceError, SourceId};

use crate::models::SearchResponse;
use crate::transport::{Transport, TransportError, UreqTransport};

/// The OMDB metadata source (http://www.omdbapi.com).
///
/// A title search costs exactly one HTTP request.
pub struct OmdbSource {
    api_key: String,
    base_url: String,
    transport: Box<dyn Transport>,
}

impl OmdbSource {
    /// Create a source using the production ureq transport.
    pub fn new(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self::with_transport(api_key, base_url, Box::new(UreqTransport::new()))
    }

    /// Create a source with a custom transport (used by tests).
    pub(crate) fn with_transport(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        transport: Box<dyn Transport>,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: base_url.into(),
            transport,
        }
    }

    fn search_url(&self, query: &str) -> String {
        let mut query_string = String::new();
        let mut serializer = form_urlencoded::Serializer::new(&mut query_string);
        serializer.append_pair("apikey", &self.api_key);
        serializer.append_pair("s", query);
        // Trim trailing slashes from the base, then add exactly one, so
        // "http://host" and "http://host/" both yield a well-formed URL.
        format!("{}/?{}", self.base_url.trim_end_matches('/'), query_string)
    }
}

impl MovieSource for OmdbSource {
    fn name(&self) -> &'static str {
        "omdb"
    }

    fn search_movies(&self, query: &str) -> Result<Vec<MovieSearchResult>, SourceError> {
        if query.trim().is_empty() {
            return Err(SourceError::NotFound("empty search query".to_string()));
        }

        let response = self
            .transport
            .get(&self.search_url(query))
            .map_err(|error: TransportError| SourceError::Transport(error.to_string()))?;
        if !(200..300).contains(&response.status) {
            return Err(SourceError::Http(response.status));
        }

        let parsed: SearchResponse = serde_json::from_str(&response.body)
            .map_err(|error| SourceError::Parse(error.to_string()))?;
        if parsed.response != "True" {
            let detail = parsed.error.unwrap_or_else(|| "no results".to_string());
            return Err(SourceError::NotFound(detail));
        }

        Ok(parsed
            .search
            .into_iter()
            // mntk's movie tool only wants movies; OMDB mixes in series and
            // episodes for the same title.
            .filter(|entry| entry.r#type == "Movie")
            .map(|entry| MovieSearchResult {
                id: SourceId::new(entry.imdb_id.clone()),
                imdb_id: Some(entry.imdb_id),
                title: entry.title,
                year: parse_year(&entry.year),
            })
            .collect())
    }
}

fn parse_year(raw: &str) -> Option<u16> {
    // OMDB reports "N/A" when it does not know the year. The lower bound
    // rejects garbage like "0".
    raw.parse::<u16>().ok().filter(|year| *year >= 1800)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::TransportResponse;
    use std::cell::RefCell;

    /// In-memory transport for tests.
    #[derive(Default)]
    struct MockTransport {
        response: RefCell<Option<TransportResponse>>,
        error: RefCell<Option<String>>,
        last_url: RefCell<Option<String>>,
    }

    impl Transport for std::rc::Rc<MockTransport> {
        fn get(&self, url: &str) -> Result<TransportResponse, TransportError> {
            (**self).get(url)
        }
    }

    impl MockTransport {
        fn json(body: &str) -> Self {
            Self {
                response: RefCell::new(Some(TransportResponse {
                    status: 200,
                    body: body.into(),
                })),
                ..Default::default()
            }
        }

        fn status(code: u16) -> Self {
            Self {
                response: RefCell::new(Some(TransportResponse {
                    status: code,
                    body: String::new(),
                })),
                ..Default::default()
            }
        }

        fn failing(message: &str) -> Self {
            Self {
                error: RefCell::new(Some(message.to_string())),
                ..Default::default()
            }
        }

        fn last_url(&self) -> Option<String> {
            self.last_url.borrow().clone()
        }
    }

    impl Transport for MockTransport {
        fn get(&self, url: &str) -> Result<TransportResponse, TransportError> {
            *self.last_url.borrow_mut() = Some(url.to_string());
            if let Some(error) = self.error.borrow().clone() {
                return Err(TransportError(error));
            }
            Ok(self
                .response
                .borrow()
                .clone()
                .expect("no response configured"))
        }
    }

    fn source(transport: MockTransport) -> OmdbSource {
        OmdbSource::with_transport("secret", "http://omdb.example.com/", Box::new(transport))
    }

    const SEARCH_OK: &str = r##"
    {
        "Search": [
            { "Title": "The Matrix", "Year": "1999", "imdbID": "tt0133093", "Type": "Movie", "Poster": "https://x/1.jpg" },
            { "Title": "The Matrix Revisited", "Year": "2023", "imdbID": "tt11000618", "Type": "Movie", "Poster": "https://x/2.jpg" },
            { "Title": "The Matrix", "Year": "2021", "imdbID": "tt8986062", "Type": "Episode", "Poster": "https://x/3.jpg" },
            { "Title": "The Matrix", "Year": "1999", "imdbID": "tt10000001", "Type": "Series", "Poster": "https://x/4.jpg" },
            { "Title": "The Matrix: Unknown Year", "Year": "N/A", "imdbID": "tt10000002", "Type": "Movie", "Poster": "https://x/5.jpg" }
        ],
        "totalResults": "5",
        "Response": "True"
    }
    "##;

    #[test]
    fn name_is_omdb() {
        let source = source(MockTransport::default());
        assert_eq!(source.name(), "omdb");
    }

    #[test]
    fn search_maps_movie_results_and_filters_other_types() {
        let source = source(MockTransport::json(SEARCH_OK));
        let results = source.search_movies("the matrix").unwrap();

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].title, "The Matrix");
        assert_eq!(results[0].year, Some(1999));
        assert_eq!(results[0].id.as_str(), "tt0133093");
        assert_eq!(results[0].imdb_id.as_deref(), Some("tt0133093"));

        assert_eq!(results[1].title, "The Matrix Revisited");
        assert_eq!(results[1].year, Some(2023));

        // "N/A" year maps to None.
        assert_eq!(results[2].title, "The Matrix: Unknown Year");
        assert_eq!(results[2].year, None);
    }

    #[test]
    fn search_url_is_form_encoded() {
        let mock = std::rc::Rc::new(MockTransport::json(SEARCH_OK));
        let source = OmdbSource::with_transport(
            "secret",
            "http://omdb.example.com/",
            Box::new(mock.clone()),
        );
        source.search_movies("The Matrix & The City").unwrap();

        assert_eq!(
            mock.last_url().as_deref(),
            Some("http://omdb.example.com/?apikey=secret&s=The+Matrix+%26+The+City")
        );
    }

    #[test]
    fn response_false_returns_not_found_with_detail() {
        let source = source(MockTransport::json(
            r#"{"Response": "False", "Error": "Movie not found!"}"#,
        ));
        let err = source.search_movies("nonexistent movie xyz").unwrap_err();
        match err {
            SourceError::NotFound(detail) => assert_eq!(detail, "Movie not found!"),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn invalid_api_key_detail_surfaces() {
        let source = source(MockTransport::json(
            r#"{"Response": "False", "Error": "Invalid API key! See https://www.omdbapi.com/err0b.php."}"#,
        ));
        let err = source.search_movies("the matrix").unwrap_err();
        match err {
            SourceError::NotFound(detail) => assert!(detail.starts_with("Invalid API key!")),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn non_2xx_status_is_an_http_error() {
        let source = source(MockTransport::status(404));
        let err = source.search_movies("the matrix").unwrap_err();
        assert!(matches!(err, SourceError::Http(404)), "{err:?}");
    }

    #[test]
    fn transport_failure_is_a_transport_error() {
        let source = source(MockTransport::failing("dns lookup failed"));
        let err = source.search_movies("the matrix").unwrap_err();
        match err {
            SourceError::Transport(detail) => assert_eq!(detail, "dns lookup failed"),
            other => panic!("expected Transport, got {other:?}"),
        }
    }

    #[test]
    fn malformed_json_is_a_parse_error() {
        let source = source(MockTransport::json("{not json"));
        let err = source.search_movies("the matrix").unwrap_err();
        assert!(matches!(err, SourceError::Parse(_)), "{err:?}");
    }

    #[test]
    fn empty_query_is_rejected_without_network() {
        let mock = std::rc::Rc::new(MockTransport::json(SEARCH_OK));
        let source = OmdbSource::with_transport("k", "http://x", Box::new(mock.clone()));
        let err = source.search_movies("   ").unwrap_err();
        assert!(matches!(err, SourceError::NotFound(_)), "{err:?}");
        assert!(mock.last_url().is_none(), "no request may be sent");
    }

    #[test]
    fn parse_year_rejects_garbage() {
        assert_eq!(parse_year("1999"), Some(1999));
        assert_eq!(parse_year("N/A"), None);
        assert_eq!(parse_year("0"), None);
        assert_eq!(parse_year(""), None);
    }
}
