# Rust Implementation Notes

This document describes the current Rust implementation. `docs/SPEC.md` remains the
stack-neutral behavior and API source of truth.

## Stack

| Area | Current choice |
|---|---|
| Language | Rust 2024 |
| Dependencies | `axum`, `tokio`, `serde`, `serde_json`, `clap`, `toml`, `thiserror`, `anyhow`, `sha2`, `base64` |
| HTTP | `axum` router on `tokio::net::TcpListener` |
| Concurrency | Async tasks with `tokio::sync::Mutex` / `Semaphore` |
| JSON | `serde` + `serde_json` |
| Config | TOML file deserialized via `serde` |
| CLI | `clap` derive API |
| Error handling | `thiserror` for library errors, `anyhow` at boundaries |
| Toolchain management | `mise.toml` pins Rust stable and Cherri 2.3.0 |
| Build tools | `cherri` and macOS `shortcuts sign` invoked with structured args |
| Storage | Local files under the configured storage directory |

`Cargo.toml` was originally std-only for P0. The current stack adds crates only where they replace
hand-rolled infrastructure (parsers, config, CLI) or where the runtime model benefits from an async
framework (HTTP routing, middleware).

## Runtime Commands

Happy path:

```bash
cargo run -- init
cargo run -- start
cargo run -- smoke
```

Low-level server:

```bash
mise run run-local -- --config "$HOME/Library/Application Support/ShortcutForge/shortcut-forge.toml"
```

Local cleanup:

```bash
cargo run -- gc --config "$HOME/Library/Application Support/ShortcutForge/shortcut-forge.toml"
```

Other operator commands:

```bash
cargo run -- status
cargo run -- doctor
cargo run -- logs --lines 80
cargo run -- config show
cargo run -- token rotate
cargo run -- build docs/examples/minimal-request.json
```

## Configuration

| Config | CLI | Config file key | Env | Default |
|---|---|---|---|---|
| config file | `--config` | n/a | `SHORTCUT_SERVER_CONFIG` | none |
| host | `--host` | `host` | `SHORTCUT_SERVER_HOST` | `127.0.0.1` |
| port | `--port` | `port` | `SHORTCUT_SERVER_PORT` | `8787` |
| public base URL | `--public-base-url` | `public_base_url` | `SHORTCUT_SERVER_PUBLIC_BASE_URL` | `http://127.0.0.1:<port>` |
| storage | `--storage` | `storage` | `SHORTCUT_SERVER_STORAGE` | `./data` |
| max source bytes | `--max-source-bytes` | `max_source_bytes` | `SHORTCUT_SERVER_MAX_SOURCE_BYTES` | `524288` |
| build timeout | `--build-timeout-seconds` | `build_timeout_seconds` | `SHORTCUT_SERVER_BUILD_TIMEOUT_SECONDS` | `30` |
| max build concurrency | `--max-build-concurrency` | `max_build_concurrency` | `SHORTCUT_SERVER_MAX_BUILD_CONCURRENCY` | `1` |
| auth token | `--auth-token` | `auth_token` | `SHORTCUT_SERVER_AUTH_TOKEN` | required |
| health cache TTL | `--health-cache-ttl-seconds` | `health_cache_ttl_seconds` | `SHORTCUT_SERVER_HEALTH_CACHE_TTL_SECONDS` | `60` |
| Cherri binary | `--cherri-bin` | `cherri_bin` | `SHORTCUT_SERVER_CHERRI_BIN` | `cherri` |
| Shortcuts binary | `--shortcuts-bin` | `shortcuts_bin` | `SHORTCUT_SERVER_SHORTCUTS_BIN` | `shortcuts` |

Precedence is CLI flags, then environment variables, then config file, then defaults.
`SERVER_AUTH_TOKEN` is also accepted by the binary for local smoke-test compatibility.

## Source Layout

Modules are split by responsibility:

```text
src/main.rs
  async entry point (tokio::main) and module declarations

src/cli.rs
  clap derive definitions for all commands, flags, and help text

src/config.rs
  TOML loading, env mapping, Config deserialization, defaults

src/error.rs
  thiserror error enums and anyhow boundary conversions

src/model.rs
  shared serde structs: BuildRequest, BuildResponse, BuildMetadata, etc.

src/http.rs
  axum Router, middleware (auth, body limit), and server startup

src/api.rs
  async route handlers: health, build, metadata, download

src/state.rs
  AppState: Config, build-slot Semaphore, build-lock table

src/build.rs
  temp-dir creation, Cherri invocation, shortcuts sign, metadata write

src/store.rs
  scan_metadata, run_gc, token hashing, storage layout

src/operator.rs
  init, doctor, start/stop/restart/status/logs, LaunchAgent plist
```

`src/json.rs` has been removed; all JSON handling uses `serde_json`.

## Build Identity

The canonical fingerprint is:

```text
source_format + "\n" + sign_mode + "\n" + source
```

The public build ID is the first 32 lowercase hex characters of the SHA-256 fingerprint. Metadata
stores the full `source_hash`; if a truncated ID collision is detected, the service returns
`INTERNAL_ERROR` and does not overwrite unrelated data.

Repeated same-source `POST /api/builds` requests reuse the same ID. If the artifact exists and the
toolchain fingerprint is unchanged, the server refreshes mutable metadata and returns a new
download URL without recompiling.

## Storage

Default storage root: `./data`.

Current layout:

```text
data/
  .lock
  builds/
    <first-two-id-chars>/
      <build-id>/
        artifact.shortcut
        metadata.json
  tmp/
    build-<random>/
```

`metadata.json` is the source of truth. It stores:

- build ID and full source hash
- display name
- source format and sign mode
- ready/failed status
- hashed download tokens with expirations
- toolchain fingerprint
- created/updated/expires timestamps
- sanitized error metadata

It does not store raw source, plaintext download tokens, or absolute download URLs.

## Download Tokens

Download tokens use `/dev/urandom` and the visible form:

```text
dl_<base64url-no-padding>
```

The server stores only `sha256(download_token)`. `/s/<download_token>` hashes the supplied token and
scans metadata for an active match. The download route is intentionally unauthenticated because the
iPhone import flow opens that URL directly.

`GET /api/builds/<id>` never reconstructs or issues a download token; it returns
`"download_url": null` and a `download_token_count` diagnostic.

## Build Pipeline

For each rebuild:

1. Create a private temp directory under the storage root.
2. Write `source.cherri` with owner-only permissions where supported.
3. Run `cherri <source-file> --skip-sign --output=<unsigned-shortcut> --no-ansi`.
4. Discover the unsigned `.shortcut` if Cherri writes a version-specific output name.
5. Run `shortcuts sign --mode anyone --input <unsigned> --output <signed>`.
6. Copy the signed artifact into the build directory.
7. Atomically write metadata.
8. Remove temp source and temp build files.

Commands are invoked with structured argument arrays. The implementation never shells out through
`sh -c`.

Timeout handling kills the child process and, on Unix, the process group.

## HTTP Routes

- `GET /health`
  - Anonymous basic liveness.
  - Detailed Cherri/Shortcuts status when called with valid Bearer auth.
  - Invalid `Authorization` returns `UNAUTHORIZED`.
- `POST /api/builds`
  - Requires Bearer auth before body parsing.
  - Validates the JSON request and source size.
  - Builds or renews the shortcut.
- `GET /api/builds/<id>`
  - Requires Bearer auth.
  - Returns persisted metadata.
  - Expired ready builds return 200 with `status: "expired"`.
- `GET /s/<download_token>`
  - No Bearer auth.
  - Serves the signed shortcut only for active ready non-expired tokens.

Malformed build IDs and download tokens return 404 before storage lookup.

The service does not expose `DELETE /api/builds/<id>` or any destructive cleanup HTTP route.

## Concurrency

The service uses:

- A per-build in-memory lock table (`tokio::sync::Mutex<HashMap<...>>`) to prevent identical
  builds from writing the same output at the same time.
- A global build-slot `tokio::sync::Semaphore` controlled by `max_build_concurrency`.
- An exclusive storage lock at `<storage>/.lock` to prevent two server processes from serving the
  same storage root.

Default concurrency is one build at a time. When build slots are saturated, the server returns
`SERVER_BUSY` immediately; it does not queue builds behind the global limit.

## Logging and Secrecy

Logs include request method, route pattern, response status, and startup configuration. Route
patterns are used instead of raw token-bearing paths.

Never log:

- request source
- full request body
- shortcut bytes
- `Authorization` headers
- service auth tokens
- plaintext download tokens

Compiler and signer output is not persisted as raw diagnostics because those tools may echo source.

## Tests

Run:

```bash
mise run test
mise run check-openapi
```

The unit tests cover validation, JSON parsing, ID stability, token behavior, metadata
serialization, storage paths, GC, auth, and concurrency saturation. The smoke script covers the real
HTTP path and requires a running server plus working macOS build/sign tools.

## Module Boundary Justification

`src/main.rs` was split because it reached 5,456 lines / 217 functions, exceeding the threshold
where a single human can keep the entire file in working memory. The boundary lines follow the
responsibility split already present in the code:

- `cli.rs` / `config.rs` — operator-facing surface (argument parsing and configuration).
- `http.rs` / `api.rs` — request transport and route handling.
- `build.rs` — the compile-and-sign pipeline.
- `store.rs` — persistence, GC, and token resolution.
- `operator.rs` — macOS service lifecycle.

Keeping these boundaries stable means new features (new API routes, new CLI commands, new build
stages) map to a single module rather than re-expanding the monolith.

## Migration Notes

- Config files migrated from flat `key = value` to TOML. String values now require quotes.
- The blocking `TcpListener` loop was replaced by `tokio` + `axum`.
- Shared mutable state (`build_lock_table`, concurrency slot counter) uses `tokio::sync::Mutex`
  and `tokio::sync::Semaphore` because handlers hold them across await points.
- Process spawning (`cherri`, `shortcuts sign`) uses `tokio::process::Command` to avoid blocking
  the async runtime worker threads.
