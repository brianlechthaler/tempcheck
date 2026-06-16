# Security audit findings

Audit date: 2026-06-15. Scope: full codebase.

## Executive summary

No Critical or High findings. Three Low/Informational items documented below. MCP controls align with NSA CSI MCP guidance (see [threat-model](threat-model.md)).

| Severity | Count |
|----------|-------|
| Critical | 0 |
| High | 0 |
| Medium | 0 |
| Low | 2 |
| Informational | 1 |

## Findings

### SEC-001 — MCP auth optional by default

| Field | Value |
|-------|-------|
| Severity | Low |
| Category | Auth |
| Location | `src/mcp/server.rs`, `TEMPCHECK_MCP_TOKEN` |
| Evidence | `authorize()` returns `Ok(())` when env var unset |
| Impact | Local stdio MCP usable without token on trusted dev machines |
| Remediation | Set `TEMPCHECK_MCP_TOKEN` in production; documented in threat model |

### SEC-002 — SQLite file permissions depend on host umask

| Field | Value |
|-------|-------|
| Severity | Low |
| Category | Data protection |
| Location | `src/storage.rs` |
| Evidence | `Connection::open` does not set restrictive permissions |
| Impact | DB readable by other local users if umask is permissive |
| Remediation | Operator: `chmod 600 tempcheck.db`; container uses dedicated `/data` volume |

### SEC-003 — No per-message MCP signing

| Field | Value |
|-------|-------|
| Severity | Informational |
| Category | MCP integrity |
| Location | Protocol layer |
| Evidence | Ecosystem gap; TLS/stdio only |
| Impact | Intermediary tampering not cryptographically detected |
| Remediation | Documented residual risk R-01 in threat model; use gateway proxy in high-assurance deployments |

## Scanners run

- Manual code review (auth, injection, secrets)
- `grep` for hardcoded credentials — none found
- Dependency review via `cargo` lockfile (rusqlite bundled SQLite, rmcp 0.16)

## Fixed in scope

- Fail-closed MCP auth when token configured
- Parameter JSON Schema validation
- Response size cap (256 KiB)
- Analysis row limit (10,000)
- Structured audit logging with param fingerprints (not raw secrets)
- Non-root container user

**Total: 3 findings (0 critical, 0 high, 0 medium, 2 low, 1 informational)**
