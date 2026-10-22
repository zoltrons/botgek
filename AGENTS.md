# Agent instructions

This file holds the repo configuration consumed by the engineering skills.

## Agent skills

### Issue tracker

GitHub Issues in `zoltrons/botgek`, driven with the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default five triage roles, where each label string is the role name. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `GLOSSARY.md` at the repo root, architecture decisions in `docs/adr/`. See `docs/agents/domain.md`.

## Bot MCP server

The bot serves a development/debug MCP server at `/mcp` on the WebUI address, guarded by
`MCP_TOKEN`. Setup, client registration, call examples, tools, and limits: [docs/MCP.md](docs/MCP.md).
