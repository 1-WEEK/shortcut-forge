# AGENTS.md

This repository is Shortcut Forge, a standalone macOS Shortcut build/sign server.

Keep the project generic. Do not import Tsugi source code, add Tsugi-specific request fields, or
teach the server business meaning. The caller renders complete Cherri source; this service builds,
signs, stores, and serves it.

## Read Order

For any non-trivial change, read these first:

1. `README.md` - human run/test/operate entrypoint.
2. `docs/SPEC.md` - behavior and API contract.
3. `docs/RUST_ARCHITECTURE.md` - current implementation shape.
4. `docs/AGENT_HANDOFF.md` - concise continuation context.

Use `docs/openapi.yaml` when API behavior changes. Use `docs/PLAN.md` only for roadmap/backlog
context; it is not a from-scratch implementation prompt.

## Mission

Maintain a macOS HTTP service that:

- Accepts complete Cherri source.
- Compiles it with `cherri`.
- Signs it with macOS `shortcuts sign`.
- Stores the signed `.shortcut` and metadata.
- Returns and serves tokenized download URLs.
- Exposes health and build metadata endpoints.

## Responsibility Split

### Caller Responsibilities

- Understand business meaning.
- Render complete Cherri source.
- Embed endpoints, tokens, names, app-intent parameters, and control flow into that source.
- Generate and display QR codes if needed.
- Decide when a shortcut should be regenerated.

### Server Responsibilities

- Validate generic build requests.
- Require generic service authentication for protected API endpoints.
- Compile Cherri source.
- Sign compiled shortcuts.
- Persist signed output and metadata.
- Return and serve stable tokenized download URLs.
- Expose health and build metadata endpoints.

## Non-Responsibilities

- Do not understand Tsugi business fields.
- Do not accept fields like `profile_id`, `switch_url`, `sleep_url`, or `token`.
- Do not template or mutate shortcut source.
- Do not treat service authentication as a Tsugi business token or request body field.
- Do not create iCloud sharing links.
- Do not generate QR codes for P0.
- Do not generate or manage iPhone NFC automations.
- Do not expose destructive cleanup/delete HTTP endpoints for P0.

## Security Rules

- Do not log request `source`; it can contain caller-embedded secrets.
- Do not log `Authorization` headers, service auth tokens, or download tokens.
- Persist only download token hashes, never plaintext download tokens.
- Authenticate protected routes before reading or parsing request bodies.
- Invoke `cherri` and `shortcuts sign` with structured arguments, never shell interpolation.
- Sanitize filenames before using request `name` in `Content-Disposition`.

## Technology Policy

The original package contract is stack-neutral, but this repository now contains the P0 Rust
implementation. Keep documentation clear about that distinction:

- `docs/SPEC.md` remains stack-neutral.
- `docs/RUST_ARCHITECTURE.md` documents the current Rust implementation.
- Do not reintroduce a separate startup prompt that tells agents to choose a fresh stack unless the
  operator explicitly asks for a rewrite.

## Verification

For code or API changes, run:

```bash
bash scripts/check-env.sh
bash scripts/check-openapi.sh
cargo test
```

For runtime acceptance, start the server:

```bash
cargo run -- serve --config "$HOME/Library/Application Support/ShortcutForge/shortcut-forge.conf"
```

In another shell:

```bash
SERVER_AUTH_TOKEN=<token> bash scripts/smoke-build.sh
```

Final acceptance requires importing the generated signed shortcut on an iPhone.
