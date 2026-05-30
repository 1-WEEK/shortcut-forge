# Refactor Plan

Status: approved  
Scope: migrate from P0 zero-dependency single-file implementation to a modular, crate-based async Rust service. Business logic stays; infrastructure and runtime model change.

## Current Pain Points

1. **Single file at capacity.** `src/main.rs` is 5,456 lines with 217 functions. Adding new API routes or build pipeline stages pushes it past human working memory limits.
2. **Reinvented wheels are liabilities.** Hand-rolled HTTP/1.1 parser, JSON parser, Base64 encoder, and config file parser are each potential sources of request-smuggling bugs, JSON edge-case crashes, or parser DoS. They add ~1,500 lines that do not advance the product.
3. **Zero dependency policy has hit diminishing returns.** The original constraint made sense for a quick P0, but it now blocks safe evolution. No unit-test framework, no structured error types, and no ecosystem-hardened parsers.
4. **Code style is repetitive.** Every CLI command handler repeats the same `if let Err(err) = run_xyz(...) { eprintln!("..."); std::process::exit(1); }` block 15+ times, masking real error diversity.

## Stack After Refactor

| Area | New choice |
|---|---|
| HTTP server | `axum` on `tokio` |
| Concurrency | Async tasks with `tokio::sync::Mutex` / `Semaphore` |
| JSON | `serde` + `serde_json` |
| Config | TOML via `toml` + `serde::Deserialize` |
| CLI | `clap` derive API |
| Error handling | `thiserror` for library errors, `anyhow` at boundaries |

## What Not To Touch

- Build identity (SHA-256 fingerprint + truncated ID).
- Storage layout (`data/builds/<shard>/<id>/`).
- Download token format and hashing.
- API routes and response shapes (keep `docs/openapi.yaml` stable).
- Bearer auth behavior and logging rules.
- Cherri / `shortcuts sign` invocation arguments.
- LaunchAgent label and plist shape.

## Phases

### Phase 1: Module split + crate introduction (sync runtime)

Goal: delete hand-rolled parsers, split the monolith into modules, verify behavior before introducing async.

`Cargo.toml` additions:

```toml
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }
toml = "0.8"
thiserror = "1"
anyhow = "1"
```

Module boundaries:

- `src/cli.rs` — `clap` derive definitions for all commands and flags.
- `src/config.rs` — TOML loading, env mapping, `Config` deserialization.
- `src/error.rs` — `thiserror` enums (`BuildError`, `StoreError`, `ApiError`), `anyhow` at boundaries.
- `src/model.rs` — shared `serde` structs: `BuildRequest`, `BuildResponse`, `BuildMetadata`, etc.
- `src/api.rs` — route handlers (`health`, `build`, `metadata`, `download`), sync signatures.
- `src/build.rs` — temp-dir creation, Cherri invocation, `shortcuts sign`, metadata write.
- `src/store.rs` — `scan_metadata`, `run_gc`, token hashing, storage layout.
- `src/operator.rs` — `init`, `doctor`, `start`/`stop`/`restart`/`status`/`logs`, LaunchAgent plist.
- `src/http.rs` — blocking `TcpListener` loop kept for this phase, but isolated and cleaned up.
- `src/main.rs` — entry point and module declarations only.
- `src/json.rs` — **deleted**; all JSON replaced by `serde_json`.

Config format breaking change: old flat `key = value` (unquoted strings) is no longer supported. TOML requires quoted strings:

```toml
host = "127.0.0.1"
port = 8787
storage = "./data"
```

Verification:

```bash
cargo test
mise run check-env
mise run smoke-build
```

### Phase 2: Async runtime migration (axum + tokio)

Goal: replace blocking HTTP layer with async; move auth and limits to middleware.

`Cargo.toml` additions:

```toml
axum = "0.7"
tokio = { version = "1", features = ["rt-multi-thread", "net", "process", "macros"] }
```

Changes:

- `src/http.rs` — blocking `TcpListener` + manual threads → `tokio::net::TcpListener` + `axum::Router`.
- Auth and request-body size limits become axum middleware.
- `src/api.rs` — handlers become `async fn`; state injected via `axum::extract::State<Arc<AppState>>`.
- `src/state.rs` (new) — `AppState` with `Config`, `tokio::sync::Semaphore` for build slots, `tokio::sync::Mutex<HashMap<...>>` for build lock table.
- Process spawning moves from `std::process::Command` to `tokio::process::Command` to avoid blocking worker threads.
- `src/main.rs` — entry becomes `#[tokio::main] async fn main()`.

Verification:

```bash
cargo test
mise run check-env
mise run smoke-build
```

Plus concurrency stress test: saturate `max_build_concurrency` and confirm `SERVER_BUSY` and lock-table behavior remain correct under async.

## Rollback Plan

Each phase is independently mergeable:

- Phase 1 can be reverted by removing new crates and restoring the old hand-rolled parsers. The module split itself is low-risk and need not be reverted.
- Phase 2 can be reverted by removing `axum`/`tokio` and restoring the blocking `TcpListener` loop from Phase 1. Business logic in `src/api.rs` and `src/build.rs` stays unchanged.

## Decision Gate

Phase 1 may begin immediately. Phase 2 may begin only after:

1. Phase 1 passes `cargo test` and `mise run smoke-build` on a real Mac.
2. A second review confirms the `serde` struct shapes match the existing API contract.
