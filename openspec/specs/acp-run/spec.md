# acp-run Specification

## Purpose
Defines how the server drives an ACP agent to answer a Bot's prompt: process lifecycle, session resume, streaming, permissions, and abort. The first iteration supports the stdio transport only; ACP over TCP is out of scope.

## Requirements

### Requirement: Run a prompt over ACP
For each Bot turn, the server SHALL start the agent's ACP process (stdio JSON-RPC 2.0), perform the `initialize` handshake, create or resume a session, and send the prompt as one `session/prompt`. The server SHALL NOT keep an ACP process alive between turns.

#### Scenario: New session
- **WHEN** a Bot with no stored ACP session id gets a prompt
- **THEN** the server calls `session/new` in the Bot's working directory, stores the returned session id on the Bot, and sends the prompt

#### Scenario: Resumed session
- **WHEN** a Bot has a stored ACP session id
- **THEN** the server calls `session/load` with that id before sending the prompt, so the agent's own context continues

#### Scenario: Process ends with the turn
- **WHEN** the agent reports the end of the turn (`stop`)
- **THEN** the server stops reading events and terminates the ACP process

### Requirement: Stream agent events to the web app
The server SHALL map ACP notifications to the existing event stream: text chunks to streamed text, finished messages to chat bubbles, and tool updates to tool events. The user-visible stream SHALL look the same as before the ACP switch.

#### Scenario: Streaming reply
- **WHEN** the agent emits `agent_message_chunk` notifications
- **THEN** the web app shows the reply building up live, as one chat bubble per finished message

#### Scenario: Tool use
- **WHEN** the agent emits a tool-call update
- **THEN** the web app shows which tool the agent is using, with its arguments

### Requirement: Auto-approve permission requests
The server SHALL approve every `session/request_permission` the agent sends, without asking the user. The server is self-hosted and Bots are pre-authorized, matching the previous `--allow-all` reach of every adapter.

#### Scenario: Agent requests a tool permission
- **WHEN** the agent asks permission to run a tool during a turn
- **THEN** the server responds with approval immediately and the turn continues

### Requirement: Abort a run
The server SHALL cancel an in-flight ACP turn promptly when the user asks, preserving the Bot's session id.

- **WHEN** the user stops a Bot's turn
- **THEN** the server sends `session/cancel` for the current prompt request, stops emitting events, and terminates the ACP process without saving a partial reply as finished

#### Scenario: Stop button
- **WHEN** the user hits stop while the agent is working
- **THEN** the run ends promptly and the Bot's stored session id is preserved for the next turn

### Requirement: Recover from a lost session
The server SHALL recover once from an unresumable session by starting fresh, instead of leaving the Bot broken.

- **WHEN** the agent cannot resume the stored session id (its files were wiped or upgraded away)
- **THEN** the server starts a fresh session once and retries the same prompt, instead of leaving the Bot broken

#### Scenario: Wiped agent state
- **WHEN** `session/load` fails on a Bot with a stored session id and the turn produced no reply
- **THEN** the server clears the stored id, creates a new session, and re-sends the prompt one time

### Requirement: Surface agent failures
The server SHALL surface ACP process, handshake, and mid-turn errors as a clear chat error, without wiping the stored session id on a failed start.

- **WHEN** the ACP process fails to start, errors during the handshake, or reports a fatal error mid-turn
- **THEN** the Bot's chat shows a clear error message and the turn ends; a failed start does not wipe the stored session id

#### Scenario: Handshake failure
- **WHEN** the agent process starts but `initialize` fails or times out
- **THEN** the Bot's chat shows an error naming the agent, and the stored session id is unchanged so the next turn retries resume
