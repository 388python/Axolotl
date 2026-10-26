//! When each server of an instance was last joined.

use chrono::{DateTime, TimeZone, Utc};

use super::{Collection, Handle};

/// The server join log, kept per instance and server address.
pub(crate) struct JoinLog;

/// One record of the log, identified by the instance and the server joined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JoinLogKey {
    pub instance_id: String,
    pub host: String,
    pub port: u16,
}

/// Every server of one instance.
pub(crate) struct InstanceJoins {
    pub instance_id: String,
}

/// What a join left behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct JoinLogEntry {
    pub join_time: DateTime<Utc>,
}

impl Collection for JoinLog {
    type Key = JoinLogKey;
    type Scope = InstanceJoins;
    type Record = JoinLogEntry;

    async fn store(
        handle: &Handle,
        key: &Self::Key,
        record: &Self::Record,
    ) -> crate::Result<()> {
        sqlx::query(
            "
            INSERT INTO join_log (instance_id, host, port, join_time)
            VALUES (?, ?, ?, ?)
            ON CONFLICT (instance_id, host, port) DO UPDATE SET
                join_time = excluded.join_time
            ",
        )
        .bind(&key.instance_id)
        .bind(&key.host)
        .bind(i64::from(key.port))
        .bind(record.join_time.timestamp())
        .execute(handle.pool())
        .await?;

        Ok(())
    }

    async fn list(
        handle: &Handle,
        scope: &Self::Scope,
    ) -> crate::Result<Vec<(Self::Key, Self::Record)>> {
        let rows: Vec<(String, i64, i64)> = sqlx::query_as(
            "
            SELECT host, port, join_time
            FROM join_log
            WHERE instance_id = ?
            ",
        )
        .bind(&scope.instance_id)
        .fetch_all(handle.pool())
        .await?;

        Ok(rows
            .into_iter()
            .filter_map(|(host, port, join_time)| {
                let Some(port) = u16::try_from(port).ok() else {
                    tracing::warn!(
                        %host,
                        port,
                        "Ignoring a join log entry whose port is out of range"
                    );
                    return None;
                };
                Some((
                    JoinLogKey {
                        instance_id: scope.instance_id.clone(),
                        host,
                        port,
                    },
                    JoinLogEntry {
                        join_time: to_time(join_time),
                    },
                ))
            })
            .collect())
    }
}

fn to_time(timestamp: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(timestamp, 0)
        .single()
        .unwrap_or_else(Utc::now)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A log of its own, with the instances the log refers to already there.
    async fn handle(instances: &[&str]) -> Handle {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        for instance_id in instances {
            sqlx::query(
                "INSERT INTO instances
                    (id, path, install_stage, launcher_feature_version,
                     name, created, modified)
                 VALUES (?, ?, 'installed', '1', ?, 1, 1)",
            )
            .bind(instance_id)
            .bind(instance_id)
            .bind(instance_id)
            .execute(&pool)
            .await
            .unwrap();
        }
        Handle::new(pool)
    }

    fn key(instance_id: &str, host: &str, port: u16) -> JoinLogKey {
        JoinLogKey {
            instance_id: instance_id.to_string(),
            host: host.to_string(),
            port,
        }
    }

    #[tokio::test]
    async fn a_join_replaces_the_time_the_address_held() {
        let handle = handle(&["instance"]).await;
        let address = key("instance", "mc.example.com", 25565);

        JoinLog::store(
            &handle,
            &address,
            &JoinLogEntry {
                join_time: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            },
        )
        .await
        .unwrap();
        JoinLog::store(
            &handle,
            &address,
            &JoinLogEntry {
                join_time: Utc.timestamp_opt(1_700_000_600, 0).unwrap(),
            },
        )
        .await
        .unwrap();

        let joins = JoinLog::list(
            &handle,
            &InstanceJoins {
                instance_id: "instance".to_string(),
            },
        )
        .await
        .unwrap();

        assert_eq!(
            joins,
            vec![(
                address,
                JoinLogEntry {
                    join_time: Utc.timestamp_opt(1_700_000_600, 0).unwrap(),
                },
            )]
        );
    }

    #[tokio::test]
    async fn a_scope_reads_only_its_own_instance() {
        let handle = handle(&["first", "second"]).await;
        for (instance_id, host) in
            [("first", "mc.example.com"), ("second", "mc.example.org")]
        {
            JoinLog::store(
                &handle,
                &key(instance_id, host, 25565),
                &JoinLogEntry {
                    join_time: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
                },
            )
            .await
            .unwrap();
        }

        let joins = JoinLog::list(
            &handle,
            &InstanceJoins {
                instance_id: "second".to_string(),
            },
        )
        .await
        .unwrap();

        assert_eq!(joins.len(), 1);
        assert_eq!(joins[0].0.host, "mc.example.org");
        assert_eq!(joins[0].0.instance_id, "second");
    }

    #[tokio::test]
    async fn a_port_outside_the_range_is_ignored() {
        let handle = handle(&["instance"]).await;
        JoinLog::store(
            &handle,
            &key("instance", "mc.example.com", 25565),
            &JoinLogEntry {
                join_time: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            },
        )
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO join_log (instance_id, host, port, join_time)
             VALUES ('instance', 'mc.example.org', 70000, 1700000000)",
        )
        .execute(handle.pool())
        .await
        .unwrap();

        let joins = JoinLog::list(
            &handle,
            &InstanceJoins {
                instance_id: "instance".to_string(),
            },
        )
        .await
        .unwrap();

        assert_eq!(joins.len(), 1);
        assert_eq!(joins[0].0.host, "mc.example.com");
        assert_eq!(joins[0].0.port, 25565);
    }
}
