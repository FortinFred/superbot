//! GitHub Copilot CLI: `copilot --output-format json`.
//! A new session takes `--session-id <uuid>`; resuming uses `-r <id>`.
//! The prompt goes in on stdin, never as an argument: quotes, backticks, `$`, very
//! long text, and shell or WSL interop layers then have nothing to mangle.
//! Limitation: the CLI has no flag to append a system prompt, so the Bot's system
//! prompt is prepended to the prompt text. It is rebuilt every run, so memory
//! changes still reach the Bot.

use super::{describe, new_uuid, output, str_of, Adapter, AgentEvent, ModelOption, RunInput};
use serde_json::Value;

pub struct Copilot;

impl Adapter for Copilot {
    fn bin(&self) -> &'static str {
        "copilot"
    }

    fn args(&mut self, input: &RunInput, out: &mut Vec<AgentEvent>) -> Vec<String> {
        let mut a: Vec<String> = vec![
            "--output-format".into(),
            "json".into(),
            // Non-interactive runs must pre-approve tools; same reach as the other adapters.
            "--allow-all".into(),
            // The server is self-hosted: no telemetry leaves it on our behalf.
            "--disable-client-telemetry".into(),
        ];
        match &input.session_id {
            Some(id) => a.extend(["-r".into(), id.clone()]),
            None => {
                let id = new_uuid();
                out.push(AgentEvent::Session(id.clone()));
                a.extend(["--session-id".into(), id]);
            }
        }
        if let Some(m) = &input.model {
            a.extend(["--model".into(), m.clone()]);
        }
        // `auto` routes internally and refuses any effort ("does not support
        // reasoning effort configuration"); only explicit models take one.
        if let Some(e) = &input.effort
            && input.model.as_deref() != Some("auto")
        {
            a.extend(["--reasoning-effort".into(), e.clone()]);
        }
        if let Some(c) = &input.context {
            a.extend(["--context".into(), c.clone()]);
        }
        a
    }

    fn stdin(&mut self, input: &RunInput) -> Option<String> {
        Some(format!("{}\n\n---\n\n{}", input.system.trim(), input.prompt))
    }

    fn on_event(&mut self, e: &Value, out: &mut Vec<AgentEvent>) {
        let d = &e["data"];
        match str_of(e, "type") {
            "assistant.message_delta" => {
                let delta = str_of(d, "deltaContent");
                if !delta.is_empty() {
                    out.push(AgentEvent::TextDelta(delta.into()));
                }
            }
            "assistant.message" => {
                // Tool requests are reported by `tool.execution_start`, which has parsed arguments.
                let text = str_of(d, "content");
                if !text.trim().is_empty() {
                    out.push(AgentEvent::Text(text.into()));
                }
            }
            "tool.execution_start" => {
                out.push(AgentEvent::Tool { name: str_of(d, "toolName").into(), detail: describe(&d["arguments"]) })
            }
            "session.error" => {
                let msg = str_of(d, "message");
                out.push(AgentEvent::Error(if msg.is_empty() { "Copilot failed".into() } else { msg.into() }));
            }
            _ => {}
        }
    }

    /// The CLI has no command to list models (`/model` only exists inside an
    /// interactive session), and its own catalog answers `auto` for a plan that
    /// routes internally: `copilot -p ok --model gpt-6-luna` is refused even when
    /// the router just used that model. So offer what is provably usable:
    /// Auto, the ids in COPILOT_MODELS, the BYOK COPILOT_MODEL, and the model
    /// saved in the CLI's own settings. Auto advertises no efforts: it picks the
    /// model and the effort itself, and rejects `--reasoning-effort`.
    fn models(&self) -> Vec<ModelOption> {
        let efforts: Vec<String> = ["none", "minimal", "low", "medium", "high", "xhigh", "max"].map(String::from).into();
        // `--context` takes default or long_context; which of them a model really
        // offers is only in the CLI's model picker, so both are offered here.
        let contexts: Vec<String> = ["default", "long_context"].map(String::from).into();
        let mut list = vec![ModelOption {
            id: "auto".into(),
            label: "Auto (Copilot picks)".into(),
            efforts: vec![],
            contexts: contexts.clone(),
            aliases: vec![],
        }];
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let configured = env("COPILOT_MODELS")
            .map(|v| v.split(',').map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).collect::<Vec<_>>())
            .unwrap_or_default();
        let configured = configured
            .into_iter()
            .chain(env("COPILOT_MODEL"))
            .chain(Some(output("copilot", &["config", "model"]).trim().to_string()).filter(|m| !m.is_empty()));
        for m in configured.filter(|m| m != "auto") {
            if !list.iter().any(|o| o.id == m) {
                list.push(ModelOption { label: m.clone(), id: m, efforts: efforts.clone(), contexts: contexts.clone(), aliases: vec![] });
            }
        }
        list
    }
}
