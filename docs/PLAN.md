# Development Plan

## Deliverable

Build Shortcut Forge, a generic macOS shortcut build/sign server. The server accepts complete Cherri source,
compiles it, signs it, persists the signed shortcut, and serves it by stable download URL.

This plan intentionally does not prescribe a language, framework, package manager, or directory
layout. The Mac-side agent chooses the implementation stack, then documents how to run and test it.

## Scope

- HTTP API defined in `SPEC.md`.
- Local persistent storage for signed shortcuts and metadata.
- Random download tokens stored only as hashes.
- Cherri compile integration.
- macOS Shortcuts signing integration.
- Generic Bearer authentication for protected API endpoints.
- Ops-friendly health endpoint.
- Static OpenAPI contract in `docs/openapi.yaml`.
- Deterministic/idempotent build behavior.
- Smoke test support using `scripts/smoke-build.sh`.

## Out Of Scope

- Business-specific source rendering.
- Tsugi-specific request fields.
- QR generation.
- iCloud links.
- Business-specific authentication or Tsugi token parsing.
- Destructive cleanup/delete HTTP endpoints.
- Auth token rotation.
- Rust runtime OpenAPI generation or OpenAPI middleware.
- iPhone NFC automation.
- Hosted database or cloud storage.

## Architecture

```text
Caller
  -> POST /api/builds with Bearer auth and complete Cherri source
  -> Mac server validates generic request
  -> Mac server compiles source with cherri
  -> Mac server signs compiled shortcut with shortcuts sign
  -> Mac server persists signed shortcut and metadata
  -> Caller receives id + tokenized download_url
  -> iPhone downloads GET /s/<download_token>
```

## Required Public Interfaces

The implementation must expose:

- `GET /health`
- `POST /api/builds`
- `GET /api/builds/<id>`
- `GET /s/<download_token>`

The implementation must document:

- Local test command.
- Local run command.
- LAN run command with bind host, port, public base URL, storage directory, source size limit, and
  build timeout.
- Max build concurrency; P0 default is 1.
- Auth token configuration.

## Implementation Steps

1. Choose a local macOS server stack.
   - Record the choice in the implementation README.
   - Keep dependencies minimal and local.
   - Do not introduce Tsugi-specific concepts.

2. Implement request validation.
   - Require `Authorization: Bearer <auth_token>` for protected API routes.
   - Authenticate before reading or parsing request bodies.
   - Keep `/health` anonymously accessible for basic liveness.
   - Validate required fields and source size.
   - Reject unsupported `source_format` and `sign_mode`.
   - Validate path parameter formats before storage lookup.
   - Return stable JSON errors.

3. Implement deterministic build identity.
   - Hash `source_format`, `sign_mode`, and raw `source`.
   - Use at least the first 32 hex characters of the hash as the stable ID.
   - Verify the full source hash when loading an existing ID to guard against truncated-ID
     collisions.
   - Repeated identical requests return the same build ID.

4. Implement durable storage.
   - Persist signed shortcut bytes.
   - Persist metadata needed by `GET /api/builds/<id>`.
   - Persist or index active random download token hashes for each ready build.
   - Persist only the download token hash, never the plaintext token.
   - Rotate the tokenized download URL on repeated `POST /api/builds`; callers must use the latest
     returned URL.
   - Keep older unexpired download token hashes active until their `expires_at`.
   - Do not expose token hashes via JSON APIs; return `download_token_count` for diagnostics.
   - Preserve `created_at` on same-source renewal; update `updated_at` and `expires_at`.
   - Treat metadata files as source of truth and rebuild any download-token index from metadata on
     startup.
   - Persist toolchain fingerprint metadata for Cherri and Shortcuts signing.
   - Do not persist raw source in metadata.
   - Do not persist absolute download URLs as source-of-truth metadata; derive them from current
     `public_base_url`.
   - Treat `expires_at` as a real serving cutoff.
   - Keep expired metadata/artifacts until renewed by POST or manually removed by the operator.
   - Acquire an exclusive storage lock before serving requests.
   - Use atomic write/rename plus file and parent-directory flushes where practical.
   - Ensure service restart does not lose ready builds.

5. Implement the build pipeline.
   - Write source to a temporary directory.
   - Ensure temporary source directories are private to the service process.
   - Compile with `cherri --skip-sign`.
   - Sign with `shortcuts sign --mode anyone`.
   - Move signed output into durable storage atomically.
   - Clean temporary files after success or failure.
   - Rebuild same-source artifacts when the stored toolchain fingerprint differs from the current
     toolchain fingerprint.
   - On timeout, terminate the child process and process group where supported.

6. Implement concurrency protection.
   - Prevent two identical builds from writing the same output concurrently.
   - Bound total concurrent builds so different source requests cannot spawn unbounded tool
     processes.
   - Use max build concurrency default 1 for P0.
   - Return `SERVER_BUSY` immediately when the global build concurrency limit is saturated.
   - Return the existing ready build when possible.

7. Implement download serving.
   - Serve `GET /s/<download_token>` with shortcut bytes and attachment headers.
   - Do not require Bearer auth for `/s/<download_token>`; the random token is the download
     credential.
   - Reject missing, failed, expired, or in-progress builds with 404.
   - Do not expose a P0 HTTP delete/cleanup endpoint.

8. Implement health and diagnostics.
   - `GET /health` reports basic service status without auth.
   - `GET /health` may include tool availability when called with valid Bearer auth.
   - Cache detailed tool probes, default 60 seconds.
   - Logs include method, route pattern, status, and build ID.
   - Logs never include raw source, Authorization headers, service auth tokens, or download tokens.

9. Implement local maintenance.
   - Provide an operator-only local command or documented procedure to remove expired artifacts.
   - Do not expose cleanup over HTTP.

10. Maintain OpenAPI contract.
   - Keep `docs/openapi.yaml` aligned with `docs/SPEC.md`.
   - Treat OpenAPI as a static downstream integration contract.
   - Do not introduce Rust runtime OpenAPI generation for P0.
   - Validate the OpenAPI file with `scripts/check-openapi.sh`.

11. Add tests.
   - Missing/invalid auth on protected routes returns `UNAUTHORIZED`.
   - Anonymous `/health` returns basic liveness.
   - Authenticated `/health` returns detailed tool status.
   - Detailed health probes are cached.
   - Validation errors.
   - Deterministic ID stability.
   - Build IDs are at least 32 hex characters.
   - Missing build lookup.
   - Same request idempotency.
   - Repeated same-source request with a different `ttl_seconds` refreshes `expires_at`, returns a
     new `download_url`, and keeps the same ID.
   - Download URL uses a random token and is not derived from the build ID.
   - Plaintext download tokens are not written to metadata/index files.
   - `GET /api/builds/<id>` returns `download_url: null`.
   - `GET /api/builds/<id>` returns `download_token_count`, not token hashes.
   - Invalid build ID and download token path formats return 404 before storage lookup.
   - Protected routes reject unauthenticated requests before parsing large request bodies.
   - Re-posting same-source returns a new download URL; callers use the latest returned URL.
   - Older unexpired token hashes for the same build continue to work.
   - Same-source renewal preserves `created_at` and updates `updated_at`/`expires_at`.
   - Download-token index can be rebuilt from metadata.
   - Second process cannot start against the same storage directory.
   - Timeout kills compile/sign child process.
   - Toolchain fingerprint change triggers rebuild under the same ID.
   - Expired build download returns 404.
   - Expired build metadata lookup returns 200 with expired status.
   - Same source can refresh the same ID after expiry.
   - DELETE route is absent or returns method-not-allowed/not-found.
   - Error response shape.

12. Run smoke test.
   - Start the service locally.
   - Run `SERVER_AUTH_TOKEN=<token> bash scripts/smoke-build.sh`.
   - Fetch the returned download URL.
   - Import the signed shortcut on iPhone for final manual acceptance.

## Failure Handling

- Cherri compile failure returns `BUILD_FAILED`.
- Signing failure returns `SIGN_FAILED`.
- Unknown build ID returns `NOT_FOUND`.
- Missing or invalid Bearer auth on protected routes returns `UNAUTHORIZED`.
- Oversized source or request body returns `PAYLOAD_TOO_LARGE`.
- Missing required external tool returns `TOOL_UNAVAILABLE`.
- Build concurrency saturation may return `SERVER_BUSY`.
- Tool timeout returns `TIMEOUT`.
- Timeout cleanup must terminate the timed-out tool process.
- Expired builds are not served from `/s/<download_token>`, but metadata remains inspectable.
- Re-posting the same source refreshes the same deterministic ID, returns a new `download_url`, and
  does not rebuild while the artifact still exists.
- Temporary files must be cleaned even on failure.
- Failed builds may persist metadata with `status = "failed"` for inspection, but must not be served
  from `/s/<download_token>`.

## Rollback

The service stores only local files under its configured storage directory. To rollback, stop the
process and delete that storage directory. Callers will see missing or not-ready shortcuts until the
server is restored and builds are regenerated.

## Verification Checklist

```bash
bash scripts/check-env.sh
bash scripts/check-openapi.sh
<implementation test command>
<implementation run command>
```

In another shell:

```bash
SERVER_AUTH_TOKEN=<token> bash scripts/smoke-build.sh
```

Manual:

1. Open the returned `download_url` on iPhone.
2. Confirm iOS imports the signed shortcut.
3. Re-run the same build request and confirm the ID is unchanged.
4. Confirm expired builds stop downloading and re-posting refreshes the same ID with the latest
   returned download URL.
5. Confirm the implementation's local GC procedure removes expired artifacts without an HTTP delete
   endpoint.

## Backlog Considerations

- Re-evaluate Rust runtime OpenAPI integration, such as `utoipa` or `aide`, only if the API surface
  grows enough that static `docs/openapi.yaml` maintenance becomes error-prone.
