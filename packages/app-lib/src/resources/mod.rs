//! Provider-neutral resource queries with independent, rebuildable caching.

pub use content_management::resources::*;

pub mod cache;
pub mod catalog;
pub mod favorites;
pub mod identity;
pub mod plans;
pub mod providers;
pub mod search;
mod service;

pub(crate) use service::valid_artifact;
pub use service::{CachePolicy, QueryOutcome, ResourceService};

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ResourceError {
    #[error("resource cursor expired; restart the query")]
    CursorExpired,
    #[error("unsupported resource operation: {0}")]
    Unsupported(String),
    #[error("invalid resource request: {0}")]
    InvalidRequest(String),
    #[error("invalid source response: {0}")]
    InvalidResponse(String),
    #[error("resource source is unavailable")]
    Unavailable,
    #[error("resource request failed (HTTP status {status:?})")]
    Network { status: Option<u16> },
    #[error("resource request canceled")]
    Canceled,
    #[error("resource cache unavailable: {0}")]
    Cache(String),
    #[error("resource database unavailable: {0}")]
    Database(String),
}

pub type ResourceResult<T> = Result<T, ResourceError>;

#[cfg(test)]
mod tests;

impl From<InvalidResourceIdentifier> for ResourceError {
    fn from(error: InvalidResourceIdentifier) -> Self {
        Self::InvalidResponse(error.to_string())
    }
}

impl From<serde_json::Error> for ResourceError {
    fn from(_: serde_json::Error) -> Self {
        Self::InvalidResponse("unexpected JSON schema".into())
    }
}

impl From<crate::Error> for ResourceError {
    fn from(error: crate::Error) -> Self {
        let status = match &*error.raw {
            crate::ErrorKind::FetchError(error) => {
                error.status().map(|status| status.as_u16())
            }
            crate::ErrorKind::HttpError { status, .. } => Some(*status),
            _ => None,
        };
        Self::Network { status }
    }
}
