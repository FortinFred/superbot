# Architecture (draft)

Status: proposal. Nothing is decided until it has an ADR in `docs/adr/`.

## Goal

A self-hosted Grok Bot alternative that runs on any agent harness.

## Agent agnostic

The agent is the thing that does the work. We do not build our own agent loop.

We speak one protocol: the Agent Client Protocol (ACP), JSON-RPC over stdio. An agent is a launch command, so any ACP agent works without a server change (see `docs/adr/0004-acp-only.md`).

Built in:

1. Copilot CLI (`copilot --acp`)
2. OpenCode (`opencode acp`)

Configured through `ACP_AGENTS` (bridges, unsupported by us): Pi (`pi-acp`), Claude Code (`claude-agent-acp`), Codex (its ACP adapter).

One integration covers them all:

- starts or resumes an agent session for a Bot
- sends a message in
- streams events out (text, tool calls, approvals, done)
- passes in the Bot's memory as prompt text, since ACP has no system-prompt field

Everything else is shared and never knows which agent is running.

## Proposed parts

- **Web app**: chat with Bots, watch their screen, approve actions.
- **Server**: owns Bots, memory, skills, routines, approvals, and the Vault.
- **Agents**: one ACP client (`server/src/acp.rs`), configured, not coded.
- **Computer**: a Docker container on the VPS with a Linux desktop, browser, and terminal. Each Bot gets its own screen.

Language: Rust first, one Cargo workspace. Server, adapters, computer daemon, and CLI are Rust. The web UI uses as little JS as possible (Rust to WASM if it works well). Why: fast, low memory on a shared VPS (RAM is the bottleneck, see `background/grok-bot-computer-specs.md`), and one binary to deploy.

## Open decisions

1. Web UI: Rust to WASM (e.g. Leptos, Dioxus), or a thin JS frontend? The rest is Rust.
2. One shared computer for all Bots, or one per Bot?
3. v1 chat channel: web, Slack, Telegram, or desktop?
4. Single user, or teams with logins?
5. First end-to-end demo job?
