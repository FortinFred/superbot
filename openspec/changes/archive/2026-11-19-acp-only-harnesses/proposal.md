# Proposal

## Why

We maintain four hand-written one-shot adapters (Claude, Codex, Copilot, Pi), each a thin JSON-dialect shim that must be kept in sync with each CLI's flags and output format. The Agent Client Protocol (ACP) now covers everything those adapters hand-roll — session lifecycle, streaming, tool calls, permission requests — natively in Copilot CLI and OpenCode, and via adapters for the rest. One ACP integration replaces four bespoke ones, and any future ACP agent works with zero server changes.

## What Changes

- **BREAKING**: The server no longer runs harness CLIs in one-shot mode (`--output-format json` / `-r <id>`). All agent runs go through ACP: the server acts as an ACP client over stdio JSON-RPC 2.0.
- **BREAKING**: The four harness adapters (`claude`, `codex`, `copilot`, `pi`) are removed. In their place, a single ACP integration. An agent is defined by its launch command (e.g. `copilot --acp`, `opencode acp`), not by Rust code. Existing Bots whose harness no longer resolves must be reassigned.
- The built-in agent list becomes: Copilot (`copilot --acp`), OpenCode (`opencode acp`). Others (Pi via `pi-acp`, Claude via `claude-agent-acp`, Codex via its ACP adapter) can be added by configuration as launch commands, unsupported officially.
- Per run: spawn the agent over stdio (TCP is out of scope for this first iteration), `initialize`, `session/new` (or `session/load` to resume), send the prompt, stream notifications until stop, auto-approve every `session/request_permission` (self-hosted server, same reach as today's `--allow-all`), kill the process at end of run. Abort maps to `session/cancel`.
- Session resume keeps working: the ACP session id is stored per Bot, as today.
- Model lists stay env-driven (`COPILOT_MODELS`, BYOK vars); ACP standardizes modes, not model listing.

## Capabilities

### New Capabilities
- `acp-run`: the server's ACP client behavior — process lifecycle per run, session resume, prompt streaming, event mapping, permission auto-approval, abort handling.
- `acp-agents`: which ACP agents the server can run — built-in entries (Copilot, OpenCode), user-configured launch commands, availability checks, model options per agent.

### Modified Capabilities
- None (no specs exist yet; this is the first change to define them).

## Impact
- `server/src/adapters/`: `claude.rs`, `codex.rs`, `copilot.rs`, `pi.rs`, and the `Adapter` trait in `mod.rs` are deleted, replaced by one ACP module. The shared spawn/stream plumbing is rewritten for bidirectional JSON-RPC (requests must be answered, not just read).
- `server/src/runner.rs`: harness validation, run loop, lost-session recovery, and event handling adapt to ACP shapes (`stop`, `agent_message_chunk`, tool-call updates, permission requests).
- `server/src/db.rs`: the `harness` column now stores an agent id; migration or fallback for old values.
- Web app: harness picker becomes agent picker (Copilot, OpenCode, configured extras); model/effort pickers unchanged in shape.
- Docs: `docs/backend.md` rewritten for ACP; an ADR records the drop of non-ACP support.
- `.env.example`: unchanged BYOK vars, minus one-shot-only flags.
