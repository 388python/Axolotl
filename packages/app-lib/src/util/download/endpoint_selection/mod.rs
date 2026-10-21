//! Modrinth file endpoint selection, independent of URL routing and DNS truth.

mod candidates;
mod fastly_rules;
mod model;
mod policy;
mod probe;

#[cfg(test)]
mod tests;
