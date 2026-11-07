//! Shared HTTP client settings and error formatting for the TMDB, image, and fanart.tv clients.

use anyhow::{Context, Result};
use reqwest::Client;
use std::error::Error;
use std::time::Duration;

/// Identifies warmup-rs to TMDB and fanart.tv in every request.
pub const USER_AGENT: &str = concat!(
    "warmup-rs/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/Appz4Fun/tmdbhelper)"
);

/// How long an unused connection stays in the pool. The CDNs close idle connections on
/// their side, and the first request sent over one of those fails with "error sending
/// request", so connections are dropped before that happens.
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(20);

/// Builds a client with the shared timeout, pool, and user agent settings.
pub fn client(what: &str) -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(50)
        .pool_idle_timeout(POOL_IDLE_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .with_context(|| format!("build {what} HTTP client"))
}

/// Formats an error with its chain of causes, joined by ": ". reqwest's own message for a
/// failed request is only "error sending request", and the cause (a reset connection, a
/// timeout, a DNS failure) is in the source chain.
pub fn error_chain(err: &(dyn Error + 'static)) -> String {
    let mut out = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !out.ends_with(&text) {
            out.push_str(": ");
            out.push_str(&text);
        }
        source = cause.source();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt;

    #[derive(Debug)]
    struct Layer(&'static str, Option<Box<Layer>>);

    impl fmt::Display for Layer {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.0)
        }
    }

    impl Error for Layer {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            self.1.as_deref().map(|e| e as &(dyn Error + 'static))
        }
    }

    #[test]
    fn error_chain_joins_every_cause() {
        let err = Layer(
            "error sending request",
            Some(Box::new(Layer(
                "client error (SendRequest)",
                Some(Box::new(Layer("connection reset by peer", None))),
            ))),
        );
        assert_eq!(
            error_chain(&err),
            "error sending request: client error (SendRequest): connection reset by peer"
        );
    }

    #[test]
    fn error_chain_skips_a_cause_already_in_the_message() {
        let err = Layer(
            "operation timed out",
            Some(Box::new(Layer("timed out", None))),
        );
        assert_eq!(error_chain(&err), "operation timed out");
    }

    #[test]
    fn user_agent_names_the_program_and_version() {
        assert!(USER_AGENT.starts_with(&format!("warmup-rs/{}", env!("CARGO_PKG_VERSION"))));
    }
}
