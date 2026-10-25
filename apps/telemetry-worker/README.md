# Axolotl telemetry worker

This Worker accepts only opted-in launcher heartbeat batches. It stores anonymous usage metadata in D1; error reports and error context are not accepted or persisted.

## Provisioning

```sh
pnpm exec wrangler d1 create axolotl-telemetry
pnpm exec wrangler secret put INSTALLATION_HMAC_SECRET
pnpm exec wrangler d1 migrations apply axolotl-telemetry --remote
```

Replace `database_id` in `wrangler.toml` and keep the Worker on the Free plan. The production custom domain is declared in `wrangler.toml`. Configure Cloudflare usage notifications at 50%, 75%, and 90% for Workers and D1. Do not enable Workers Paid.

Clients that still have queued error reports receive `400 error_reporting_disabled`; the payload is not logged or stored. Upgrade clients should discard their local error outbox.

The schema-version-1 heartbeat envelope and success response stay compatible with existing launchers. Unknown event types, mixed heartbeat/error batches, and extra fields (including error context and download statistics) are rejected before accessing D1. Heartbeat dates must be real UTC dates matching `occurred_at`, between today and 35 days ago inclusive. App platform/architecture strings remain extensible for future launcher builds.

## Upgrading an existing deployment

Pause ingestion during the migration/deployment window, then apply all pending forward migrations before deploying this Worker:

```sh
pnpm exec wrangler d1 migrations list axolotl-telemetry --remote
pnpm exec wrangler d1 migrations apply axolotl-telemetry --remote
pnpm deploy
pnpm configure:rate-limit
```

Resume ingestion after the new deployment is active. Confirm the custom domain points to that deployment and that a legacy error payload receives `error_reporting_disabled`; a healthy `/health` response alone cannot establish this. Migration `0005` clears historical error data. Migration `0006` adds nullable batch ownership and write-budget columns without rebuilding usage tables or changing existing client payloads. Existing usage rows, legacy batch IDs, and readers/writers naming the original columns remain compatible. Do not deploy the new ingestion code against a database missing `0006`.

The rate-limit script requires `CF_API_TOKEN` and `CF_ZONE_ID`. It creates or patches only the telemetry rule, using Free-zone-compatible parameters (2 POSTs per IP per 10 seconds, with a 10-second mitigation window); it never replaces the entire ruleset. Review shared-IP traffic when tuning the threshold. Workers and zone/WAF plans are separate: other existing rules may exhaust the zone's available rule slots, in which case the API error must be resolved rather than deleting unrelated rules.

## Storage design

D1 Free has daily limits of 5M rows read / 100k rows written. Ingestion minimizes repeated work and reserves a conservative write budget:

- Heartbeats are deduplicated by installation and UTC day, even if an older client rebatches them with new event/batch IDs. Already recorded days receive the existing success shape with `duplicate: true`, without storing another accepted batch or charging quota. A mixed batch persists only missing days.
- Batch acknowledgement, installation/global quota counters, write reservations, usage rows, and rollups commit in one D1 transaction. A failed statement rolls everything back, so retries cannot acknowledge an orphan reservation. Transactional checks also handle simultaneous duplicate requests and the final available quota slot without rejected writes.
- Requests rejected for installation/global/write quotas do not INSERT, UPDATE, or DELETE reservation rows. Duplicate and rejected traffic still incurs reads and Worker requests, so edge rate limiting and usage monitoring remain necessary.
- `MAX_RESERVED_WRITES_PER_DAY` defaults to 60,000 and is capped at 80,000. Each mutating batch reserves `8 + 8 * missing_day_count` rows, covering ingestion indexes and rollup-trigger writes. This leaves headroom for cleanup and other database activity. Reservations are conservative estimates, not Cloudflare billing counters or an account-wide guarantee; inspect actual `meta.rows_written`/D1 metrics and lower the budget if other consumers or maintenance need more headroom. Existing pre-migration batches are initially estimated at 16 rows each; check real usage when upgrading mid-day.
- `daily_active` inserts feed `wau_seen` / `mau_seen` / `daily_active_dims` through a single lightweight trigger for O(1) WAU/MAU and distribution reads.
- Offline dates outside the current 7/30-day windows do not write into the WAU/MAU sets. A nightly cron (`runMaintenance`) refreshes the bounded 35-day history to include late heartbeats, updating totals only when their values change. It rolls the active windows and applies retention, including removing legacy future-dated usage rows. Legacy error tables remain in the schema for migration compatibility but are no longer written.

Usage-data retention remains 35 days for per-installation daily activity, 8 days for batch/quota records, and 365 days for distribution aggregates. Installation identities and platform labels remain lifetime data; the daily write budget is not a total storage ceiling. Monitor database size and label/identity cardinality as the installation base grows. Legacy R2 error objects need a separate bucket lifecycle/cleanup policy; D1 error cleanup does not remove them.

Run `pnpm test` for Worker/D1, upgrade, concurrency, write-count, and mocked rate-limit-script regression tests; run `pnpm lint` for formatting and TypeScript checks. Tests use local isolated databases and mocked Cloudflare rule APIs.

Schema changes are forward-only migrations in `migrations/`; never edit an applied migration.
