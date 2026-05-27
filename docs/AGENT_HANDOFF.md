# Agent Handoff

Shortcut Forge already has a P0 Rust implementation. Do not treat this repository as a blank
handoff package unless the operator explicitly asks for a rewrite.

## Start Here

Read in this order:

1. `AGENTS.md`
2. `README.md`
3. `docs/SPEC.md`
4. `docs/RUST_ARCHITECTURE.md`
5. `docs/openapi.yaml` if API behavior is changing

`docs/PLAN.md` is roadmap context only. It is not an implementation prompt.

## Current Implementation

- Single Rust binary in `src/main.rs`.
- Rust standard library only; `Cargo.toml` intentionally has no external dependencies.
- CLI commands:
  - `cargo run -- init`
  - `cargo run -- start`
  - `cargo run -- status`
  - `cargo run -- smoke`
  - `cargo run -- doctor`
  - `cargo run -- serve`
  - `cargo run -- gc --storage ./data --expired-before 30d`
- `mise.toml` pins Rust stable and Cherri 2.3.0.
- macOS provides `shortcuts sign`.

## Core Invariants

- The server receives complete Cherri source; it does not template or mutate source.
- Protected endpoints require `Authorization: Bearer <token>`.
- Protected routes authenticate before reading or parsing request bodies.
- `/health` is anonymous for basic liveness and can return detailed tool status with valid auth.
- `/s/<download_token>` does not require Bearer auth; the random path token is the download
  credential.
- Plaintext download tokens are never persisted; metadata stores token hashes only.
- `GET /api/builds/<id>` returns `download_url: null`.
- Re-posting the same source returns the same deterministic ID and a fresh download URL.
- Expired builds are not served, but metadata remains inspectable.
- P0 exposes no destructive cleanup/delete HTTP endpoint.
- Logs must not contain raw source, service auth tokens, or download tokens.

## Common Commands

```bash
bash scripts/check-env.sh
bash scripts/check-openapi.sh
cargo test
cargo run -- init
cargo run -- start
cargo run -- smoke
cargo run -- serve --config "$HOME/Library/Application Support/ShortcutForge/shortcut-forge.conf"
SERVER_AUTH_TOKEN=<token> bash scripts/smoke-build.sh
```

## Before Changing Docs

- README is for humans running and integrating the service.
- AGENTS and this file are for coding agents.
- SPEC is the contract.
- RUST_ARCHITECTURE is the current implementation.
- PLAN is future work only.

Keep those boundaries intact.
