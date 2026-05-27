# Specification

## Goal

Provide a generic LAN-local macOS service that turns complete Cherri source into a signed
`.shortcut` file that iPhone can import.

The caller owns all business meaning. The server is a build/sign/download service only.

## Responsibility Split

### Caller

The caller is responsible for:

- Producing complete Cherri source.
- Embedding any business-specific URL, token, profile, sleep, health, WOL, or app-intent parameter.
- Deciding when a shortcut needs regeneration.
- Persisting its own caller-side manifest if needed.
- Displaying QR codes if needed.

### Shortcut Forge

The server is responsible for:

- Accepting generic source build requests.
- Validating request shape and size.
- Compiling Cherri source with the local `cherri` CLI.
- Signing compiled shortcuts with local macOS `shortcuts sign`.
- Persisting signed `.shortcut` files and metadata.
- Returning stable download URLs.
- Serving signed `.shortcut` files over HTTP.

### iPhone/User

The user is responsible for:

- Installing any third-party apps referenced by the generated shortcut, such as Scriptable or
  Wake Me Up.
- Importing the signed shortcut.
- Binding NFC automation manually if desired.

## Non-Goals

- No Tsugi-specific fields or logic.
- No shortcut source templating.
- No iCloud links.
- No QR generation in P0.
- No business-specific authentication model in P0; use generic Bearer service authentication.
- No destructive cleanup/delete HTTP endpoint in P0.
- No database service requirement.
- No prescribed implementation language, framework, or package manager.

## Baseline Technical Requirements

The implementation must:

- Run on macOS.
- Start a LAN-reachable HTTP server.
- Let the operator configure bind host, port, public base URL, storage directory, source size
  limit, build timeout, max build concurrency, and service auth token.
- Require `Authorization: Bearer <token>` for protected API endpoints.
- Invoke `cherri` without shell interpolation.
- Invoke `shortcuts sign` without shell interpolation.
- Bound request body size.
- Store signed shortcuts durably on disk.
- Avoid storing or logging raw source after a build completes.
- Store credentials defensively: persist only download token hashes, never plaintext download tokens.
- Return JSON API errors with stable codes.
- Provide a documented local run command and test command.

This specification is intentionally stack-neutral. The current repository implementation is Rust and
is documented in `docs/RUST_ARCHITECTURE.md`; keep implementation-specific details there.

## OpenAPI Contract

`docs/openapi.yaml` is the static machine-readable API contract for downstream integration,
documentation, and client generation. `SPEC.md` remains the semantic source of truth; when API
behavior changes, update both files together.

P0 does not use Rust runtime OpenAPI generation or middleware. Do not add crates such as `utoipa` or
`aide` solely to generate this small API contract at runtime. Runtime OpenAPI integration is a
backlog consideration for future maintenance only if the API surface grows enough to justify it.

## Required External Tools

- `cherri` available on `PATH`.
- macOS `shortcuts` CLI with `shortcuts sign` available.

Verify:

```bash
sw_vers
cherri --version
shortcuts help sign
```

## Process Model

```text
HTTP request
  -> validate Authorization bearer token for protected API routes
  -> validate JSON
  -> compute deterministic build id from source_format + sign_mode + source
  -> if existing signed file is ready but toolchain fingerprint changed, rebuild under the same ID
  -> if existing signed file is ready, refresh mutable metadata and expiry, add a new download token hash, then return new tokenized download URL
  -> write source to temporary build directory
  -> cherri compile with --skip-sign
  -> shortcuts sign --mode anyone
  -> atomically move signed file into storage
  -> persist metadata
  -> return download URL
```

Builds may run synchronously for P0. The implementation must prevent concurrent identical requests
from racing while writing the same output file.

## Storage Contract

The service must persist:

- Signed `.shortcut` bytes.
- Metadata for each build.
- One or more active random download token hashes for each ready build.
- Enough data to serve `GET /api/builds/<id>` and `GET /s/<download_token>` after process restart.

The storage layout is implementation-defined, but enough metadata must be persisted to return:

```json
{
  "id": "6f1e4a9c2b3d0e771122334455667788",
  "name": "Switch Profile",
  "source_format": "cherri",
  "source_hash": "sha256 hex",
  "sign_mode": "anyone",
  "status": "ready",
  "download_url": null,
  "download_token_count": 1,
  "toolchain": {
    "cherri": "Cherri Compiler v2.3.0",
    "shortcuts_sign": "available",
    "fingerprint": "sha256 hex"
  },
  "created_at": "2026-05-24T12:00:00Z",
  "updated_at": "2026-05-24T12:00:03Z",
  "expires_at": "2026-06-23T12:00:00Z",
  "error": null
}
```

Do not persist raw `source` in metadata; source can contain embedded secrets.

Do not persist an absolute `download_url` as the source of truth. Generate `POST /api/builds`
`download_url` values from the current `public_base_url` and `/s/<download_token>` so builds remain
usable when the operator switches between local and LAN mode.

Download tokens must be generated from a cryptographically secure random source with at least 128
bits of entropy. They must not be derived from the deterministic build ID or source hash.

Persist only `download_token_hash = sha256(download_token)` or stronger equivalent. Plaintext
download tokens may appear in API responses and QR/download URLs, but must not be stored in metadata,
index files, or logs. Resolve `GET /s/<download_token>` by hashing the supplied token and looking up
the hash.

Do not expose download token hashes from JSON APIs. `GET /api/builds/<id>` should return only
`download_token_count` for diagnostics.

Because plaintext download tokens are not persisted, a repeated same-source `POST /api/builds`
returns a new download URL even when the build ID is unchanged and the artifact is not recompiled.
Callers must treat the latest returned `download_url` as authoritative and update their manifest/QR
state accordingly.

The service should keep multiple active download token hashes per build until their `expires_at`, so
previously issued QR codes remain usable within their TTL without storing plaintext tokens. Expired
token hashes may be pruned opportunistically during POST, startup index rebuild, or local GC.

Use at least the first 32 hex characters of the build fingerprint as the public build ID. Collision
handling must verify the full `source_hash`; if a truncated ID collision is detected, return
`INTERNAL_ERROR` and do not overwrite an unrelated build.

Metadata must record a toolchain fingerprint containing at least the Cherri version/probe output and
Shortcuts signing probe status. In JSON responses, represent `toolchain.shortcuts_sign` as a short
string such as `"available"` or `"unavailable"` rather than a boolean. If the same source
fingerprint is submitted after the toolchain fingerprint changes, the service should rebuild and
re-sign under the same build ID.

`expires_at` is a real serving boundary. After expiration, `GET /s/<download_token>` must not serve
the signed shortcut. Expiration is a download gate, not automatic deletion. P0 does not need a
background cleanup job.

A repeated `POST /api/builds` with the same source must reuse the same deterministic ID. If the ready
artifact still exists and the toolchain fingerprint is unchanged, refresh mutable metadata,
`expires_at`, and add a new download token hash from the new request, then return without
recompiling. If the artifact is missing, rebuild from the submitted source.

API metadata status values are `ready`, `failed`, or `expired`. P0 builds run synchronously, so no
`in_progress` status is needed. Implementations may compute `expired` from `expires_at` instead of
storing it permanently.

Timestamp semantics:

- `created_at` is the first time this deterministic build ID was created and must not change on
  same-source renewal.
- `updated_at` changes whenever metadata, expiry, token hashes, artifact, or status changes.
- `expires_at` is refreshed from the latest successful `POST /api/builds`.

## Operator Configuration

The service must support equivalent configuration for:

```text
host             default: 127.0.0.1
port             default: 8787
public_base_url  default: http://127.0.0.1:<port>
storage          default: ./data or implementation equivalent
max_source_bytes default: 524288
build_timeout    default: 30 seconds
max_build_concurrency default: 1
auth_token       required; no default
health_cache_ttl default: 60 seconds
```

Configuration may come from CLI flags, environment variables, or a local config file. Long-lived
operator deployments should be able to keep the service auth token and LAN public base URL in a
permission-restricted config file instead of requiring shell environment setup on every restart.

For a caller on another LAN machine, the service must be able to bind `0.0.0.0` and return URLs
under the Mac LAN `public_base_url`.

P0 Bearer authentication over plain HTTP assumes a trusted LAN. It prevents unauthenticated callers
from invoking build APIs, but it does not protect the token against passive network capture. On
untrusted networks, run the service behind a private tunnel or transport encryption layer such as
Tailscale, WireGuard, an SSH tunnel, or an HTTPS reverse proxy.

## API

### `GET /health`

Returns service status.

This endpoint is ops-friendly: no auth is required for basic liveness. If a valid Bearer token is
provided, the response may include detailed tool availability. If an invalid `Authorization` header
is provided, return `UNAUTHORIZED`.

Anonymous response 200:

```json
{
  "ok": true,
  "data": {
    "version": "0.1.0",
    "status": "ok",
    "auth_required": true
  }
}
```

Authenticated response 200 may include:

```json
{
  "ok": true,
  "data": {
    "version": "0.1.0",
    "status": "ok",
    "auth_required": true,
    "cherri": "Cherri Compiler v2.3.0",
    "shortcuts_sign": "available",
    "cache_ttl_seconds": 60
  }
}
```

Detailed tool probes should be cached for a short TTL, default 60 seconds, so frequent health checks
do not repeatedly spawn `cherri` or `shortcuts`.

### `POST /api/builds`

Requires:

```text
Authorization: Bearer <auth_token>
```

Authentication must happen before reading or parsing the request body.

Request:

```json
{
  "name": "Switch Profile",
  "source_format": "cherri",
  "source": "#define name Switch Profile\nshowNotification(\"ok\", \"POC\")\n",
  "sign_mode": "anyone",
  "ttl_seconds": 2592000
}
```

Validation:

- `name`: required, 1-80 characters after trimming.
- `source_format`: required, only `cherri` in P0.
- `source`: required, non-empty, max configured UTF-8 byte limit.
- `sign_mode`: optional, default `anyone`; only `anyone` in P0.
- `ttl_seconds`: optional, default 2592000, min 60, max 2592000.

For the same source fingerprint, a repeated request updates mutable metadata, including `name` and
`expires_at`, and returns a fresh tokenized `download_url`, but keeps the same build ID.

Response 200:

```json
{
  "id": "6f1e4a9c2b3d0e771122334455667788",
  "download_url": "http://127.0.0.1:8787/s/dl_Eu4j6rN6kY6kYj3E9qJ0Dw",
  "expires_at": "2026-06-23T12:00:00Z"
}
```

Callers require `id` and `download_url`; `expires_at` tells the caller when the URL must be
regenerated.

### `GET /api/builds/<id>`

Requires:

```text
Authorization: Bearer <auth_token>
```

Returns build metadata for debugging. Fields such as `expired` status may be derived from persisted
metadata plus current configuration.

Expired builds still return 200 from this endpoint with `status: "expired"` so callers/operators can
diagnose what happened. Only `/s/<download_token>` stops serving bytes after expiration.

This endpoint must not issue or reconstruct plaintext download tokens. `download_url` is therefore
`null` in metadata responses. Callers that need a fresh download URL should re-post the same source
to `POST /api/builds`.

Response 200:

```json
{
  "id": "6f1e4a9c2b3d0e771122334455667788",
  "name": "Switch Profile",
  "source_format": "cherri",
  "source_hash": "sha256 hex",
  "sign_mode": "anyone",
  "status": "ready",
  "download_url": null,
  "download_token_count": 1,
  "toolchain": {
    "cherri": "Cherri Compiler v2.3.0",
    "shortcuts_sign": "available",
    "fingerprint": "sha256 hex"
  },
  "created_at": "2026-05-24T12:00:00Z",
  "updated_at": "2026-05-24T12:00:03Z",
  "expires_at": "2026-06-23T12:00:00Z",
  "error": null
}
```

### `GET /s/<download_token>`

Returns signed shortcut bytes only while the token maps to a build that exists, is ready, and has not
expired.

This endpoint is opened by iPhone QR/download flows and does not require the Bearer auth token. The
download token in the path is the download credential.

Headers:

```text
Content-Type: application/octet-stream
Content-Disposition: attachment; filename="<safe-name>.shortcut"
```

Response 404 when token is unknown, build is missing, failed, expired, or otherwise not ready.

Path parameter formats:

- Build IDs are lowercase hex and use the implementation's fixed public ID length, minimum 32
  characters.
- Download tokens are URL-safe, generated by the service, and must carry at least 128 bits of
  entropy. The recommended visible format is `dl_` plus base64url without padding.
- Path parameters that do not match the expected format return 404 without storage lookup.

### No P0 Delete Endpoint

P0 must not expose `DELETE /api/builds/<id>` or any other destructive cleanup HTTP endpoint. Manual
cleanup is an operator filesystem action under the configured storage directory. Expired artifacts may
remain on disk until the operator removes them.

## Error Shape

All JSON errors:

```json
{
  "ok": false,
  "error": {
    "code": "VALIDATION_FAILED",
    "message": "source_format must be cherri"
  }
}
```

Codes:

| Code | HTTP | Meaning |
|---|---:|---|
| `VALIDATION_FAILED` | 400 | Bad request fields |
| `UNAUTHORIZED` | 401 | Missing or invalid Bearer token for protected endpoint |
| `PAYLOAD_TOO_LARGE` | 413 | Source or request body exceeds configured limit |
| `BUILD_FAILED` | 422 | Cherri compile failed |
| `SIGN_FAILED` | 422 | `shortcuts sign` failed |
| `NOT_FOUND` | 404 | Build id or download token not found |
| `TOOL_UNAVAILABLE` | 503 | Required external tool missing or unusable |
| `SERVER_BUSY` | 503 | Build concurrency limit reached and request cannot be queued |
| `TIMEOUT` | 504 | Compile or sign phase exceeded configured timeout |
| `INTERNAL_ERROR` | 500 | Unexpected server error |

Do not include raw source in errors or logs.

## Security Boundaries

This is a LAN service, but protected build APIs require generic Bearer authentication by design.
The service auth token is operational configuration, not a Tsugi business field and not part of the
JSON request body.

Shortcut signing is not a purely local privacy boundary: macOS `shortcuts sign` sends shortcut
content to Apple for validation. This is accepted for P0, but callers must only embed secrets they
are willing to place inside a signed shortcut and should prefer short-lived or revocable tokens.

Required mitigations:

- Do not invoke compilers/signers through string-built shell commands.
- Pass CLI args as structured argument arrays or the implementation equivalent.
- Refuse to start without an auth token configured.
- Compare Bearer tokens using a constant-time comparison where the implementation stack provides one.
- Authenticate protected routes before reading or parsing request bodies.
- Do not log `Authorization` headers, service auth tokens, or download tokens.
- Do not derive download tokens from build IDs, source hashes, or source content.
- Persist only download token hashes.
- Keep source size bounded.
- Use temporary directories for compile output. Temporary directories containing source must be
  private to the service process, equivalent to mode `0700`; metadata and artifacts should be owner
  readable/writable only where the platform supports it.
- Sanitize filenames before using them in `Content-Disposition`. At minimum, strip or replace line
  breaks, quotes, backslashes, path separators, and semicolons; do not allow request `name` to
  become a raw header value.
- Prevent path traversal by treating build IDs as opaque hex strings.
- Do not log source.
- Delete temporary source files after each build attempt.
- Do not persist raw compiler/signer stderr unless it has been sanitized to avoid echoing source
  lines or embedded secrets.
- Do not expose destructive cleanup/delete HTTP endpoints.
- Bound total concurrent compile/sign work. P0 default is one build at a time.
- If the global build concurrency limit is saturated, return `SERVER_BUSY` immediately. P0 does not
  queue build requests behind the semaphore.
- Acquire an exclusive single-process storage lock, such as `<storage>/.lock`, before serving
  requests. If another process owns the lock, refuse to start.
- Treat metadata files as the source of truth. If an auxiliary download-token index is used, rebuild
  it at startup by scanning metadata instead of trusting a stale index.
- Persist artifacts and metadata crash-safely where practical: write temp files, flush file contents,
  atomically rename, and flush parent directories or use the platform's equivalent durability
  primitive.
- On compile/sign timeout, terminate the child process and, where supported, its process group. Do
  not leave timed-out `cherri` or `shortcuts` processes running in the background.
- Log route patterns, not raw paths containing download tokens.
- Do not run detailed health tool probes on every request; cache probe results for the configured
  health cache TTL.

## Local Maintenance

Do not expose cleanup over HTTP in P0. The implementation should provide an operator-only local
maintenance command or documented procedure to remove expired artifacts, such as:

```bash
<server binary> gc --expired-before 30d
```

The exact command is implementation-defined, but it must operate only on the configured storage
directory and must not require callers to invoke a network endpoint.

## Acceptance Criteria

1. `bash scripts/check-env.sh` passes on the Mac.
2. The implementation's documented test command passes.
3. `GET /health` returns basic liveness without auth, and detailed tool status with valid auth.
4. Protected endpoints reject missing or invalid Bearer auth with `UNAUTHORIZED`.
5. Authenticated `POST /api/builds` with `docs/examples/minimal-request.json` returns a tokenized
   download URL.
6. `GET /s/<download_token>` returns signed shortcut bytes that iPhone can import.
7. Re-posting the same request returns the same 32-hex-or-longer ID, refreshes `expires_at`, may
   rotate `download_url`, and does not rebuild if the ready artifact still exists and the toolchain
   fingerprint is unchanged.
8. Invalid `source_format` returns `VALIDATION_FAILED`.
9. Invalid Cherri source returns `BUILD_FAILED` without logging source.
10. Expired builds are not served from `/s/<download_token>`, `GET /api/builds/<id>` still returns metadata with
   `status: "expired"`, and re-posting the same source refreshes the same ID.
11. `DELETE /api/builds/<id>` is not exposed in P0.
12. A LAN caller can set its build server URL and auth token for this service and receive ready
    shortcut entries.
13. Plaintext download tokens are absent from persisted metadata/index files and logs.
14. Detailed health probes are cached.
