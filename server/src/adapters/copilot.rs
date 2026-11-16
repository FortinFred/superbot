//! GitHub Copilot CLI: `copilot -p <prompt> --output-format json`.
//! A new session takes `--session-id <uuid>`; resuming uses `-r <id>`.
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
        let prompt = format!("{}\n\n---\n\n{}", input.system.trim(), input.prompt);
        let mut a: Vec<String> = vec![
            "-p".into(),
            prompt,
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
        if let Some(e) = &input.effort {
            a.extend(["--reasoning-effort".into(), e.clone()]);
        }
        a
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
    /// interactive session), so offer what we can prove works: Auto, the BYOK
    /// model from the environment, and the model saved in the CLI's own settings.
    fn models(&self) -> Vec<ModelOption> {
        let efforts: Vec<String> = ["none", "minimal", "low", "medium", "high", "xhigh", "max"].map(String::from).into();
        let mut list = vec![ModelOption {
            id: "auto".into(),
            label: "Auto (Copilot picks)".into(),
            efforts: efforts.clone(),
            aliases: vec![],
        }];
        let configured = std::env::var("COPILOT_MODEL")
            .ok()
            .filter(|m| !m.trim().is_empty())
            .or_else(|| Some(output("copilot", &["config", "model"]).trim().to_string()).filter(|m| !m.is_empty()));
        if let Some(m) = configured.filter(|m| m != "auto")
            && !list.iter().any(|o| o.id == m)
        {
            list.push(ModelOption { label: m.clone(), id: m, efforts, aliases: vec![] });
        }
        list
    }
}
