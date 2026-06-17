# Daemon

The daemon polls thermal sensors on a fixed interval and writes readings to SQLite.

## Usage

```bash
tempcheck daemon --db tempcheck.db --interval-secs 30
```

## Behavior

- Reads all available sensors from:
  - `thermal_zone*` entries under `/sys/class/thermal`
  - `temp*_input` entries under `/sys/class/hwmon`
  - NVIDIA GPUs via `nvidia-smi` when installed
- Inserts one row per sensor per tick
- Logs structured events to stderr via `tracing`
- Shuts down cleanly on SIGINT (Ctrl+C)

## Configuration

| Flag | Default | Description |
|------|---------|-------------|
| `--db` | `tempcheck.db` | SQLite database path |
| `--interval-secs` | `30` | Seconds between polls (must be > 0) |

## Docker

```bash
docker compose up daemon
```

Mounts thermal sysfs read-only and persists data in a named volume.

## Related

- [Architecture](../architecture.md)
- [MCP server](mcp-server.md)
