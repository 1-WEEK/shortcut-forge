# Build lifecycle owns build submission

Accepted: the POST `/api/builds` mutation should be owned by a dedicated build lifecycle module with
a small `submit`-style interface. The module owns build identity, per-build locking, rebuild versus
renewal decisions, toolchain freshness checks, token issuance, artifact persistence, and metadata
state transitions; HTTP parsing/auth, metadata lookup, and token download resolution remain outside
that interface.

## Considered Options

- Keep the workflow in `state.rs`: easiest short term, but leaves the most domain-heavy behavior in
  a generic shared-state container.
- Move only the Cherri/Shortcuts calls behind a new module: too shallow, because that is already the
  current build pipeline shape.
- Add storage and toolchain traits at the public interface: more abstract, but speculative while the
  only production adapter is the local filesystem and local process execution.

## Consequences

The build lifecycle module should expose one command-side operation for a validated build request.
The compile/sign pipeline and filesystem helpers can remain concrete internal dependencies tested
with temporary storage and fake tools. This refactor should preserve the existing API contract and
current ready/failed/expired semantics.
