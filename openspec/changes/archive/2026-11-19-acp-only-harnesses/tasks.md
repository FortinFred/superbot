# Tasks

## 1. Agent table and ACP client core

- [x] 1.1 Add the agent registry (built-ins `copilot --acp`, `opencode acp`; user-configured name+command from env/config) and availability check resolving the launch command via PATH + user bin dirs; verify a unit test lists only agents whose command exists
- [x] 1.2 Implement the stdio JSON-RPC 2.0 run loop in a new `server/src/acp.rs`: spawn process group, `initialize`, `session/new`/`session/load`, `session/prompt`, reader task with a writer channel, kill at turn end; verify with a fake ACP agent script (shell/Python) that echoes a canned handshake→chunks→stop sequence
- [x] 1.3 Map ACP notifications to the existing `AgentEvent` stream (`agent_message_chunk`→TextDelta, message stop→Text, tool-call updates→Tool) and auto-approve every `session/request_permission`; verify the fake-agent test asserts a permission reply was written and events reached the caller
- [x] 1.4 Handle failure paths in the loop: process won't start, handshake error, agent error mid-turn, and abort (`session/cancel` then kill); verify each with the fake agent exiting or misbehaving at the right moment

## 2. Runner and persistence integration

- [x] 2.1 Replace `adapters::get`/`adapters::run` in `runner.rs` with the ACP run path; keep session-id saving from `session/new` and the one-shot lost-session recovery (load fails, no reply → clear id, new session, resend); verify with the fake agent rejecting `session/load`
- [x] 2.2 Reject unknown or uninstalled agents with a clear chat message telling the user to reassign the Bot (covers legacy `claude`/`codex`/`pi` values); verify with a bot row whose harness is "claude" and no matching agent
- [x] 2.3 Apply model/effort settings via agent env passthrough (`COPILOT_MODEL`, BYOK vars) and keep env-driven model lists per agent; verify the model picker for a Copilot bot shows Auto plus `COPILOT_MODELS` entries when set

## 3. Web app

- [x] 3.1 Replace the harness picker with an agent picker fed by the server's agent list endpoint (installed agents only); verify creating a Bot with each installed agent and that a legacy-harness Bot shows the reassignment error when prompted

## 4. Cleanup, docs, and ADR

- [x] 4.1 Delete `server/src/adapters/claude.rs`, `codex.rs`, `copilot.rs`, `pi.rs` and the old `Adapter` trait/`NAMES` plumbing; verify `cargo build` and the full test suite pass with no adapter references left (`rg adapters server/` clean of dead paths)
- [x] 4.2 Rewrite the harness section of `docs/backend.md` for ACP-only (stdio, per-turn spawn, permission auto-approval, env-driven models, bridges as user config) and update `.env.example`; verify each documented command/env var exists in code
- [x] 4.3 Record the decision in `docs/adr/0004-acp-only.md` (drop non-ACP harnesses, stdio first, agents as config); verify the ADR follows the format of the existing ADRs

## 5. Integration check

- [x] 5.1 End-to-end with a real installed agent (Copilot or OpenCode): prompt a Bot, watch streamed reply and tool events, stop a run mid-turn, restart the server and resume the same Bot session; verify each step behaves as the specs describe
