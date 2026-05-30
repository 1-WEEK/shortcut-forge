# Maintenance Roadmap

Shortcut Forge P0 has a working Rust implementation. This file tracks remaining maintenance and
future work. It is not a from-scratch implementation plan.

For current behavior, read `SPEC.md`.
For current Rust implementation details, read `RUST_ARCHITECTURE.md`.
For the current CLI product status and remaining polish, read `CLI_PRODUCT_PLAN.md`.

## P0 Contract

Keep these P0 boundaries stable:

- Generic build/sign/download service only.
- No Tsugi-specific request fields or business logic.
- Complete Cherri source comes from the caller.
- Protected API endpoints require Bearer auth.
- `/s/<download_token>` is unauthenticated because the path token is the download credential.
- Download tokens are random and persisted only as hashes.
- Same-source POST reuses the deterministic build ID and returns a fresh download URL.
- Expired builds stop downloading, but metadata remains inspectable.
- No destructive cleanup/delete HTTP endpoint.
- `docs/openapi.yaml` remains a static downstream contract.

## Current Verification

Routine checks:

```bash
mise run check-env
mise run check-openapi
mise run test
```

Runtime smoke test:

```bash
cargo run -- init
cargo run -- start
cargo run -- smoke
```

Low-level API smoke test:

```bash
mise run run-local -- --config "$HOME/Library/Application Support/ShortcutForge/shortcut-forge.toml"
SERVER_AUTH_TOKEN=<token> mise run smoke-build
```

Manual acceptance:

1. Open the returned `download_url` on an iPhone.
2. Confirm iOS imports the signed shortcut.
3. Re-run the same build request and confirm the ID is unchanged.
4. Confirm the new response may rotate `download_url`.
5. Confirm expired builds stop downloading and re-posting refreshes the same ID.

## Near-Term Maintenance

- Keep the delivered CLI product surface stable: `shortcut-forge init`, `shortcut-forge start`,
  `shortcut-forge status`, `shortcut-forge smoke`, and the related operator commands should remain
  the primary management path.
- Keep `docs/CLI_PRODUCT_PLAN.md` honest about what is already implemented versus what remains as
  polish or future work.
- Keep `README.md`, `docs/SPEC.md`, `docs/openapi.yaml`, and examples aligned whenever the API
  changes.
- Keep `docs/RUST_ARCHITECTURE.md` aligned with the actual code. Do not describe framework stacks
  that are not in `Cargo.toml`.
- Expand tests when changing auth, token handling, storage, process execution, or expiry behavior.
- Keep Cherri invocation pinned to the behavior verified for the version in `mise.toml`.
- Re-run smoke tests on a Mac after changes to build/sign behavior.
- Keep CLI support as the primary management path. A future tray app must wrap the CLI/launchd
  service, not replace headless operation.
- Keep `packaging/config/shortcut-forge.toml.example` aligned with CLI config keys.

## Backlog

- Add stronger integration test helpers for fake Cherri/Shortcuts binaries.
- Add a documented operator procedure for backing up and restoring the storage directory.
- Add optional structured logging configuration while preserving the no-secret logging rules.
- Add a macOS tray management app for status, start/stop/restart, safe config edits, service-token
  rotation with restart, log access, and smoke-test launch.
- Evaluate additional middleware or observability crates only if operational needs outgrow the current
  standard-library implementation.
- Re-evaluate runtime OpenAPI generation only if the API surface grows enough that maintaining the
  static `docs/openapi.yaml` becomes error-prone.

## Out Of Scope For P0

- Business-specific source rendering.
- Tsugi-specific request fields.
- QR generation.
- iCloud links.
- Business-specific authentication or Tsugi token parsing.
- HTTP cleanup/delete endpoints.
- iPhone NFC automation generation.
- Hosted database or cloud storage.
