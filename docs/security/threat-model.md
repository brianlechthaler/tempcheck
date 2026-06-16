# Threat model

## Trust boundaries

| Actor | Trust level | Access |
|-------|-------------|--------|
| Host operator | Trusted | Configures daemon, DB path, MCP token |
| AI agent (MCP client) | Untrusted | Tool calls only via stdio |
| SQLite file | Sensitive | Temperature history on disk |

## Data classes

- **Operational**: sensor names, temperatures, timestamps
- **Secrets**: `TEMPCHECK_MCP_TOKEN` (env only, never logged)

## Tool blast radius

| Tool | Max impact if compromised |
|------|---------------------------|
| `get_current_temperature@1` | Read current thermal sysfs (read-only) |
| `analyze_temperature@1` | Read bounded SQLite aggregates (10k rows max) |

## Controls implemented

1. **Gateway trust boundary** — MCP intended behind Cursor/gateway; server binds stdio only
2. **Least privilege** — per-call `auth_token` check; scope `temperature:read`
3. **Audit** — structured JSONL with correlation IDs and param fingerprints
4. **Input validation** — schemars JSON Schema on tool params
5. **Output sanitization** — response size cap; no shell execution
6. **Fail closed** — auth/validation errors deny the call
7. **Tool pinning** — `@1` suffix in descriptions; version field in audit events
8. **OS sandboxing** — Docker runs as non-root; read-only thermal mount

## Residual risks

| ID | Severity | Risk | Mitigation |
|----|----------|------|------------|
| R-01 | Medium | No per-message signing (MCP ecosystem gap) | TLS/stdio local only; document rotation |
| R-02 | Low | SQLite file readable on host | Filesystem permissions; operator hardening |
| R-03 | Low | Token in MCP args visible to client | Use gateway secret injection where possible |

## Operator checklist

- [ ] Set `TEMPCHECK_MCP_TOKEN` in production
- [ ] Restrict DB file permissions (`chmod 600`)
- [ ] Mount `/sys/class/thermal` read-only in containers
- [ ] Ship audit logs to SIEM
- [ ] Do not expose MCP over network without a filtering proxy
