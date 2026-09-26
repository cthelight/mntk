//! OMDB metadata source for mntk.
//!
//! Implements the [`mntk_core::MovieSource`] trait against the OMDB REST API
//! (http://www.omdbapi.com). All network I/O is isolated behind the
//! [`Transport`] trait so the provider is testable without a network.

pub(crate) mod models;
pub(crate) mod transport;

mod omdb;

pub use omdb::OmdbSource;
