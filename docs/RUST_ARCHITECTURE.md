# Rust Reference Architecture

This document defines a concrete Rust implementation shape for Shortcut Forge.
It supplements the generic package docs; `SPEC.md` remains the source of truth for behavior and API.

## Goals

- Keep the service generic and caller-agnostic.
- Use a small Rust stack that is easy to run on macOS.
- Preserve the P0 synchronous build flow from `SPEC.md`.
- Make idempotency, storage durability, and source secrecy first-class concerns.

## Recommended Stack

| Area | Choice | Why |
|---|---|---|
| Language | Rust stable | Strong typing, good process control, low runtime overhead |
| HTTP | `axum` + `hyper` | Clean routing, extraction, and response handling |
| Async runtime | `tokio` | Required for HTTP, file IO, timeouts, and child processes |
| Serialization | `serde` + `serde_json` | Contract-friendly JSON handling |
| Config | `clap` + env support | Clear local/LAN startup flags matching `SPEC.md` |
| Toolchain management | `mise` | Pin the Rust toolchain, install Cherri, and expose repeatable local tasks |
| Hashing | `sha2` | Stable SHA-256 for deterministic build identity |
| Constant-time compare | `subtle` or framework equivalent | Bearer token comparison |
| Time | `time` | RFC3339 timestamps and TTL math |
| Temp files | `tempfile` | Safe per-build temp directories with cleanup |
| Random tokens | `rand` or OS CSPRNG equivalent | Unguessable download tokens |
| Logging | `tracing` + `tracing-subscriber` | Structured logs without leaking source |
| Error types | `thiserror` | Stable internal error mapping to API codes |

Pin crate versions in `Cargo.toml` during implementation; this doc intentionally focuses on architecture.

Use `mise.toml` at the repository root to pin the Rust toolchain and install Cherri from GitHub
releases for local development. Keep macOS `shortcuts` as the remaining system dependency.

Do not add a Rust runtime OpenAPI generator or middleware for P0. `docs/openapi.yaml` is maintained
as a static contract and checked separately. Re-evaluate crates such as `utoipa` or `aide` only in a
future maintenance iteration if the API grows enough to justify generated OpenAPI.

## Runtime Shape

```text
src/
  main.rs              -> parse config, init tracing, start Axum server
  app.rs               -> router construction and shared state wiring
  config.rs            -> host/port/base_url/storage/max_source_bytes/build_timeout/max_build_concurrency/auth_token
  api/
    health.rs          -> GET /health
    builds.rs          -> POST/GET /api/builds
    downloads.rs       -> GET /s/:download_token
    response.rs        -> success/error envelope helpers
  domain/
    build_id.rs        -> deterministic hash + stable 32-hex-or-longer id
    download_token.rs  -> CSPRNG token generation and validation
    metadata.rs        -> persisted build metadata model
    validation.rs      -> request validation and filename sanitization
  services/
    build_service.rs   -> idempotent orchestration entrypoint
    pipeline.rs        -> cherri compile + shortcuts sign subprocess flow
    storage.rs         -> metadata/artifact persistence and lookup
    gc.rs              -> local expired-artifact cleanup command
    storage_lock.rs    -> exclusive lock for one process per storage root
    locks.rs           -> in-memory per-build lock table and global build semaphore
  system/
    auth.rs            -> Bearer auth extraction and constant-time compare
    tools.rs           -> cherri/shortcuts health probes and cached toolchain fingerprint
    command.rs         -> safe Command wrappers with timeout handling
  tests/
    api.rs             -> endpoint behavior
    storage.rs         -> persistence/idempotency cases
```

## Core Data Model

Use the SHA-256 of `source_format + "\n" + sign_mode + "\n" + source` as the canonical build fingerprint.

- `source_hash`: full SHA-256 hex persisted in metadata
- `id`: at least the first 32 hex chars of the hash for the public build ID
- `status`: `ready`, `failed`, or computed/persisted `expired` in P0
- `error`: nullable `{ code, message }`
- `download_token`: random, at least 128 bits of entropy, not derived from the build ID or source
- `download_tokens`: active SHA-256 hashes of plaintext download tokens, each with its own expiry
- `download_token_count`: JSON API diagnostic count; token hashes are not returned by APIs
- `download_url`: generated only for `POST /api/builds` responses from current `public_base_url`
  and `/s/<download_token>`, not persisted as an absolute source-of-truth value
- `toolchain`: Cherri version/probe, Shortcuts sign probe, and a stable fingerprint

Recommended storage layout:

```text
data/
  builds/
    6f/
      6f1e4a9c2b3d0e771122334455667788/
        metadata.json
        artifact.shortcut
  downloads/
    <sha256-download-token>.json -> build id mapping, or equivalent rebuildable index
```

Using the first byte of the ID as a shard keeps directory fan-out low while preserving opaque IDs.
Treat `metadata.json` as the source of truth. If the auxiliary `downloads/` index is missing or
stale, rebuild it at startup by scanning metadata.

Acquire an exclusive lock under the storage root, for example `data/.lock`, before accepting
requests. If another process owns the lock, fail startup rather than serving concurrently against the
same storage directory.

## Request Handling Flow

```text
POST /api/builds
  -> validate Bearer auth before reading/parsing body
  -> Axum JSON extractor with body limit
  -> validate fields and UTF-8 byte size
  -> compute source_hash + id
  -> acquire per-id async lock
  -> if ready artifact exists but toolchain fingerprint changed, rebuild/sign under same id
  -> if ready artifact exists, refresh mutable metadata, expires_at, and active download token hashes, then return new tokenized download URL
  -> create temp dir
  -> write source.cherri into temp dir
  -> run cherri --skip-sign
  -> run shortcuts sign --mode anyone
  -> atomically persist artifact + metadata
  -> release lock
  -> return { id, download_url, expires_at }
```

`GET /api/builds/:id` reads `metadata.json`.

`GET /api/builds/:id` requires valid Bearer auth.

If `expires_at` is in the past, `GET /api/builds/:id` still returns 200 with `status == "expired"`.

`GET /s/:download_token` does not require Bearer auth. It serves `artifact.shortcut` only when the
token maps to metadata, `status == "ready"`, and `expires_at` is still in the future.

`GET /health` returns basic liveness anonymously. With a valid Bearer token it may include Cherri and
Shortcuts tool availability. With an invalid `Authorization` header it returns `UNAUTHORIZED`.

P0 must not route `DELETE /api/builds/:id`. Manual cleanup is performed by deleting files under the
configured storage directory while the operator controls the service.

## Concurrency Model

P0 can stay single-process and still avoid duplicate work:

1. Use a shared `tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>` or equivalent lock table keyed by build ID.
2. Lock before checking storage, not after.
3. Re-check persisted state inside the lock.
4. Use a small global semaphore, default 1, around actual compile/sign execution so different
   source requests cannot spawn unbounded tool processes.
5. Only one identical request runs the toolchain; followers reuse the ready result.

Expose the global semaphore size as `max_build_concurrency`; default it to 1 for P0.

This avoids cross-request races without introducing a database or OS-level file locks. If a future
version needs multi-process safety, file locks can be added under `services/storage.rs` without
changing the public API.

## Build Pipeline Design

Use `tokio::process::Command` with explicit argument arrays:

```text
cherri <source-file> --skip-sign --output=<unsigned-shortcut>
shortcuts sign --mode anyone --input <unsigned-shortcut> --output <signed-shortcut>
```

Key rules:

- Never invoke through `sh -c`.
- With this repository's pinned `github:electrikmilk/cherri = 2.3.0`, use the shown `--output=...`
  form. Only verify/adapt the exact output flag if the operator intentionally overrides the pinned
  Cherri version.
- Apply `tokio::time::timeout` separately to compile and sign phases.
- On timeout, kill the child process and, where supported, its process group.
- Capture stdout/stderr for diagnostics, but never include raw source in logs, API errors, or
  persisted metadata. Compiler output may echo source lines and must be sanitized before storage.
- Map non-zero exit from `cherri` to `BUILD_FAILED`.
- Map non-zero exit from `shortcuts sign` to `SIGN_FAILED`.
- On failure, persist only sanitized metadata plus phase-specific error text.

## Persistence Strategy

Write metadata with atomic replacement:

1. Build outputs land in a temp dir.
2. Signed shortcut is moved into the final build directory.
3. Metadata is written to `metadata.json.tmp`.
4. `rename()` swaps the temp metadata file into place.

If the process crashes before metadata rename, the next request recomputes the same ID and can safely
repair or rebuild that directory.

Metadata should contain:

- `id`
- `name`
- `source_format`
- `source_hash`
- `sign_mode`
- `status`
- `download_tokens`
- `download_token_count` in API responses only
- `toolchain`
- `created_at`
- `updated_at`
- `expires_at`
- `error`

Do not persist raw `source`.

Do not persist an absolute `download_url`; generate it for `POST /api/builds` from current
configuration and a fresh plaintext download token in the response so local and LAN mode do not
return stale URLs. `GET /api/builds/:id` should return metadata with `download_url: null`; callers
that need a fresh URL should re-post the same source.

Do not persist plaintext download tokens. Persist only active `download_tokens` as hashes. A repeated
same-source `POST /api/builds` rotates the tokenized `download_url` while keeping the deterministic
build ID. Keep older unexpired token hashes active so existing QR codes continue to work within their
TTL. Callers should still treat the latest response as authoritative for QR/manifest state.

Do not return token hashes from JSON APIs. Return `download_token_count` instead.

`created_at` is immutable for a deterministic build ID. Update `updated_at` on metadata/artifact
changes and refresh `expires_at` from the latest successful `POST /api/builds`.

Prune expired token hashes opportunistically during POST, startup index rebuild, or local GC.

Do not automatically delete expired artifacts in P0. Expired artifacts remain available for renewal
by a repeated same-source `POST /api/builds`; `/s/:download_token` still returns 404 until renewal.

Record the toolchain fingerprint used for the artifact. If current Cherri/Shortcuts probe output
differs from stored metadata, rebuild and re-sign under the same build ID.

For durability, write artifact and metadata temp files, flush file contents, atomically rename into
place, and flush parent directories where the platform supports it. Keep this local-filesystem only;
network filesystems are out of scope for P0.

## Configuration Surface

Expose configuration by CLI flag plus env override:

| Config | CLI | Env |
|---|---|---|
| host | `--host` | `SHORTCUT_SERVER_HOST` |
| port | `--port` | `SHORTCUT_SERVER_PORT` |
| public_base_url | `--public-base-url` | `SHORTCUT_SERVER_PUBLIC_BASE_URL` |
| storage | `--storage` | `SHORTCUT_SERVER_STORAGE` |
| max_source_bytes | `--max-source-bytes` | `SHORTCUT_SERVER_MAX_SOURCE_BYTES` |
| build_timeout | `--build-timeout-seconds` | `SHORTCUT_SERVER_BUILD_TIMEOUT_SECONDS` |
| max_build_concurrency | `--max-build-concurrency` | `SHORTCUT_SERVER_MAX_BUILD_CONCURRENCY` |
| auth_token | Prefer env only | `SHORTCUT_SERVER_AUTH_TOKEN` |
| health_cache_ttl | `--health-cache-ttl-seconds` | `SHORTCUT_SERVER_HEALTH_CACHE_TTL_SECONDS` |

Defaults should mirror `SPEC.md`. `auth_token` has no default; refuse to start unless configured.

## Logging and Observability

Emit structured logs with:

- request method
- route pattern, not raw token-bearing path
- response status
- build ID when available
- phase (`validate`, `compile`, `sign`, `persist`)
- tool exit status / timeout classification

Never log:

- request `source`
- full request body
- raw shortcut bytes
- `Authorization` headers
- service auth tokens
- download tokens

Detailed health/tool probes should be cached for the configured TTL, default 60 seconds.

## Security Boundaries

- `shortcuts sign` sends shortcut content to Apple for validation. P0 accepts this boundary, but
  callers should embed only secrets suitable for inclusion in a signed shortcut, preferably
  short-lived or revocable tokens.
- Require Bearer auth for `POST /api/builds` and `GET /api/builds/:id`.
- Authenticate protected routes before reading or parsing request bodies.
- Keep `/health` anonymous for basic liveness only; detailed health requires valid Bearer auth.
- Generate download tokens with a CSPRNG and at least 128 bits of entropy.
- Treat `/s/:download_token` path tokens as credentials; do not log them.
- Persist download token hashes only.
- Keep older unexpired download token hashes active.
- Enforce request body size in the HTTP layer and again in validation.
- Accept only lowercase hex build IDs in path params.
- Use a fixed public build ID length of at least 32 lowercase hex characters.
- Validate download tokens as URL-safe service-generated tokens, recommended format `dl_` plus
  base64url without padding and at least 128 bits of entropy.
- Return 404 for malformed build IDs or download tokens without storage lookup.
- Sanitize `name` before forming `Content-Disposition` filenames by stripping or replacing line
  breaks, quotes, backslashes, path separators, and semicolons.
- Use temp dirs under the configured storage root or system temp root.
- Ensure temp dirs containing Cherri source are private to the service process, equivalent to mode
  `0700`; metadata and artifacts should be owner readable/writable only where supported.
- Remove temp dirs on both success and failure.
- Keep all command invocations argument-based.
- Treat tool output as diagnostics only; do not trust it for filesystem paths.
- Do not expose destructive cleanup/delete routes in P0.
- Limit compile/sign execution with a global semaphore; P0 default is one build at a time.
- Return `SERVER_BUSY` immediately when the global semaphore is saturated; P0 does not queue build
  requests.
- Cache detailed health probes; do not fork external tools on every health request.
- Acquire an exclusive storage lock before serving.
- Kill timed-out child processes and process groups where supported.
- Flush files and parent directories around atomic artifact/metadata renames where practical.

## Local Maintenance

Do not expose cleanup over HTTP. Implement a local maintenance subcommand or documented operator
procedure to remove expired artifacts under the configured storage root, for example:

```bash
SHORTCUT_SERVER_AUTH_TOKEN=<token> cargo run -- gc --storage ./data --expired-before 30d
```

## Testing Strategy

### Unit Tests

- request validation
- build ID stability
- safe filename generation
- metadata serialization
- error mapping
- auth extraction/comparison
- download token generation/validation
- path parameter format validation
- token hash lookup
- toolchain fingerprint comparison

### Integration Tests

- protected routes reject missing/invalid Bearer auth with `UNAUTHORIZED`
- protected routes reject unauthenticated large-body requests before body parsing
- anonymous `/health` returns basic liveness
- authenticated `/health` returns tool availability
- detailed health probes are cached
- `POST /api/builds` rejects invalid `source_format`
- same request twice returns same ID
- build ID is at least 32 hex chars
- same request with a different `ttl_seconds` refreshes `expires_at`, returns a new `download_url`,
  and does not change ID
- `GET /api/builds/:id` returns persisted metadata after restart simulation
- `GET /api/builds/:id` returns 200 with expired status after expiry
- `GET /s/:download_token` returns 404 for unknown, missing, failed, or expired builds
- download token is not equal to or derived from build ID
- plaintext download token is absent from metadata/index files
- `GET /api/builds/:id` returns `download_url: null`
- `GET /api/builds/:id` returns `download_token_count`, not token hashes
- malformed build IDs and download tokens return 404 before storage lookup
- repeated same-source POST returns a new `download_url`
- older unexpired token hashes remain valid
- same-source renewal preserves `created_at` and updates `updated_at`/`expires_at`
- download-token index rebuilds from metadata
- storage lock prevents a second process using the same storage root
- timeout terminates the child process
- changed toolchain fingerprint rebuilds under same ID
- oversized payload returns `PAYLOAD_TOO_LARGE`
- missing tool maps to `TOOL_UNAVAILABLE`
- timeout maps to `TIMEOUT`
- concurrency saturation maps to `SERVER_BUSY`
- re-posting the same source after expiry refreshes the same ID
- `DELETE /api/builds/:id` is not routed in P0

### Tooling Boundary Tests

Abstract the command runner behind a trait so tests can fake:

- successful compile/sign
- compile failure
- sign failure
- timeout

This keeps most tests runnable off-macOS while leaving one smoke path for real macOS tools.

## Suggested Commands

Use these as the implementation baseline once the Rust service exists:

```bash
mise trust
mise install
bash scripts/check-openapi.sh
cargo test
SHORTCUT_SERVER_AUTH_TOKEN=<token> cargo run -- --host 127.0.0.1 --port 8787
SHORTCUT_SERVER_AUTH_TOKEN=<token> cargo run -- --host 0.0.0.0 --port 8787 --public-base-url http://<mac-lan-host>:8787
```

## Why This Shape Fits The Package

- It preserves the package's generic contract instead of mixing in Tsugi concepts.
- It gives the Mac-side agent a concrete Rust path without rewriting `SPEC.md`.
- It keeps OpenAPI as a static downstream contract instead of coupling HTTP handlers to a runtime
  OpenAPI generation layer.
- It keeps the operational surface small enough for a P0 local service while leaving room for future
  upgrades like async workers, stronger diagnostics, or multi-process-safe storage.
