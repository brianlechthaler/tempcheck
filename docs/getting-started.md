# Getting started

## Prerequisites

- Docker and Docker Compose
- Linux host with thermal sysfs for production sensor reads

## Build

```bash
make docker-build
```

## Run the daemon

Background logging to SQLite (default 30s interval):

```bash
docker compose up daemon
```

Or locally after building:

```bash
cargo run -- daemon --db tempcheck.db --interval-secs 30
```

Stop with Ctrl+C.

## One-shot reading

```bash
cargo run -- once
cargo run -- once --save --db tempcheck.db
```

## MCP server

Add to Cursor MCP config (stdio):

```json
{
  "mcpServers": {
    "tempcheck": {
      "command": "tempcheck",
      "args": ["mcp", "--db", "/path/to/tempcheck.db"],
      "env": {
        "TEMPCHECK_MCP_TOKEN": "your-secret-token"
      }
    }
  }
}
```

When `TEMPCHECK_MCP_TOKEN` is set, pass `auth_token` in tool arguments.

## Development

```bash
make docker-test      # unit + integration tests
make docker-lint      # fmt + clippy
make docker-coverage  # llvm-cov summary
make docker-shell     # dev container shell
```

## Troubleshooting

| Problem | Fix |
|---------|-----|
| `thermal sysfs path not found` | Run on Linux or mount `/sys/class/thermal` read-only into the container |
| MCP auth denied | Set `TEMPCHECK_MCP_TOKEN` and pass matching `auth_token` |
| Empty analysis results | Ensure the daemon has been writing to the same `--db` path |
