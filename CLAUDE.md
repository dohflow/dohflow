@AGENTS.md

## Claude Code-specific instructions

- Treat `AGENTS.md` as the authoritative shared instruction file.
- Do not duplicate shared rules here; update `AGENTS.md` instead.
- Use `/memory` when needed to confirm which CLAUDE.md, CLAUDE.local.md, and rule files are loaded.
- Use plan mode for large refactors, destructive operations, migrations, security-sensitive work, unclear tasks, or work touching financial/user data.
- Follow the non-destructive policy in `AGENTS.md` even if a tool permission would technically allow the action.
- **Invoking `bd`:** always write `\bd ...`, not `bd ...`. The `bd=br` alias that made this mandatory is gone (removed 2026-09-06 with `br` itself), but the backslash costs nothing and the MCP Agent Mail installer recreates the alias if it is ever re-run without `--skip-beads`. Verify with `\bd --version` → `1.2.2` (Homebrew core). Treat the `bd ...` examples in the auto-generated Beads Issue Tracker section below as `\bd ...`. **`bd doctor`, `bd sql` and `bd history` do not work in embedded mode; `bd dolt push` does not reach the remote; and `bd export` silently drops memories unless you pass `--include-memories`** — see AGENTS.md §9.0 before debugging any of them.


<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:ca08a54f -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

## Session Completion

**When ending a work session**, you MUST complete ALL steps below. Work is NOT complete until `git push` succeeds.

**MANDATORY WORKFLOW:**

1. **File issues for remaining work** - Create issues for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **PUSH TO REMOTE** - This is MANDATORY:
   ```bash
   git pull --rebase
   bd dolt push
   git push
   git status  # MUST show "up to date with origin"
   ```
5. **Clean up** - Clear stashes, prune remote branches
6. **Verify** - All changes committed AND pushed
7. **Hand off** - Provide context for next session

**CRITICAL RULES:**
- Work is NOT complete until `git push` succeeds
- NEVER stop before pushing - that leaves work stranded locally
- NEVER say "ready to push when you are" - YOU must push
- If push fails, resolve and retry until it succeeds
<!-- END BEADS INTEGRATION -->
