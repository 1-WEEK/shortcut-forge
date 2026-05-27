# Shortcut Forge

A Mac service that builds and signs iOS Shortcuts from [Cherri](https://github.com/electrikmilk/cherri) source.

Send it Cherri source, get back a download URL that iPhone can open directly. The server handles compilation and signing; you own what the shortcut does.

## Install

On the signing Mac:

```bash
mise trust
mise install
mise run check-env
mise run install
```

Then set up config and register the background service:

```bash
shortcut-forge init
```

This creates the config file, data directory, log directories, and a LaunchAgent plist so the service can run at login.

`init` prints a generated `auth_token`. It keeps an existing token unless you ask to rotate it later.

## Quick Start

```bash
shortcut-forge init
shortcut-forge start
shortcut-forge smoke
```

`smoke` submits the sample request from `docs/examples/minimal-request.json`, fetches the download URL, and saves the signed shortcut to `/tmp/minimal.signed.shortcut`. Open that on iPhone to import.

## Config

The config file lives at `~/Library/Application Support/ShortcutForge/shortcut-forge.conf`. It's a flat `key = value` file:

```text
host = "0.0.0.0"
port = 8787
public_base_url = "http://mac-mini.local:8787"
storage = "/Users/YOU/Library/Application Support/ShortcutForge/data"
auth_token = "CHANGE_ME"
cherri_bin = "/Users/YOU/.local/share/mise/installs/github-electrikmilk-cherri/2.3.0/cherri"
shortcuts_bin = "/usr/bin/shortcuts"
```

Config from CLI flags overrides environment variables, which override the config file.

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

Health check (no auth needed for basic liveness):

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

`host` controls where the server binds. `public_base_url` controls the download URLs it generates. Use a hostname callers can resolve — Bonjour name (`mac-mini.local`), DHCP hostname, fixed IP, or local DNS record.

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

```bash
mise run check-env
mise run check-openapi
mise run test
shortcut-forge init
shortcut-forge start
shortcut-forge smoke
mise run run-local -- --config "$HOME/Library/Application Support/ShortcutForge/shortcut-forge.conf"
```

Relevant docs:

- `docs/SPEC.md` — full behavior and API contract
- `docs/RUST_ARCHITECTURE.md` — implementation notes
- `docs/AGENT_HANDOFF.md` — context for coding agents
- `docs/openapi.yaml` — machine-readable API contract
