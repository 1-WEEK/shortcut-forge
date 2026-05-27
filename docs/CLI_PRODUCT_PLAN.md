# CLI Product Plan

Shortcut Forge should feel like a Mac utility, not a pile of launchd, plist, env, and curl
instructions.

The CLI is the primary product surface. A tray app may come later, but it must wrap the same
CLI/config/launchd model rather than replacing it.

## Current Status

The core CLI productization work is now landed.

The operator happy path is:

```bash
shortcut-forge init
shortcut-forge start
shortcut-forge smoke
```

Delivered commands:

- `shortcut-forge init`
- `shortcut-forge doctor`
- `shortcut-forge start`
- `shortcut-forge stop`
- `shortcut-forge restart`
- `shortcut-forge status`
- `shortcut-forge logs`
- `shortcut-forge config show`
- `shortcut-forge config set`
- `shortcut-forge token rotate`
- `shortcut-forge smoke`
- `shortcut-forge build`
- `shortcut-forge serve`
- `shortcut-forge gc`

## Product Goal

A user should be able to install, initialize, run, inspect, test, and maintain Shortcut Forge with
these commands:

```bash
shortcut-forge init
shortcut-forge start
shortcut-forge status
shortcut-forge smoke
```

Advanced users can still use:

```bash
shortcut-forge serve --config <file>
shortcut-forge gc --config <file>
```

## First-Run Flow

```bash
mise run install
shortcut-forge init
shortcut-forge start
shortcut-forge smoke
```

Current `init` behavior:

- creates `~/Library/Application Support/ShortcutForge/`
- creates `~/Library/Application Support/ShortcutForge/data/`
- creates `~/Library/Logs/ShortcutForge/`
- generates and prints a high-entropy service auth token when one does not already exist
- preserves an existing `auth_token` and prints `[unchanged]` unless the operator rotates it later
- detects the Cherri binary path when possible
- defaults `shortcuts_bin` to `/usr/bin/shortcuts`
- suggests `http://<hostname>.local:8787` as `public_base_url`
- supports interactive input or `--yes` / `--non-interactive`
- writes `shortcut-forge.conf` with mode `0600`
- writes `~/Library/LaunchAgents/com.shortcut-forge.plist`
- validates the plist with `plutil -lint`
- can optionally start the service immediately in interactive mode
- prints the service URL, config path, auth token (or `[unchanged]`), and smoke command

The user should not need to hand-edit a plist during the happy path.

## Current Command Surface

### `shortcut-forge init`

Interactive setup.

Options:

```text
--config <file>             default ~/Library/Application Support/ShortcutForge/shortcut-forge.conf
--host <host>               default 0.0.0.0
--port <port>               default 8787
--public-base-url <url>     default http://<hostname>.local:<port>
--storage <dir>             default ~/Library/Application Support/ShortcutForge/data
--non-interactive           fail instead of prompting
--yes                       accept defaults and overwrite generated files where safe
```

### `shortcut-forge doctor`

Environment and deployment diagnostics.

Current checks:

- macOS detected
- `shortcut-forge` binary path
- config file exists and parses
- auth token is present but not printed
- Cherri path exists and `cherri --version` works
- `shortcuts help sign` works
- storage and log directories are writable
- configured port is bindable or already owned by the service
- `public_base_url` host is resolvable from the Mac
- LaunchAgent exists and passes `plutil -lint`
- running service responds to `/health`, if started

Supports `--json`.

### `shortcut-forge start`

Start the LaunchAgent.

Current behavior:

- create or update LaunchAgent from the current config if missing
- run `launchctl bootstrap` when not loaded
- run `launchctl kickstart -k` when loaded
- print the local status and health URL

### `shortcut-forge stop`

Stop the LaunchAgent with `launchctl bootout`.

If the service is not loaded, print that state and exit 0.

### `shortcut-forge restart`

Restart the LaunchAgent.

Equivalent to `start` when it is not running.

### `shortcut-forge status`

Show operator status without exposing secrets.

Current output includes:

- loaded/running/stopped state from `launchctl`
- PID when available
- configured URL
- storage path
- config path
- health result
- Cherri version and Shortcuts sign availability when health is available
- last few stderr log lines when unhealthy

Supports `--json`.

### `shortcut-forge logs`

Tail service logs.

Options:

```text
--follow
--lines <n>                 default 80
```

Reads `~/Library/Logs/ShortcutForge/stdout.log` and `stderr.log`.

### `shortcut-forge config show`

Print effective config with secrets redacted.

### `shortcut-forge config set <key> <value>`

Safely edit config file values.

Currently supported keys:

```text
host
port
public_base_url
storage
max_source_bytes
build_timeout_seconds
max_build_concurrency
health_cache_ttl_seconds
cherri_bin
shortcuts_bin
expired_before
```

After changing values that affect the running service, print:

```text
Run `shortcut-forge restart` to apply this change.
```

### `shortcut-forge token rotate`

Generate and store a new service auth token.

Current behavior:

- update `auth_token` in config
- keep file mode `0600`
- do not print the raw token unless `--print` is explicitly passed
- warn that callers must be updated
- print `shortcut-forge restart` as the next step

### `shortcut-forge smoke`

Run a local API smoke test without requiring the shell script.

Current behavior:

- read config
- call `POST /api/builds` with the minimal sample request
- fetch the returned `download_url`
- write the result to `/tmp/minimal.signed.shortcut`
- print the build ID, expiry, and output path

Options:

```text
--request <json-file>       default docs/examples/minimal-request.json when run from repo
--output <file>             default /tmp/minimal.signed.shortcut
```

### `shortcut-forge build <json-file>`

Submit a build request to the running server.

This is a CLI wrapper around `POST /api/builds`; it does not replace the HTTP API.

Output is readable by humans by default and JSON with `--json`.

### `shortcut-forge gc`

Keep the existing cleanup command.

Current behavior:

- read `storage` from config when `--config` is provided
- read `expired_before` from config when present
- print removed count and storage path

### `shortcut-forge serve`

Keep as the low-level server command used by launchd.

It remains scriptable and non-interactive.

## Remaining Work

The foundational CLI surface is complete. Remaining work is polish and follow-through rather than
first delivery.

### Short-Term Polish

- Make `doctor` output more compact and more explicitly fix-oriented.
- Improve `status` messaging for partial install states and launchd failures.
- Add stronger operator docs for backup/restore of config, logs, and storage.
- Keep `README.md`, `docs/RUST_ARCHITECTURE.md`, and packaging examples aligned with behavior.

### Medium-Term Maintenance

- Add stronger integration-test helpers around fake Cherri/Shortcuts binaries.
- Consider splitting `src/main.rs` if the current single-file implementation becomes hard to change.
- Improve launchd troubleshooting guidance without changing the low-level service model.

### Optional Future Product Work

- macOS tray app that wraps the CLI for status, start/stop/restart, config access, token rotation,
  log access, and smoke test launch
- richer operator status surfaces if the service grows more operational complexity

The tray must not be required for headless use.

## Acceptance Criteria

The README happy path should remain:

```bash
shortcut-forge init
shortcut-forge start
shortcut-forge smoke
```

A clean Mac with `mise`, Cherri, and Shortcuts signing available should not require manual plist
editing.

Service auth token should live in the permission-restricted config file by default, not in shell
profile files or launchd environment variables.

`shortcut-forge status` should answer:

- Is it installed?
- Is it running?
- What URL should callers use?
- Are Cherri and Shortcuts signing available?
- Where are config, data, and logs?

`shortcut-forge doctor` should explain what to fix when setup is incomplete.

Existing HTTP API behavior from `docs/SPEC.md` must remain unchanged.
