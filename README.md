# Shortcut Forge

Shortcut Forge is a standalone generic Mac service used to build and sign iOS Shortcuts.

The service does not understand Tsugi or any other caller's business domain. It receives complete
Cherri source, compiles and signs it, stores the signed `.shortcut`, and returns a download URL.
P0 protected API endpoints require generic Bearer authentication. Download URLs use random
unguessable tokens, rotate on renewal, expire for real, and expired artifacts are not
automatically deleted. The service does not expose destructive cleanup/delete HTTP endpoints.

## Copy To Mac

Copy this whole directory to the Mac. If you are following the Rust implementation path, install
`mise` first, then run:

```bash
mise trust
mise install
bash scripts/check-env.sh
```

Then give `docs/IMPLEMENTATION_PROMPT.md` to the Mac-side agent when continuing implementation.

The implementation is Rust. `mise.toml` pins the local Rust toolchain, installs `cherri`
through the `github:electrikmilk/cherri` backend, and exposes helper
tasks:

```bash
mise run check-env
mise run test
mise run run-local
mise run smoke-build
```

`shortcuts` is still provided by macOS. The Rust path now expects `cherri` to come from `mise
install`, so Homebrew is no longer required for the local Cherri CLI in this repository.

Note that macOS `shortcuts sign` sends shortcut content to Apple for validation. This is an accepted
P0 boundary, but callers should embed only secrets suitable for inclusion in a signed shortcut.

## Run And Test

Install dependencies:

```bash
mise trust
mise install
```

Run tests:

```bash
cargo test
```

Start locally:

```bash
SHORTCUT_SERVER_AUTH_TOKEN=<token> cargo run -- serve
```

Start on LAN:

```bash
SHORTCUT_SERVER_AUTH_TOKEN=<token> cargo run -- serve \
  --host 0.0.0.0 \
  --port 8787 \
  --public-base-url http://<mac-lan-host>:8787 \
  --storage ./data \
  --max-source-bytes 524288 \
  --build-timeout-seconds 30 \
  --max-build-concurrency 1 \
  --health-cache-ttl-seconds 60
```

The same options can be configured with `SHORTCUT_SERVER_*` environment variables:
`SHORTCUT_SERVER_HOST`, `SHORTCUT_SERVER_PORT`, `SHORTCUT_SERVER_PUBLIC_BASE_URL`,
`SHORTCUT_SERVER_STORAGE`, `SHORTCUT_SERVER_MAX_SOURCE_BYTES`,
`SHORTCUT_SERVER_BUILD_TIMEOUT_SECONDS`, `SHORTCUT_SERVER_MAX_BUILD_CONCURRENCY`,
`SHORTCUT_SERVER_AUTH_TOKEN`, and `SHORTCUT_SERVER_HEALTH_CACHE_TTL_SECONDS`.

Run local expired-artifact maintenance:

```bash
cargo run -- gc --storage ./data --expired-before 30d
```

No destructive cleanup or delete operation is exposed over HTTP.

## What The Implementation Provides

Shortcut Forge implements a runnable service that satisfies `docs/SPEC.md` and documents:

- How to install any chosen implementation dependencies.
- How to run tests.
- How to start the service locally.
- How to start the service on LAN, including host, port, public base URL, storage directory,
  source size limit, build timeout, max build concurrency, and auth token configuration.

The implementation path is Rust. `docs/RUST_ARCHITECTURE.md` provides the concrete reference
architecture while still following the generic package contract.

`docs/openapi.yaml` is the static machine-readable API contract for downstream integration. Keep it
aligned with `docs/SPEC.md`, but do not introduce Rust runtime OpenAPI generation or middleware for
P0.

## Smoke Test

After the service is running on `http://127.0.0.1:8787`:

```bash
SERVER_AUTH_TOKEN=<token> bash scripts/smoke-build.sh
```

If the service runs elsewhere:

```bash
SERVER_URL=http://<host>:<port> SERVER_AUTH_TOKEN=<token> bash scripts/smoke-build.sh
```

## Caller Integration

A caller should set its build server URL to this service, then send `POST /api/builds` with complete
Cherri source.

For Tsugi on Windows:

```toml
[shortcuts]
build_server_url = "http://<mac-lan-host>:8787"
```

Then reload Tsugi config. Tsugi should create or update
`%APPDATA%\Tsugi\shortcuts\manifest.json` with ready entries.

## Files

- `docs/` centrally manages the package documentation, contracts, and examples.
- `docs/SPEC.md` defines requirements and API contract.
- `docs/PLAN.md` defines implementation steps without prescribing stack.
- `docs/RUST_ARCHITECTURE.md` defines a concrete Rust stack, module layout, and runtime design.
- `docs/openapi.yaml` defines the static OpenAPI contract for callers and client generation.
- `docs/IMPLEMENTATION_PROMPT.md` is the prompt to paste into the Mac agent.
- `docs/contracts/` contains request and response examples.
- `docs/examples/` contains a minimal Cherri source and JSON request.
- `Cargo.toml` and `src/` contain the Shortcut Forge Rust implementation.
- `mise.toml` pins the Rust toolchain, installs Cherri, and exposes local helper tasks.
- `scripts/check-env.sh` verifies macOS tool availability.
- `scripts/check-openapi.sh` parses the static OpenAPI contract locally; set `REDOCLY_LINT=1` to run
  Redocly lint with Node/npx.
- `scripts/smoke-build.sh` tests the implemented HTTP API.
