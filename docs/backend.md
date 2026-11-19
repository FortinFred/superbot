# Backend

One Rust binary, `botd`. It serves the web app, stores everything in SQLite, and runs each Bot on a coding-agent CLI.

## How it works

- **Bots** live in SQLite. Each Bot has an ACP agent (`copilot`, `opencode`, or a configured one), an optional model, and an agent session id.
- **A message** is queued. Each Bot runs one task at a time, in order. Different Bots run in parallel.
- **A run** starts the agent over ACP (JSON-RPC 2.0 on stdio), streams its updates, and saves each text block as a chat bubble.
- **Live updates** go to the browser over one Server-Sent Events stream: new messages, streaming text, and what the Bot is doing ("Running: ls").
- **Memory** is plain files, re-sent in the system prompt every run. The Bot edits them itself:
  - `DATA_DIR/bots/<id>/MEMORY.md`: one Bot's job, rules, and work log.
  - `DATA_DIR/USER.md`: facts about the user, shared by all Bots.
- **The computer** is `WORKSPACE_DIR`, shared by all Bots, like Grok Bot's shared VM.
- **Routines** are cron schedules. Bots create them with the `botctl` CLI, which works from any agent's shell.
- **Bot to Bot**: a Bot runs `botctl ask "<Bot>" "<message>"`. The other Bot answers in its own chat and the reply prints back. If it takes over 9 minutes, the reply arrives later as a new message.
- **Model and effort** are saved per Bot. The web app shows a faint picker above the input. Unset means the agent default.
- **Choice cards**: a reply ending in a ` ```question ` JSON block becomes a multiple-choice card.
- **Restarts**: queued messages run after a restart. A task that was running is reported as interrupted, not re-run, so it can't repeat an action like sending an email.

## Agents (ACP only)

The server speaks one protocol to every agent: the [Agent Client Protocol](https://agentclientprotocol.com), JSON-RPC 2.0 over stdio. An agent is a launch command, not Rust code, so anything that implements ACP works.

- **Built in**: Copilot (`copilot --acp`) and OpenCode (`opencode acp`). Both are installed and logged in as the user that runs `botd`.
- **Configured**: `ACP_AGENTS=pi:Pi:npx pi-acp@0.0.34,claude:Claude:npx @zed-industries/claude-code-acp` adds more. Bridges are community projects: no promises, but no server change needed either.
- **One process per turn**: `initialize`, `session/new` (or `session/load` to resume), `session/prompt`, read updates until the turn ends, then the process is killed. Sessions live in the agent, so a Bot keeps its context across runs and restarts.
- **Streaming**: `session/update` notifications. Text chunks stream into one chat bubble; `tool_call` updates become the "Running: ..." status.
- **Permissions**: every `session/request_permission` is approved without asking. The server is the computer, and Bots are pre-authorized, so treat it that way.
- **System prompt**: ACP has no field for it, so the Bot's identity, rules, and memory are prepended to the prompt text, rebuilt every run.
- **Models**: ACP standardizes modes, not models. The picker offers Auto plus the ids in `<AGENT>_MODELS` (e.g. `COPILOT_MODELS`) and `<AGENT>_MODEL`, which is also exported to the agent's own env var for the run. Reasoning effort and context tiers are saved but not applied yet: ACP has no field for them.
- **Stdio only** for now. ACP also runs over TCP (`--port`), which is not wired up.

Add an agent: set `ACP_AGENTS` and log the bridge in as the `botd` user.

## Run it

```sh
cargo build --release          # makes target/release/botd and target/release/botctl
cp .env.example .env           # fill it in
set -a; . ./.env; set +a
./target/release/botd
```

Settings are in [.env.example](../.env.example). Keep `botctl` next to `botd`: the server puts that folder on the Bots' PATH.

## Deploying on a Linux VPS

- Keep your server IP, domain, and secrets out of this repo. They go in `.env` on the server.

- Install the agents you want and log them in **as the user that runs `botd`**: `copilot` (`copilot login`, or BYOK through `COPILOT_PROVIDER_BASE_URL` / `COPILOT_PROVIDER_API_KEY` / `COPILOT_MODEL`) and `opencode` (`opencode auth login`). Their env vars pass through to the Bots. Bridges go in `ACP_AGENTS`.
- Run `botd` as a normal user, not root. Claude Code refuses to skip permissions as root.
- Run it under systemd with `Restart=always`. It shuts down cleanly on SIGTERM.
- Put HTTPS in front (e.g. Caddy) and set `AUTH_PASSWORD` and `HOST=0.0.0.0`, or keep `HOST=127.0.0.1` behind the proxy. The browser logs in with any username and that password.
- Proxies must not buffer `/api/events` (it's an SSE stream).
- Set `TZ` to the user's time zone. Routines use it.
- Back up `DATA_DIR`. `WORKSPACE_DIR` is the Bots' scratch and files.
- `GET /healthz` returns `{"ok":true}` without auth.

## API

All JSON. Browser auth: HTTP Basic with `AUTH_PASSWORD`.

- `GET /api/state`: user name, server version, installed agents (`{id, name}`), all Bots
- `GET /api/events`: SSE stream (`message`, `draft`, `status`, `bot`, `botDeleted`, `resync`)
- `POST /api/bots` `{name, harness?, model?, color?, shape?}` (`harness` is the agent id)
- `PATCH /api/bots/:id` `{name?, hidden?, harness?, model?, effort?, context?}`, `DELETE /api/bots/:id`
- `GET /api/agents/:name/models`: models for the picker, from `<AGENT>_MODELS` / `<AGENT>_MODEL` (ACP has no model listing)
- `GET|POST /api/bots/:id/messages` `{text, answer?: {messageId, option}}`
- `POST /api/bots/:id/read`, `POST /api/bots/:id/stop`
- `GET|POST /api/bots/:id/routines` `{name, cron, instruction}`
- `PATCH|DELETE /api/routines/:id` `{enabled?, name?, cron?, instruction?}`
- `GET /api/bots/:id/screen`: starts the Bot's screen and returns its live view URL
- `POST /api/handoff` `{from, text}`: `botctl handoff`; waits for the user's answer

## Not built yet

Approvals before risky actions, Vault for secrets, per-Bot resource limits, Slack.

Computer use (each Bot's screen, browser, and live view) is in [computer.md](computer.md).
