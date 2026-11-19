# 0004: One protocol, ACP, instead of four harness adapters

Date: 2026-12-02. Status: accepted.

## Decision

- The server speaks only the Agent Client Protocol (ACP), JSON-RPC 2.0 over stdio. `server/src/acp.rs` is the only place that runs an agent.
- The four one-shot adapters (`claude`, `codex`, `copilot`, `pi`) are gone, with their JSON dialects, flags, and per-harness quirks.
- An agent is data: an id, a display name, and a launch command. Copilot (`copilot --acp`) and OpenCode (`opencode acp`) ship built in; anything else goes in `ACP_AGENTS=id:Name:command`.
- One process per turn: `initialize`, `session/new` or `session/load`, `session/prompt`, stream until the turn ends, kill. No long-lived daemon.
- Every `session/request_permission` is approved automatically, matching the old `--allow-all` reach.
- Stdio only for now. ACP over TCP is a later addition.

## Why

- Four adapters meant four parsers, four session schemes, four sets of CLI flags to track, and four ways to break on a CLI upgrade. ACP standardizes the parts we kept re-implementing: session lifecycle, streaming, tool calls, permissions.
- Copilot and OpenCode implement ACP natively (Copilot since Jan 2026, in public preview). New agents cost a config line instead of a Rust file.
- It matches the "harness-specific code lives only in its adapter" rule better than before: now there is no harness-specific code at all.

## Limits

- Copilot's ACP support is a public preview; its message shapes can change.
- Claude Code, Codex, and Pi have no native ACP server. They need community bridges (`claude-agent-acp`, `pi-acp`, Codex's adapter) configured through `ACP_AGENTS`. Those are third-party and unsupported here.
- ACP standardizes modes, not models: the model picker stays env-driven (`COPILOT_MODELS`, `<AGENT>_MODEL`). Reasoning effort and context tiers are saved on the Bot but not applied, because the protocol has no field for them.
- ACP has no system-prompt field, so identity and memory are still prepended to the prompt text every run.
- Bots saved with an old harness value (`claude`, `codex`, `pi`) must be reassigned; the run says so instead of guessing.
