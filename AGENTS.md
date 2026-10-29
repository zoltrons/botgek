# Agent instructions

This file holds the repo configuration consumed by the engineering skills.

## Start here (navigation, minimal tokens)

The source is ~43k lines; do not explore it blind. In this order:

1. [docs/agents/MAP.md](docs/agents/MAP.md): find your task in the "Görev ─► ilk bakılacak yer" table. It names the
   starting file, the module doc, a grep anchor, and the narrowest verify command.
2. Read only the relevant section of that module doc (`Değişmezler`, `Değiştirirken`).
3. `scripts/outline.sh <file>` gives a file's item outline with line numbers. Then `Read` with `offset`/`limit`.
   Never read a file over ~1000 lines whole. Other modes: `--tree`, `--find NAME`, `--refs NAME`, `--check`.
4. Working method, search recipes, verify matrix, pitfalls: [docs/agents/WORKFLOW.md](docs/agents/WORKFLOW.md).

If you add, move, or delete a source file or module doc, update MAP.md and run `scripts/outline.sh --check`.

## Agent skills

### Issue tracker

GitHub Issues in `zoltrons/botgek`, driven with the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default five triage roles, where each label string is the role name. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `GLOSSARY.md` at the repo root, architecture decisions in `docs/adr/`. See `docs/agents/domain.md`.

## Module docs

Per-module internals (files, invariants, change checklists) live in [docs/modules/README.md](docs/modules/README.md).
Read the matching module doc before changing a module.

## Bot MCP server

The bot serves a development/debug MCP server at `/mcp` on the WebUI address, guarded by
`MCP_TOKEN`. Setup, client registration, call examples, tools, and limits: [docs/MCP.md](docs/MCP.md).
