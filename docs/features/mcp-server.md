# MCP server

stdio MCP server built with `rmcp` 0.16. Agents query live sensors and historical SQLite data.

## Start

```bash
tempcheck mcp --db tempcheck.db --audit-log /var/log/tempcheck/audit.jsonl
```

## Tools (version-pinned)

### get_current_temperature@1

Reads live values from `/sys/class/thermal`. No database required.

Parameters:

| Field | Type | Required |
|-------|------|----------|
| `auth_token` | string | When `TEMPCHECK_MCP_TOKEN` is set |

### analyze_temperature@1

Aggregates stored readings over a time window.

Parameters:

| Field | Type | Default |
|-------|------|---------|
| `from` | ISO-8601 string | 24 hours ago |
| `to` | ISO-8601 string | now |
| `sensor` | string | all sensors |
| `auth_token` | string | — |

Returns per-sensor `min_c`, `max_c`, `avg_c`, `latest_c`, and `count`.

## Security controls

- Optional `TEMPCHECK_MCP_TOKEN` — fail-closed per call
- JSON Schema validation on all parameters
- SHA-256 parameter fingerprints in audit log (no raw secrets)
- 256 KiB response cap
- 10,000 row analysis limit

## Audit log

Each invocation emits JSONL events:

- `mcp.tool.invoke` — before execution
- `mcp.tool.result` — after execution (status, byte count)

## Cursor configuration

```json
{
  "mcpServers": {
    "tempcheck": {
      "command": "/usr/local/bin/tempcheck",
      "args": ["mcp", "--db", "/data/tempcheck.db"]
    }
  }
}
```

## Related

- [Threat model](../security/threat-model.md)
- [Architecture](../architecture.md)
