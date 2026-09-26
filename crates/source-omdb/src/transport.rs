//! The HTTP seam.
//!
//! [`UreqTransport`] is the only thing in this crate that touches the
//! network. Everything above it deals in plain strings, so the provider is
//! fully testable with an in-memory transport.
//!
//! `Transport` is deliberately not `Send + Sync`: v1 usage is
//! single-threaded, and the in-memory test transports are simpler without
//! the bound.

use std::time::Duration;

/// A raw HTTP response from a metadata source.
#[derive(Debug, Clone)]
pub struct TransportResponse {
    /// The HTTP status code.
    pub status: u16,
    /// The decoded response body.
    pub body: String,
}

/// A transport-level failure: the request never got an HTTP response (DNS,
/// connection refused, timeout, TLS...).
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct TransportError(pub String);

/// Sends GET requests and returns raw responses.
pub trait Transport {
    fn get(&self, url: &str) -> Result<TransportResponse, TransportError>;
}

/// Total time budget for a request (connect, send, and body).
const TIMEOUT: Duration = Duration::from_secs(30);

/// The production transport, backed by ureq (rustls by default).
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    pub fn new() -> Self {
        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .timeout_global(Some(TIMEOUT))
                .build(),
        );
        Self { agent }
    }
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport for UreqTransport {
    fn get(&self, url: &str) -> Result<TransportResponse, TransportError> {
        match self.agent.get(url).call() {
            Ok(response) => {
                let status = response.status().as_u16();
                let body = response
                    .into_body()
                    .read_to_string()
                    .map_err(|error| TransportError(error.to_string()))?;
                Ok(TransportResponse { status, body })
            }
            // ureq turns non-2xx statuses into Err(StatusCode) by default;
            // hand them back as responses so callers can react.
            Err(ureq::Error::StatusCode(status)) => Ok(TransportResponse {
                status,
                body: String::new(),
            }),
            Err(error) => Err(TransportError(error.to_string())),
        }
    }
}
