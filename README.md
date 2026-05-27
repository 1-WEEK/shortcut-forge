# Shortcut Forge

Shortcut Forge is a Mac service that builds and signs iOS Shortcuts.

You send complete Cherri source. It compiles the source with `cherri`, signs the shortcut with
macOS `shortcuts sign`, stores the signed `.shortcut`, and returns an iPhone-openable download URL.

It does not render shortcut source, understand business fields, create QR codes, or make iCloud
sharing links.

## Install

On the Mac that will sign shortcuts:

```bash
mise trust
mise install
mise run check-env
mise run install
```

Initialize the operator config and LaunchAgent:

```bash
shortcut-forge init
```

This creates:

- `~/Library/Application Support/ShortcutForge/shortcut-forge.conf`
- `~/Library/Application Support/ShortcutForge/data/`
- `~/Library/Logs/ShortcutForge/`
- `~/Library/LaunchAgents/com.shortcut-forge.plist`

`init` generates and prints a new `auth_token` when one does not already exist. It preserves an existing token unless you explicitly rotate it later.

## Use

Shortcut Forge is CLI-first. There is no required GUI.

```bash
shortcut-forge --help
shortcut-forge init
shortcut-forge start
shortcut-forge status
shortcut-forge smoke
shortcut-forge doctor
shortcut-forge logs --lines 80
shortcut-forge config show
shortcut-forge token rotate
shortcut-forge build docs/examples/minimal-request.json
shortcut-forge gc --config "$HOME/Library/Application Support/ShortcutForge/shortcut-forge.conf"
shortcut-forge serve --config "$HOME/Library/Application Support/ShortcutForge/shortcut-forge.conf"
```

Commands:

- `init` creates the config, log directories, and LaunchAgent plist.
- `doctor` checks config, toolchain, launchd, ports, and local health.
- `start`, `stop`, `restart`, and `status` manage the launchd service.
- `logs` tails `stdout.log` and `stderr.log`.
- `config show` and `config set` inspect and edit config safely.
- `token rotate` rotates the service Bearer auth token.
- `smoke` submits the sample request and downloads the signed shortcut locally.
- `build` wraps `POST /api/builds` for local operator use.
- `serve` starts the HTTP build/sign server.
- `gc` removes expired local build artifacts.
- `--help` prints supported flags.

Config precedence:

```text
CLI flags > SHORTCUT_SERVER_* environment variables > config file > defaults
```

The config file is a flat `key = value` file. Example:

```text
host = "0.0.0.0"
port = 8787
public_base_url = "http://mac-mini.local:8787"
storage = "/Users/YOU/Library/Application Support/ShortcutForge/data"
auth_token = "CHANGE_ME"
cherri_bin = "/Users/YOU/.local/share/mise/installs/github-electrikmilk-cherri/2.3.0/cherri"
shortcuts_bin = "/usr/bin/shortcuts"
```

The CLI is the primary product surface. A tray app can be added later as a management UI over the
same CLI/config/launchd service.

## Build A Shortcut

Happy path:

```bash
shortcut-forge init
shortcut-forge start
shortcut-forge smoke
```

`smoke` submits `docs/examples/minimal-request.json`, fetches the returned download URL, and writes
the signed shortcut to `/tmp/minimal.signed.shortcut` by default.

You can still call the API directly or run the low-level server command:

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

Open `download_url` on the iPhone to import the signed shortcut.

## Other Operations

Health:

```bash
shortcut-forge status
shortcut-forge doctor
```

Tail logs:

```bash
shortcut-forge logs --follow
```

Show or edit config:

```bash
shortcut-forge config show
shortcut-forge config set public_base_url "http://mac-mini.local:8787"
```

Build from a request file without writing curl flags by hand:

```bash
shortcut-forge build docs/examples/minimal-request.json
```

Refresh a download URL by re-posting the same source:

```bash
shortcut-forge build docs/examples/minimal-request.json
```

The build ID stays stable and the server returns a fresh `/s/dl_...` download URL. Older
unexpired download URLs keep working until their TTL.

Rotate the service auth token:

1. Run `shortcut-forge token rotate`.
2. Update callers.
3. Run `shortcut-forge restart`.

P0 does not support hot token reload or overlapping old/new service tokens.

Run cleanup:

```bash
shortcut-forge gc --config "$HOME/Library/Application Support/ShortcutForge/shortcut-forge.conf"
```

There is no HTTP delete endpoint in P0.

## LAN Hostname

For LAN use:

```text
host = "0.0.0.0"
public_base_url = "http://mac-mini.local:8787"
```

`host` controls where the server binds. `public_base_url` controls generated download URLs. It does
not register DNS.

Use a hostname or IP that the caller and iPhone can resolve:

- Bonjour name, such as `mac-mini.local`
- router DHCP hostname
- fixed LAN IP
- local DNS record

## Run At Login

Use `launchd` for normal Mac deployment.

`shortcut-forge init` writes `~/Library/LaunchAgents/com.shortcut-forge.plist` for you.

Normal operator flow:

```bash
shortcut-forge start
shortcut-forge status
shortcut-forge restart
shortcut-forge stop
```

If you need the underlying LaunchAgent details:

```bash
plutil -lint "$HOME/Library/LaunchAgents/com.shortcut-forge.plist"
launchctl print "gui/$(id -u)/com.shortcut-forge"
tail -n 80 "$HOME/Library/Logs/ShortcutForge/stdout.log" \
  "$HOME/Library/Logs/ShortcutForge/stderr.log"
```

## Develop

```bash
mise run check-env
mise run check-openapi
mise run test
shortcut-forge init
shortcut-forge start
shortcut-forge smoke
mise run run-local -- --config "$HOME/Library/Application Support/ShortcutForge/shortcut-forge.conf"
```

The packaged templates remain in `packaging/` for reference and recovery. Manual plist editing is
no longer required on the happy path.

Useful docs:

- `docs/SPEC.md` - behavior and API contract
- `docs/RUST_ARCHITECTURE.md` - current Rust implementation notes
- `docs/AGENT_HANDOFF.md` - context for future coding agents
- `docs/openapi.yaml` - static OpenAPI contract
