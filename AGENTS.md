# AGENTS.md

Guidance for coding agents working in this repository. See `CLAUDE.md` for the architecture, commands and testing strategy; this file holds the rules that apply to every agent.

## Spinning up workers

When spinning up workers, use `/orchestration` to split the work across parallel child agents. They must show up under the **same workspace in Orca** as the coordinator, so the owner can tab-tile them in one window.
