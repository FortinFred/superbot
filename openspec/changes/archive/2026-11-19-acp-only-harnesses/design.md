# Design

## Context

Today `server/src/adapters/mod.rs` defines an `Adapter` trait (bin + args + stdin + `on_event`) and four one-shot implementations; `runner.rs` spawns each turn's process, saves `AgentEvent::Session` ids, and retries once when a session is lost. The web app picks a harness from a fixed list and a model per harness. ACP replaces all of this with one stdio JSON-RPC 2.0 client in the server. First iteration: stdio only, no TCP.

## Goals / Non-Goals

**Goals:**
- One generic ACP run path in Rust, no per-agent Rust code; agents are data (id, name, launch command, env).
- Preserve observable behavior: live streaming, session resume, tool events, abort, lost-session recovery, env-driven model lists.
- Minimal dependency surface; hand-rolled JSON-RPC loop, no SDK crate.

**Non-Goals:**
- ACP over TCP (`--port`) — later, unchanged shape.
- A model-listing protocol (ACP standardizes modes, not models); lists stay env-driven.
- An in-server permission UI; permissions are always auto-approved.
- Long-lived per-Bot agent daemons.

## Decisions

**One process per turn, not a persistent daemon.** Spawn on prompt, `initialize`, `session/new`/`session/load`, `session/prompt`, read until `stop`, kill. Matches the existing runner lifecycle (process group + kill-on-drop already there) and needs no idle process management. Alternative: a persistent per-Bot ACP process — rejected for now: more state to babysit, and agents persist sessions themselves so resume works across spawns.

**Agents are config, not code.** A small table: Copilot (`copilot --acp`), OpenCode (`opencode acp`), plus user-declared entries (command + name, e.g. `npx pi-acp`). `NAMES`/`get()` and the four adapter files disappear. Alternative: keep per-agent code — rejected: that's what we're removing. Availability = resolve the launch command like `adapters::find` does today.

**Bidirectional JSON-RPC in the run loop.** The current adapter contract is write-once/read-lines; ACP needs answers mid-stream. One reader task parses lines: notifications map to `AgentEvent`s; `session/request_permission` is answered immediately with all outcomes approved (the ACP equivalent of today's `--allow-all`); responses match pending request ids. Writing goes through a channel so the reader can reply. Keep emitting the same `AgentEvent` stream so `runner.rs` and the web app change little.

**System prompt stays prepended.** `session/new` has no standard system-prompt field. Keep the current Copilot trick: prepend the Bot's identity/rules/memory text to the prompt. Revisit if ACP standardizes one.

**Models and efforts stay env-driven.** Copilot: `COPILOT_MODELS`, BYOK `COPILOT_*`, saved CLI default. OpenCode: its own env vars if any, otherwise "Auto". Store model choice per Bot as today; apply via the agent's own env/config (`COPILOT_MODEL` etc.), not protocol fields, since ACP has none.

**Session resume: `session/load`, fallback fresh.** ACP session ids replace harness session ids in the same `bots.session_id` column. Lost-session recovery keeps its current shape: if `session/load` fails before any reply, clear the id, `session/new`, resend once.

**DB keeps the `harness` column, values become agent ids.** No schema change; old values (`claude`, `codex`, `pi`) simply stop resolving and the run tells the user to reassign the Bot. Renaming the column buys nothing.

## Risks / Trade-offs

- [Copilot ACP is public preview; protocol may shift] → Pin nothing; treat `stop`/chunk/permission shapes defensively; bump against releases.
- [Bridges (pi-acp, claude-agent-acp) are community projects] → Supported only as user-configured launch commands; no promise, no vendoring.
- [A failed `initialize` leaves a Bot unusable with an unclear cause] → Map handshake/process errors to one clear chat error, like today's "Couldn't start ...".
- [Reader/writer interleaving bugs (unmatched request ids)] → Single reader task owns parsing; writer only responds via channel; add a fake-agent integration test exercising the full sequence.
- [Model/effort fidelity regresses vs one-shot flags] → Acceptable: env vars cover the Copilot BYOK path we actually use; revisit when ACP standardizes modes enough.

## Migration Plan

1. Land the ACP run path and agent table; delete the four adapters and the old trait in the same change (no dual-stack — the point is to drop non-ACP).
2. Docs: rewrite `docs/backend.md` harness section; add an ADR recording the ACP-only decision.
3. Rollback: `git revert` restores the one-shot adapters; no data migration to undo (`session_id` contents are just ids).
