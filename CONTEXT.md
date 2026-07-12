# Shortcut Forge

Shortcut Forge is a generic build/sign/download context for complete Cherri source. The caller owns
business meaning; this context owns turning source into a signed shortcut artifact with tokenized
download access.

## Language

**Build Request**:
A caller-submitted request containing a display name, source format, complete source, signing mode,
and download TTL. It is not a business command and does not carry caller-domain fields.
_Avoid_: Tsugi request, shortcut template request

**Build**:
The durable record identified by the deterministic fingerprint of source format, signing mode, and
source. A build can be ready, failed, or expired for download purposes.
_Avoid_: Job, task

**Build Lifecycle**:
The command-side workflow for accepting a build request, deciding whether it is a rebuild or renewal,
running the build pipeline when needed, and updating artifact metadata and download tokens.
_Avoid_: Build route, state handler

**Build Lifecycle Decision**:
The outcome of evaluating a Build Request against existing build state and the current Toolchain
Fingerprint to decide whether submission proceeds as a Renewal or a Rebuild.
_Avoid_: Submission plan, branch choice

**Build Pipeline**:
The compile-and-sign work that turns complete Cherri source into signed shortcut bytes. It does not
decide build identity, renewal, token issuance, or metadata transitions.
_Avoid_: Build lifecycle

**Renewal**:
A same-fingerprint submission that keeps the deterministic build ID, refreshes mutable metadata and
expiry, and issues a fresh download token without recompiling when the ready artifact and toolchain
fingerprint are still current.
_Avoid_: Rebuild, regenerate

**Rebuild**:
A same-fingerprint submission that keeps the deterministic build ID but reruns the build pipeline
because the artifact is missing, the previous build failed, or the toolchain fingerprint changed.
_Avoid_: Renewal

**Toolchain Fingerprint**:
A digest of the local Cherri and Shortcuts signing probe results used to decide whether an existing
signed artifact is still current for the submitted source.
_Avoid_: Version string

**Download Token**:
A random bearer credential embedded in a download URL and stored only as a hash in build metadata.
It is separate from service authentication.
_Avoid_: Auth token, source token

**Signed Shortcut Artifact**:
The persisted `.shortcut` bytes produced by the build pipeline and served through a valid download
token while the build has not expired.
_Avoid_: Source, metadata
