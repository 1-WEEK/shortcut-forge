# Shortcut Forge

A macOS service that builds and signs iOS Shortcuts from [Cherri](https://github.com/electrikmilk/cherri) source.

Send it Cherri source, get back a download URL that iPhone can open directly. The server handles compilation and signing. You own what the shortcut does.

## Install

You need [Rust](https://rustup.rs/) and [Cherri](https://github.com/electrikmilk/cherri) installed first.

Install Cherri (pick one):

```bash
# Homebrew
brew tap electrikmilk/cherri
brew install electrikmilk/cherri/cherri

# Or grab a prebuilt binary from GitHub Releases
# https://github.com/electrikmilk/cherri/releases
```

Install Shortcut Forge:

```bash
cargo install --git https://github.com/1-week/shortcut-forge
```

Initialize config and register the background service:

```bash
shortcut-forge init
```

`init` creates the config file, data directory, log directory, and a LaunchAgent plist so the service starts at login. It prints a generated `auth_token`. If a token already exists, it keeps it. You can rotate later.

## Quick Start

```bash
shortcut-forge init
shortcut-forge start
shortcut-forge smoke
```

`smoke` submits the sample request in `docs/examples/minimal-request.json`, fetches the download URL, and saves the signed shortcut to `/tmp/minimal.signed.shortcut`. Open that file on iPhone to import.

## Config

The config file lives at `~/Library/Application Support/ShortcutForge/shortcut-forge.conf`. Flat `key = value` format:

```text
host = "0.0.0.0"
port = 8787
public_base_url = "http://mac-mini.local:8787"
storage = "/Users/YOU/Library/Application Support/ShortcutForge/data"
auth_token = "CHANGE_ME"
cherri_bin = "/opt/homebrew/bin/cherri"
shortcuts_bin = "/usr/bin/shortcuts"
```

Priority order: CLI flags > environment variables > config file.

View and edit config safely:

```bash
shortcut-forge config show
shortcut-forge config set public_base_url "http://mac-mini.local:8787"
```

Rotate the auth token:

```bash
shortcut-forge token rotate
shortcut-forge restart
```

## API

Start the server and build a shortcut:

```bash
shortcut-forge serve --config "$HOME/Library/Application Support/ShortcutForge/shortcut-forge.conf"

curl -sS \
  -X POST http://mac-mini.local:8787/api/builds \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <auth_token_from_config>" \
  --data-binary @docs/examples/minimal-request.json
```

Response:

```json
{
  "id": "566a84474b1f07034532421fb70da43c",
  "download_url": "http://mac-mini.local:8787/s/dl_...",
  "expires_at": "2026-06-26T06:11:41Z"
}
```

Open `download_url` on iPhone to import the signed shortcut.

You can also use the `build` command as a shorthand:

```bash
shortcut-forge build docs/examples/minimal-request.json
```

Re-posting the same source returns a fresh download URL under the same build ID. Old URLs keep working until they expire.

Health check (no auth required):

```bash
curl http://mac-mini.local:8787/health
```

The full API contract is in `docs/openapi.yaml`.

## LAN Access

For other devices on the LAN to reach the service:

```text
host = "0.0.0.0"
public_base_url = "http://mac-mini.local:8787"
```

`host` controls where the server binds. `public_base_url` controls the download URLs it generates. Use a hostname callers can resolve: Bonjour name (`mac-mini.local`), DHCP hostname, fixed IP, or local DNS record.

Bearer auth over plain HTTP assumes a trusted network. On untrusted networks, put the service behind Tailscale, WireGuard, an SSH tunnel, or an HTTPS reverse proxy.

## Background Service

`shortcut-forge init` registers a LaunchAgent. Manage it with:

```bash
shortcut-forge start
shortcut-forge status
shortcut-forge restart
shortcut-forge stop
```

View logs:

```bash
shortcut-forge logs --follow
```

Check system health:

```bash
shortcut-forge doctor
```

Clean up expired builds:

```bash
shortcut-forge gc --config "$HOME/Library/Application Support/ShortcutForge/shortcut-forge.conf"
```

## Development

Building from source requires [mise](https://mise.jdx.dev/) to lock the toolchain versions:

```bash
mise trust
mise install
mise run check-env
mise run test
cargo build --release
```

Local development flow:

```bash
mise run check-env
mise run check-openapi
mise run test
shortcut-forge init
shortcut-forge start
shortcut-forge smoke
mise run run-local -- --config "$HOME/Library/Application Support/ShortcutForge/shortcut-forge.conf"
```

Docs:

- `docs/SPEC.md` - full behavior and API contract
- `docs/RUST_ARCHITECTURE.md` - implementation notes
- `docs/AGENT_HANDOFF.md` - context for coding agents
- `docs/openapi.yaml` - machine-readable API contract
