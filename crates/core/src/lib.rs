//! Core library for mntk.
//!
//! Provides the source-agnostic building blocks shared by all mntk tools:
//! the metadata source abstraction, file naming rules, and configuration
//! loading.

pub mod config;
pub mod source;

pub use source::{MovieSearchResult, MovieSource, SourceError, SourceId};
