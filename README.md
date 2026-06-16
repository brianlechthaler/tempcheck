# tempcheck

Rust daemon that reads Linux thermal sensors, logs readings to SQLite, and exposes an MCP server for live and historical temperature queries.

## Quick start

```bash
# Build and test (Docker only — no host Rust required)
make docker-build
make docker-test

# Run the logging daemon
docker compose run --rm daemon

# MCP server (stdio) for Cursor/agents
tempcheck mcp --db tempcheck.db
```

See [Getting started](docs/getting-started.md) for setup details.

## Documentation

- [Getting started](docs/getting-started.md)
- [Architecture](docs/architecture.md)
- [Daemon](docs/features/daemon.md)
- [MCP server](docs/features/mcp-server.md)
- [Security threat model](docs/security/threat-model.md)
- [Security audit findings](docs/security/audit-findings.md)

## Requirements

- Linux with `/sys/class/thermal` (for live readings)
- Docker (for build, test, lint)
- Optional: `TEMPCHECK_MCP_TOKEN` when MCP auth is enabled

## License

[MIT](LICENSE)
