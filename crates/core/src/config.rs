//! Configuration for mntk.
//!
//! Settings come from a YAML file, by default located at
//! `$XDG_CONFIG_HOME/mntk/config.yaml` (or `~/.config/mntk/config.yaml`).
//! A missing config file is not an error: every setting has a default.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

/// The default metadata source name.
pub const DEFAULT_SOURCE: &str = "omdb";
/// The default OMDB API base URL (the free API endpoint).
pub const OMDB_DEFAULT_BASE_URL: &str = "http://www.omdbapi.com";

/// Environment variables that may carry the OMDB API key, in priority order.
pub const OMDB_API_KEY_ENV: &str = "OMDB_API_KEY";
/// Legacy environment variable name, kept for compatibility with the
/// original bash scripts.
pub const OMDB_API_KEY_ENV_LEGACY: &str = "OMDB_ID";

/// The fully resolved mntk configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The default metadata source name. May be overridden by a CLI flag.
    pub source: String,
    /// Whether file names should embed a `[imdbid-...]` tag.
    pub embed_imdb_id: bool,
    /// OMDB-specific settings.
    pub omdb: OmdbConfig,
}

/// OMDB-specific settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OmdbConfig {
    /// The OMDB API key, if set in the config file.
    pub api_key: Option<String>,
    /// The OMDB API base URL.
    pub base_url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            source: DEFAULT_SOURCE.to_string(),
            embed_imdb_id: true,
            omdb: OmdbConfig::default(),
        }
    }
}

impl Default for OmdbConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: OMDB_DEFAULT_BASE_URL.to_string(),
        }
    }
}

impl Config {
    /// Load the configuration.
    ///
    /// If `explicit` is `Some`, that file must exist and be valid. If it is
    /// `None`, the default location is used when it exists; otherwise the
    /// built-in defaults are returned.
    pub fn load(explicit: Option<&Path>) -> Result<Config, ConfigError> {
        match explicit {
            Some(path) => {
                let contents = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
                    path: path.to_path_buf(),
                    source,
                })?;
                Self::parse(path, &contents)
            }
            None => match default_path() {
                Some(path) if path.is_file() => {
                    let contents =
                        std::fs::read_to_string(&path).map_err(|source| ConfigError::Io {
                            path: path.clone(),
                            source,
                        })?;
                    Self::parse(&path, &contents)
                }
                _ => Ok(Config::default()),
            },
        }
    }

    fn parse(path: &Path, contents: &str) -> Result<Config, ConfigError> {
        let raw: RawConfig =
            serde_yaml_ng::from_str(contents).map_err(|source| ConfigError::Parse {
                path: path.to_path_buf(),
                source,
            })?;
        Ok(Config {
            source: nonempty(raw.source).unwrap_or_else(|| DEFAULT_SOURCE.to_string()),
            embed_imdb_id: raw.embed_imdb_id.unwrap_or(true),
            omdb: OmdbConfig {
                api_key: nonempty(raw.omdb.api_key),
                base_url: nonempty(raw.omdb.base_url)
                    .unwrap_or_else(|| OMDB_DEFAULT_BASE_URL.to_string()),
            },
        })
    }
}

/// The default config file location, if the platform has one.
pub fn default_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("mntk").join("config.yaml"))
}

/// Read the OMDB API key from the environment.
///
/// Checks [`OMDB_API_KEY_ENV`] first, then the legacy
/// [`OMDB_API_KEY_ENV_LEGACY`].
pub fn api_key_from_env() -> Option<String> {
    nonempty(std::env::var(OMDB_API_KEY_ENV).ok())
        .or_else(|| nonempty(std::env::var(OMDB_API_KEY_ENV_LEGACY).ok()))
}

/// Combine API key sources in priority order: CLI flag, environment, config
/// file.
///
/// `env` is the value of [`api_key_from_env`], passed in by the caller so
/// that this function stays pure.
pub fn resolve_api_key(
    cli: Option<&str>,
    env: Option<&str>,
    config: Option<&str>,
) -> Option<String> {
    nonempty(cli.map(str::to_string))
        .or_else(|| nonempty(env.map(str::to_string)))
        .or_else(|| nonempty(config.map(str::to_string)))
}

fn nonempty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawConfig {
    source: Option<String>,
    embed_imdb_id: Option<bool>,
    omdb: RawOmdb,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawOmdb {
    api_key: Option<String>,
    base_url: Option<String>,
}

/// Errors from loading or parsing the configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config file {}: {source}", path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_yaml_ng::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    struct TempFile(PathBuf);

    impl TempFile {
        fn new(name: &str, contents: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "mntk-config-test-{}-{}",
                std::process::id(),
                name
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("config.yaml");
            let mut file = std::fs::File::create(&path).unwrap();
            file.write_all(contents.as_bytes()).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
        }
    }

    #[test]
    fn explicit_missing_file_is_an_io_error() {
        let path = Path::new("/nonexistent/mntk/config.yaml");
        let err = Config::load(Some(path)).expect_err("missing file must fail");
        assert!(matches!(err, ConfigError::Io { .. }), "{err}");
    }

    #[test]
    fn empty_file_yields_defaults() {
        let file = TempFile::new("empty", "");
        let config = Config::load(Some(file.path())).unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn full_config_is_parsed() {
        let file = TempFile::new(
            "full",
            "source: omdb\nembed_imdb_id: false\nomdb:\n  api_key: key123\n  base_url: https://api.omdbbox.com\n",
        );
        let config = Config::load(Some(file.path())).unwrap();
        assert_eq!(config.source, "omdb");
        assert!(!config.embed_imdb_id);
        assert_eq!(config.omdb.api_key.as_deref(), Some("key123"));
        assert_eq!(config.omdb.base_url, "https://api.omdbbox.com");
    }

    #[test]
    fn partial_config_keeps_defaults() {
        let file = TempFile::new("partial", "omdb:\n  api_key: key123\n");
        let config = Config::load(Some(file.path())).unwrap();
        assert_eq!(config.source, DEFAULT_SOURCE);
        assert!(config.embed_imdb_id);
        assert_eq!(config.omdb.api_key.as_deref(), Some("key123"));
        assert_eq!(config.omdb.base_url, OMDB_DEFAULT_BASE_URL);
    }

    #[test]
    fn empty_strings_are_treated_as_unset() {
        let file = TempFile::new(
            "empty-strings",
            "source: \"\"\nomdb:\n  api_key: \"   \"\n  base_url: \"\"\n",
        );
        let config = Config::load(Some(file.path())).unwrap();
        assert_eq!(config.source, DEFAULT_SOURCE);
        assert!(config.omdb.api_key.is_none());
        assert_eq!(config.omdb.base_url, OMDB_DEFAULT_BASE_URL);
    }

    #[test]
    fn invalid_yaml_is_a_parse_error() {
        let file = TempFile::new("invalid", "source: [unclosed\n  omdb: : :");
        let err = Config::load(Some(file.path())).expect_err("invalid yaml must fail");
        assert!(matches!(err, ConfigError::Parse { .. }), "{err}");
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let file = TempFile::new("unknown", "source: omdb\ntvdb: { api_key: xyz }\n");
        assert!(Config::load(Some(file.path())).is_ok());
    }

    #[test]
    fn api_key_precedence_is_cli_then_env_then_config() {
        assert_eq!(
            resolve_api_key(Some("cli"), Some("env"), Some("config")),
            Some("cli".to_string())
        );
        assert_eq!(
            resolve_api_key(None, Some("env"), Some("config")),
            Some("env".to_string())
        );
        assert_eq!(
            resolve_api_key(None, None, Some("config")),
            Some("config".to_string())
        );
        assert_eq!(resolve_api_key(Some(""), Some(""), Some("")), None);
        assert_eq!(resolve_api_key(None, None, None), None);
    }

    #[test]
    fn api_key_env_falls_back_to_legacy_name() {
        // SAFETY: these env vars are only touched by this test.
        unsafe {
            std::env::remove_var(OMDB_API_KEY_ENV);
            std::env::remove_var(OMDB_API_KEY_ENV_LEGACY);
            assert_eq!(api_key_from_env(), None);

            std::env::set_var(OMDB_API_KEY_ENV_LEGACY, "legacy-key");
            assert_eq!(api_key_from_env(), Some("legacy-key".to_string()));

            std::env::set_var(OMDB_API_KEY_ENV, "modern-key");
            assert_eq!(api_key_from_env(), Some("modern-key".to_string()));

            std::env::remove_var(OMDB_API_KEY_ENV);
            std::env::remove_var(OMDB_API_KEY_ENV_LEGACY);
        }
    }

    #[test]
    fn default_path_points_into_config_dir() {
        if let Some(path) = default_path() {
            let file_name = path.file_name().unwrap().to_string_lossy().into_owned();
            assert_eq!(file_name, "config.yaml");
            let dir_name = path
                .parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            assert_eq!(dir_name, "mntk");
        }
    }
}
