# Prompt For Mac Agent

You are implementing Shortcut Forge, the Mac shortcut build/sign server, in this directory.

Read these files first, relative to the repository root:

1. `AGENTS.md`
2. `docs/SPEC.md`
3. `docs/PLAN.md`
4. `docs/openapi.yaml`
5. `docs/RUST_ARCHITECTURE.md`

Implement a runnable service that satisfies the requirements and API contract. Keep it generic:
do not add Tsugi-specific fields, routes, business token handling, or source templating.

P0 protected API endpoints require generic `Authorization: Bearer <auth_token>` service
authentication. This is not a Tsugi business field and must not appear in the JSON request body.
`GET /health` remains anonymous for basic liveness; with valid Bearer auth it may return detailed
tool availability.

Protected routes must authenticate before reading or parsing request bodies.

P0 must not expose destructive cleanup/delete HTTP endpoints. `ttl_seconds`/`expires_at` are real:
expired builds must not download, and re-posting the same source refreshes the same deterministic
ID. Re-posting returns a new tokenized download URL while older unexpired tokens remain valid.
Generate absolute download URLs from the
current `public_base_url`; do not persist stale absolute URLs as source-of-truth metadata.

Download URLs must use a random unguessable download token, not the deterministic build ID. The
download route does not require Bearer auth because iPhone install flows open it directly; the path
token is the download credential and must not be logged or persisted in plaintext.
`GET /api/builds/<id>` must not reconstruct or issue plaintext download tokens; return
`download_url: null` there and expose only `download_token_count`, not token hashes. Callers needing
a fresh URL re-post the same source.

Use these P0 semantics:

- A repeated same-source request refreshes mutable metadata and `expires_at`, returns a new
  `download_url`, and keeps the same build ID.
- Same-source renewal preserves `created_at` and updates `updated_at`/`expires_at`.
- Expired artifacts are not automatically deleted.
- `GET /api/builds/<id>` returns expired metadata with 200; only `/s/<download_token>` returns 404
  after expiry.
- Bound compile/sign concurrency; default to one build at a time.
- Return `SERVER_BUSY` immediately when build concurrency is saturated; do not queue P0 builds.
- Refuse to start without an auth token configured.

You may choose the implementation language, framework, and project layout. Document those choices
and provide exact test and run commands.

Implement with the Rust reference path unless the operator explicitly changes the stack. Use the
repository `mise.toml` to manage the local Rust toolchain, install Cherri, and run helper tasks.

Keep `docs/openapi.yaml` aligned with the implemented API. It is a static downstream contract for
P0; do not add Rust runtime OpenAPI generation or middleware. Treat `utoipa`/`aide`-style runtime
integration as backlog only.

Required external tools:

- `cherri`
- macOS `shortcuts sign`

`shortcuts sign` sends shortcut content to Apple for validation. This is an accepted P0 boundary, but
do not log or persist raw source or unsanitized compiler/signer output.

After implementation, run:

```bash
bash scripts/check-env.sh
bash scripts/check-openapi.sh
<documented test command>
<documented run command>
```

In another shell:

```bash
SERVER_AUTH_TOKEN=<token> bash scripts/smoke-build.sh
```

Do not log shortcut source. It may contain secrets embedded by the caller.
