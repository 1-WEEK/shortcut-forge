# AGENTS.md

This directory is a standalone handoff package for building Shortcut Forge, the Mac Shortcut Build/Sign Server.
It must remain generic: do not import or depend on Tsugi source code.

## Mission

Build a macOS HTTP service that accepts complete Cherri source, compiles it with `cherri`,
signs it with macOS `shortcuts sign`, stores the signed `.shortcut`, and serves a stable download URL.

## Responsibility Split

### Caller Responsibilities

- Understand business meaning.
- Render complete Cherri source.
- Embed endpoints, tokens, names, app-intent parameters, and control flow into that source.
- Generate and display QR codes.
- Decide when a shortcut should be regenerated.

### Mac Server Responsibilities

- Validate generic build requests.
- Compile Cherri source.
- Sign compiled shortcuts.
- Persist signed output and metadata.
- Return and serve stable download URLs.
- Expose health and build metadata endpoints.

### Explicit Non-Responsibilities

- Do not understand Tsugi business fields.
- Do not accept fields like `profile_id`, `switch_url`, `sleep_url`, or `token`.
- Do not template or mutate shortcut source.
- Do not log request `source`; it can contain secrets embedded by the caller.
- Do require generic service authentication for protected API endpoints in P0.
- Do not treat service authentication as a Tsugi business token or request body field.
- Do not log service auth tokens or download tokens.
- Do not expose destructive cleanup/delete HTTP endpoints for P0.
- Do not create iCloud sharing links.
- Do not generate QR codes for P0.
- Do not generate or manage iPhone NFC automations.

## Technology Policy

Do not treat this package as prescribing an implementation stack.

The implementation may use any reasonable local macOS server technology as long as it satisfies
`docs/SPEC.md`, ships clear run/test commands, and depends only on tools that are documented in the
implementation README.

Required external capabilities are only:

- `cherri`
- macOS `shortcuts sign`
- A local HTTP server implementation chosen by the Mac-side agent

## Verification Before Done

Run:

```bash
bash scripts/check-env.sh
<project test command documented by the implementation>
<project run command documented by the implementation>
```

In another shell:

```bash
SERVER_AUTH_TOKEN=<token> bash scripts/smoke-build.sh
```

Final acceptance requires importing the generated signed shortcut on an iPhone.
