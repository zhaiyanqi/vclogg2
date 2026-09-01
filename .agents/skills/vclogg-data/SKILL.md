---
name: vclogg-data
description: Navigate and modify VCLogg2 SQLite persistence, cache lifecycle, persisted path encoding, and storage query records. Use for work in crates/vclogg-data or migration out of app state_store; do not use for search algorithms or GPUI interaction state.
---

# VCLogg2 Data

Data owns durable state and cache lifecycle without depending on GPUI or the app crate. Read [doc/architecture.md](../../../doc/architecture.md) before changing schemas or cross-layer records.

## Route the change

- `index_cache.rs`: enumerate, age, bound and clear managed on-disk index entries. The index file format and index construction remain core behavior.
- `path_codec.rs`: lossless platform-aware path keys for persistence, including non-Unicode paths.
- `state.rs`: stable DTOs returned by persistence repositories.
- `state_repository.rs`: SQLite access for recent files, pinning, history protection/deletion, workspace recovery queries and database statistics.
- `crates/vclogg-app/src/state_store.rs`: transitional adapter for presentation-specific payload encoding and the remaining settings/session SQL; do not add new persistence responsibilities there.

SQLite schema, transactions, conflict-safe writes, cached data, persisted settings and recovery payload storage belong here. UI models may encode/decode DTO payloads in app, but data must not import GPUI types or app modules.

## Preserve these invariants

- Keep data operations safe for background execution and never retain a GPUI context or entity.
- Preserve lossless native path identity and accept existing legacy encodings.
- Revalidate cache entry identity before deletion; never delete an entry from a stale directory snapshot.
- Schema migrations are idempotent and forward-version rejection is explicit.

## Verify

Run `cargo test -p vclogg-data` plus the state-store tests affected by a migration. For schema/session work also run `cargo test -p vclogg2 state_store`. Finish with `bash scripts/check-architecture.sh` and the workspace checks.
