//! OMDB JSON response shapes.
//!
//! Only the fields mntk needs are deserialized; everything else (Posters,
//! totalResults, ...) is ignored.

use serde::Deserialize;

/// The response to a `s=<title>` search query.
#[derive(Debug, Deserialize)]
pub(crate) struct SearchResponse {
    /// OMDB uses the strings `"True"`/`"False"` here, not JSON booleans.
    #[serde(rename = "Response")]
    pub response: String,
    /// Present when `response` is `"False"`; carries OMDB's explanation
    /// (e.g. `"Invalid API key!"`).
    #[serde(rename = "Error", default)]
    pub error: Option<String>,
    /// Present when `response` is `"True"`.
    ///
    /// The free API (www.omdbapi.com) names this array `Search`; the paid
    /// API (api.omdbapi.com) names it `Results`. Both spellings are
    /// accepted. `totalResults` sits next to it at the root level.
    #[serde(rename = "Search", alias = "Results", default)]
    pub results: Vec<SearchEntry>,
}

/// One row of a search result.
#[derive(Debug, Deserialize)]
pub(crate) struct SearchEntry {
    #[serde(rename = "Title")]
    pub title: String,
    /// OMDB uses `"N/A"` when it does not know the year.
    #[serde(rename = "Year")]
    pub year: String,
    #[serde(rename = "imdbID")]
    pub imdb_id: String,
    /// Lowercase `"movie"`, `"series"` or `"episode"` on the search endpoint
    /// (the detail endpoint capitalizes the first letter), so callers must
    /// compare case-insensitively.
    #[serde(rename = "Type")]
    pub r#type: String,
}
