//! Modrinth file endpoint selection, independent of URL routing and DNS truth.

mod candidates;
mod clients;
mod coordinator;
mod fastly_rules;
pub(crate) mod metrics;
mod model;
mod policy;
mod probe;

pub(crate) use coordinator::{EndpointSelector, SelectionMode};
pub(crate) use model::{EndpointLease, PoolKey, Protocol};

#[cfg(test)]
mod tests;
