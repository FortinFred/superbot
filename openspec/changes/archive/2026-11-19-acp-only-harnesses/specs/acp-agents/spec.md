# Spec Delta

## Purpose

Defines which ACP agents the server can run and how they are chosen: built-in entries, user-configured extras, availability checks, and model options per agent. Agents are launch commands, not server code.

## ADDED Requirements

### Requirement: Built-in ACP agents
The server SHALL ship with Copilot (`copilot --acp`) and OpenCode (`opencode acp`) as built-in agents, launched over stdio. The server SHALL NOT ship one-shot harness adapters for Claude, Codex, Copilot, or Pi; ACP is the only supported way to run an agent.

#### Scenario: Copilot Bot
- **WHEN** a Bot is set to the Copilot agent and the Copilot CLI is installed and logged in
- **THEN** the server runs its prompts through `copilot --acp` over stdio

#### Scenario: OpenCode Bot
- **WHEN** a Bot is set to the OpenCode agent and `opencode` is installed and logged in
- **THEN** the server runs its prompts through `opencode acp` over stdio

### Requirement: User-configured agents
The server SHALL let the user declare additional ACP agents by launch command (e.g. a community bridge like `pi-acp`), with a display name and an optional env prefix. Configured agents appear in the same picker as built-ins.

#### Scenario: Adding a bridge agent
- **WHEN** the user configures an agent with command `npx pi-acp` and name "Pi"
- **THEN** the agent picker offers "Pi", and a Bot set to it runs its prompts through that command over stdio

### Requirement: Only listed agents are valid
The server SHALL refuse to run a Bot whose agent id is neither built-in nor user-configured, with a clear message.

- **WHEN** a Bot references an agent id that is neither built-in nor configured
- **THEN** the server refuses to run it and tells the user the agent is unknown

#### Scenario: Legacy harness value
- **WHEN** a Bot still has a pre-ACP harness value such as "claude" and no agent with that id exists
- **THEN** the run fails with a clear message telling the user to reassign the Bot to an available agent

### Requirement: Availability check
The server SHALL report which agents are installed (launch command resolvable on PATH or user bin dirs), and the picker SHALL only offer installed agents.

#### Scenario: Missing CLI
- **WHEN** the Copilot CLI is not installed
- **THEN** the agent picker does not offer Copilot, and a Bot already set to it fails with a message about installing it

### Requirement: Model options per agent
The server SHALL offer each agent's models as declared through its env vars, plus an "Auto" entry.

- **WHEN** the user opens the model picker for a Bot
- **THEN** the server offers the models declared for that agent: an "Auto" entry plus env-declared lists (e.g. `COPILOT_MODELS`, BYOK vars) and the agent's saved default, with the agent's supported reasoning efforts where known

#### Scenario: Copilot BYOK model
- **WHEN** `COPILOT_MODEL` is set in the server environment
- **THEN** that model appears in the Copilot Bot's model picker
