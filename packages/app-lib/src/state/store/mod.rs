//! The records the launcher keeps between runs.
//!
//! Callers name a collection and the fields they look a record up by; which
//! storage answers, and how it is asked, is this package's business alone. A
//! storage is therefore replaceable without touching a single caller, and no
//! caller names one: everything below `Handle` stays in here.
//!
//! Every collection answers the same verbs, and the set grows only when a
//! collection needs another one, so moving a caller onto this package stays
//! mechanical and a later storage only has to answer the same questions.

pub(crate) mod join_log;

use sqlx::SqlitePool;

/// What a caller passes on instead of a storage: a handle on the launcher's
/// records, created once at startup by whatever provides them today.
#[derive(Clone)]
pub(crate) struct Handle {
    pool: SqlitePool,
}

impl Handle {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub(in crate::state::store) fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

/// A collection of records, and the fields it is looked up by.
pub(crate) trait Collection {
    /// The fields one record is identified by.
    type Key;

    /// The narrower set of fields a group of records is read by.
    type Scope;

    /// What one record carries beyond its key.
    type Record;

    /// Writes `record` under `key`, replacing what that key held.
    async fn store(
        handle: &Handle,
        key: &Self::Key,
        record: &Self::Record,
    ) -> crate::Result<()>;

    /// Every record of one scope, keyed by what identifies it.
    async fn list(
        handle: &Handle,
        scope: &Self::Scope,
    ) -> crate::Result<Vec<(Self::Key, Self::Record)>>;
}
