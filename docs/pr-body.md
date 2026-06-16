## Summary
- Rust daemon polls Linux thermal sysfs and logs readings to SQLite
- MCP stdio server exposes `get_current_temperature@1` and `analyze_temperature@1` with optional token auth, JSON Schema validation, and JSONL audit events
- Docker-based dev/test/lint, GitHub Actions CI (test, lint, GHCR container), and security documentation

## Test plan
- [x] `make docker-test` — 40 tests pass (36 unit + 4 integration)
- [x] `make docker-lint` — fmt + clippy clean
- [x] `make docker-coverage` — ~90% line coverage on library code
- [x] Security audit documented in `docs/security/audit-findings.md` (0 critical/high)
- [x] CI passes on GitHub Actions
