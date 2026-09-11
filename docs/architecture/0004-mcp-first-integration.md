# ADR 0004: MCP-first external-agent integration

Status: accepted
Date: 2026-09-11

## Context

External agents need a supported entry point into Cut without configuring an
embedded provider or depending on internal runtime contracts.

## Decision

- `cutd` remains the only owner of project state, revisions, history,
  idempotency receipts, proposals, and render jobs.
- `cut-mcp` is the public agent boundary. Internal IPC and the typed domain API
  remain implementation details behind the adapters.
- Provider APIs are optional for Cut's embedded agent rail and terminal agent
  workflow. External MCP clients use their own agents and need no Cut provider
  API key.
- Stdio is the first transport. Remote HTTP transport can be future work;
  this milestone adds no remote service.
- Proposal/review and tool allowlist safety remain mandatory. MCP integration
  does not authorize bypassing review or broaden allowed operations.
- `cut-mcp --print-config` emits a generic client configuration with absolute
  executable and project paths, without starting `cutd` or mutating a project.

## Consequences

External clients can configure Cut directly through MCP while the native
editor and embedded agent retain the existing daemon and proposal architecture.
Inbound MCP attachments remain available to the embedded agent. Transport
changes must preserve the same state ownership and safety boundaries.
