# Documentation Index

This directory separates the docs by audience. Do not mix from-scratch agent prompts, implementation
notes, and human run instructions in the same file.

## Read This First

- For running or operating the service, start with `../README.md`.
- For CLI productization work, read `CLI_PRODUCT_PLAN.md`.
- For changing API behavior, start with `SPEC.md`, then update `openapi.yaml`.
- For changing Rust code, read `RUST_ARCHITECTURE.md`.
- For handing work to another agent, use `AGENT_HANDOFF.md`.

## Files

- `SPEC.md` - stack-neutral behavior and API contract. This is the semantic source of truth.
- `openapi.yaml` - static machine-readable API contract for downstream callers and client
  generation.
- `RUST_ARCHITECTURE.md` - current Rust implementation notes, including stack, storage, commands,
  and test boundaries.
- `CLI_PRODUCT_PLAN.md` - target CLI experience and phased implementation plan.
- `PLAN.md` - maintenance roadmap and future backlog. It is not a prompt to rebuild the service
  from scratch.
- `AGENT_HANDOFF.md` - concise context and workflow for future coding agents.
- `contracts/` - example JSON request and response bodies.
- `examples/` - minimal Cherri source and a sample build request.

## Contract Rules

- Keep `SPEC.md` and `openapi.yaml` aligned.
- Keep `SPEC.md` generic; implementation details belong in `RUST_ARCHITECTURE.md`.
- Keep README run commands exact and copy-pasteable.
- Do not add Tsugi-specific request fields or business-token concepts to server docs.
