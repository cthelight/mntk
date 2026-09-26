//! File naming rules.
//!
//! Turns a [`MovieSearchResult`] into a directory and file name suitable
//! for moving a media file into. All functions are pure: no I/O, no
//! environment access, so the rename decision is fully unit-testable.

use std::path::{Path, PathBuf};

use crate::source::MovieSearchResult;

/// Characters that are invalid in file names.
///
/// This is Windows' set (the most restrictive common denominator) plus the
/// backslash; the original bash scripts used the same set minus `\`.
const INVALID_CHARS: &[char] = &[':', '\\', '/', '*', '"', '<', '>', '|', '?'];

/// Replaces characters that are invalid in file names with `_`.
///
/// Leading whitespace is trimmed and any trailing run of dots and
/// whitespace is stripped, both of which are problematic on Windows.
pub fn sanitize(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| if INVALID_CHARS.contains(&c) { '_' } else { c })
        .collect();
    let trimmed = replaced.trim_start();
    let end = trimmed
        .trim_end_matches(|c: char| c == '.' || c.is_whitespace())
        .len();
    trimmed[..end].to_string()
}

/// Checks that `id` looks like an IMDB id: `tt` followed by 7 to 10 digits.
///
/// The `tt` prefix is accepted in either case; ids are conventionally
/// lowercase.
pub fn is_valid_imdb_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    bytes.len() >= 9
        && bytes.len() <= 12
        && (bytes[0] == b't' || bytes[0] == b'T')
        && (bytes[1] == b't' || bytes[1] == b'T')
        && bytes[2..].iter().all(u8::is_ascii_digit)
}

/// The computed names for a rename operation.
///
/// Both names are relative to whatever base directory the caller chooses
/// (the CLI uses the current working directory).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamePlan {
    /// The movie directory name, e.g. `The Matrix (1999)`.
    pub dir_name: String,
    /// The file name including extension, e.g.
    /// `The Matrix (1999) [imdbid-tt0133093].mkv`.
    pub file_name: String,
}

impl RenamePlan {
    /// The movie directory path under `base`.
    pub fn target_dir(&self, base: &Path) -> PathBuf {
        base.join(&self.dir_name)
    }

    /// The final file path under `base`.
    pub fn target_file(&self, base: &Path) -> PathBuf {
        self.target_dir(base).join(&self.file_name)
    }
}

/// Computes the directory and file name for moving `source` to its new
/// location according to the metadata in `result`.
///
/// The layout is `<dir_name>/<file_name>` where `dir_name` is the sanitized
/// `Title (Year)` and `file_name` is that plus an optional
/// `[imdbid-ttXXXXXXX]` tag (Jellyfin recognizes it) and the source file's
/// extension.
pub fn plan_rename(source: &Path, result: &MovieSearchResult, embed_imdb_id: bool) -> RenamePlan {
    let base = base_name(&result.title, result.year);
    let dir_name = sanitize(&base);
    let mut stem = dir_name.clone();
    if embed_imdb_id && let Some(id) = result.imdb_id.as_deref().filter(|id| is_valid_imdb_id(id)) {
        // Normalize to lowercase: that is the canonical form.
        stem.push_str(&format!(" [imdbid-{}]", id.to_ascii_lowercase()));
    }
    let file_name = match source.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => format!("{stem}.{ext}"),
        None => stem,
    };
    RenamePlan {
        dir_name,
        file_name,
    }
}

fn base_name(title: &str, year: Option<u16>) -> String {
    match year {
        Some(year) => format!("{title} ({year})"),
        None => title.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceId;

    fn result(title: &str, year: Option<u16>, imdb_id: Option<&str>) -> MovieSearchResult {
        MovieSearchResult {
            id: SourceId::new(imdb_id.unwrap_or("tt0000000")),
            imdb_id: imdb_id.map(str::to_string),
            title: title.to_string(),
            year,
        }
    }

    #[test]
    fn sanitize_replaces_each_invalid_character() {
        for invalid in ['/', '\\', ':', '*', '"', '<', '>', '|', '?'] {
            let input = format!("a{invalid}b");
            assert_eq!(sanitize(&input), "a_b", "failed for {invalid:?}");
        }
    }

    #[test]
    fn sanitize_leaves_valid_characters_untouched() {
        assert_eq!(sanitize("The Matrix (1999)"), "The Matrix (1999)");
        // Unicode is preserved; only the invalid ASCII set is replaced.
        assert_eq!(sanitize("Šöme Mörvie: 24\""), "Šöme Mörvie_ 24_");
    }

    #[test]
    fn sanitize_trims_whitespace_and_trailing_dots() {
        assert_eq!(sanitize("  padded  "), "padded");
        assert_eq!(sanitize("trailing..."), "trailing");
        assert_eq!(sanitize("dots . . "), "dots");
        assert_eq!(sanitize("no trailing junk"), "no trailing junk");
    }

    #[test]
    fn plan_normalizes_imdb_id_case() {
        let plan = plan_rename(
            Path::new("/tmp/a.mkv"),
            &result("The Matrix", Some(1999), Some("TT0133093")),
            true,
        );
        assert_eq!(plan.file_name, "The Matrix (1999) [imdbid-tt0133093].mkv");
    }

    #[test]
    fn sanitize_handles_empty_and_degenerate_input() {
        assert_eq!(sanitize(""), "");
        assert_eq!(sanitize("..."), "");
        assert_eq!(sanitize("///"), "___");
    }

    #[test]
    fn is_valid_imdb_id_accepts_tt_plus_7_to_10_digits() {
        assert!(is_valid_imdb_id("tt0133093"));
        assert!(is_valid_imdb_id("TT0133093"));
        assert!(is_valid_imdb_id("tt1234567890"));
        assert!(!is_valid_imdb_id("tt123456")); // too short
        assert!(!is_valid_imdb_id("tt12345678901")); // too long
        assert!(!is_valid_imdb_id("xx0133093")); // wrong prefix
        assert!(!is_valid_imdb_id("tt0133o93")); // non-digit
    }

    #[test]
    fn plan_with_year_and_imdb_id() {
        let plan = plan_rename(
            Path::new("/tmp/The.Matrix.1999.1080p.mkv"),
            &result("The Matrix", Some(1999), Some("tt0133093")),
            true,
        );
        assert_eq!(plan.dir_name, "The Matrix (1999)");
        assert_eq!(plan.file_name, "The Matrix (1999) [imdbid-tt0133093].mkv");
    }

    #[test]
    fn plan_without_imdb_embedding() {
        let plan = plan_rename(
            Path::new("/tmp/a.mkv"),
            &result("The Matrix", Some(1999), Some("tt0133093")),
            false,
        );
        assert_eq!(plan.file_name, "The Matrix (1999).mkv");
    }

    #[test]
    fn plan_ignores_invalid_imdb_ids() {
        let plan = plan_rename(
            Path::new("/tmp/a.mkv"),
            &result("The Matrix", Some(1999), Some("not-an-id")),
            true,
        );
        assert_eq!(plan.file_name, "The Matrix (1999).mkv");
    }

    #[test]
    fn plan_without_year_omits_parentheses() {
        let plan = plan_rename(
            Path::new("/tmp/a.avi"),
            &result("Mystery", None, None),
            true,
        );
        assert_eq!(plan.dir_name, "Mystery");
        assert_eq!(plan.file_name, "Mystery.avi");
    }

    #[test]
    fn plan_sanitizes_the_title() {
        let plan = plan_rename(
            Path::new("/tmp/a.mkv"),
            &result("Schindler's List: The Cut", Some(1993), None),
            false,
        );
        assert_eq!(plan.dir_name, "Schindler's List_ The Cut (1993)");
        assert_eq!(plan.file_name, "Schindler's List_ The Cut (1993).mkv");
    }

    #[test]
    fn plan_preserves_the_source_extension_case() {
        let plan = plan_rename(
            Path::new("/tmp/a.MP4"),
            &result("X", Some(2000), None),
            false,
        );
        assert_eq!(plan.file_name, "X (2000).MP4");
    }

    #[test]
    fn plan_without_extension_has_no_dot() {
        let plan = plan_rename(Path::new("/tmp/a"), &result("X", Some(2000), None), false);
        assert_eq!(plan.file_name, "X (2000)");
    }

    #[test]
    fn target_paths_join_under_base() {
        let plan = RenamePlan {
            dir_name: "The Matrix (1999)".to_string(),
            file_name: "The Matrix (1999).mkv".to_string(),
        };
        let base = Path::new("/home/user/movies");
        assert_eq!(
            plan.target_dir(base),
            Path::new("/home/user/movies/The Matrix (1999)")
        );
        assert_eq!(
            plan.target_file(base),
            Path::new("/home/user/movies/The Matrix (1999)/The Matrix (1999).mkv")
        );
    }
}
