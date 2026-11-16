//! GitHub Copilot CLI: `copilot --output-format json`.
//! A new session takes `--session-id <uuid>`; resuming uses `-r <id>`.
//! The prompt goes in on stdin, never as an argument: quotes, backticks, `$`, very
//! long text, and shell or WSL interop layers then have nothing to mangle.
//!
//! The CLI surface varies by version and by what an enterprise build allows, so the
//! flags are read from `copilot --help` once per process: JSON output when that build
//! has it, `--silent` text output otherwise (the reply still arrives, without per-tool
//! status). Same care for `--allow-all` vs `--allow-all-tools`, telemetry and effort.
//!
//! Limitation: the CLI has no flag to append a system prompt, so the Bot's system
//! prompt is prepended to the prompt text. It is rebuilt every run, so memory
//! changes still reach the Bot.

use super::{describe, new_uuid, output, str_of, Adapter, AgentEvent, ModelOption, RunInput};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::OnceLock;

pub struct Copilot;

/// What this copilot binary accepts, parsed from its own help once per process.
struct Caps {
    /// `--output-format json`. Without it the CLI prints plain text and we stream that.
    json: bool,
    silent: bool,
    allow_all: bool,
    allow_all_tools: bool,
    telemetry: bool,
    model: bool,
    effort: bool,
    session_id: bool,
    resume: bool,
}

fn caps() -> &'static Caps {
    static CAPS: OnceLock<Caps> = OnceLock::new();
    CAPS.get_or_init(|| {
        let help = output("copilot", &["--help"]);
        let flags = help_flags(&help);
        let has = |f: &str| flags.contains(f);
        Caps {
            // `[possible values: text, json]` sits in the --output-format section.
            json: section(&help, "--output-format").contains("json"),
            silent: has("--silent"),
            allow_all: has("--allow-all"),
            allow_all_tools: has("--allow-all-tools"),
            telemetry: has("--disable-client-telemetry"),
            model: has("--model"),
            effort: has("--reasoning-effort"),
            session_id: has("--session-id"),
            resume: has("--resume") || has("--continue"),
        }
    })
}

/// Every `--flag` named in a help page, short forms and `<args>` stripped.
fn help_flags(help: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    for line in help.lines() {
        let t = line.trim_start();
        if !t.starts_with('-') {
            continue;
        }
        for tok in t.split_whitespace() {
            let Some(f) = tok.strip_prefix("--") else { continue };
            let name = f.split(['=', ',']).next().unwrap_or(f).split('<').next().unwrap_or(f).trim();
            if !name.is_empty() {
                set.insert(format!("--{name}"));
            }
        }
    }
    set
}

/// The help text of one flag: its line plus the wrapped description lines that follow.
fn section(help: &str, flag: &str) -> String {
    let Some(i) = help.find(flag) else { return String::new() };
    let mut out = String::new();
    for (n, line) in help[i + flag.len()..].lines().enumerate() {
        if n > 0 && line.trim_start().starts_with("--") {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

impl Adapter for Copilot {
    fn bin(&self) -> &'static str {
        "copilot"
    }

    fn args(&mut self, input: &RunInput, out: &mut Vec<AgentEvent>) -> Vec<String> {
        let c = caps();
        let mut a: Vec<String> = Vec::new();
        if c.json {
            a.extend(["--output-format".into(), "json".into()]);
        } else if c.silent {
            // Text output, without the stats footer, so the reply stays clean.
            a.push("--silent".into());
        }
        // Non-interactive runs must pre-approve tools; same reach as the other adapters.
        if c.allow_all {
            a.push("--allow-all".into());
        } else if c.allow_all_tools {
            a.push("--allow-all-tools".into());
        }
        // The server is self-hosted: no telemetry leaves it on our behalf.
        if c.telemetry {
            a.push("--disable-client-telemetry".into());
        }
        match &input.session_id {
            Some(id) => {
                if c.resume {
                    a.extend(["-r".into(), id.clone()]);
                }
            }
            None => {
                let id = new_uuid();
                out.push(AgentEvent::Session(id.clone()));
                if c.session_id {
                    a.extend(["--session-id".into(), id]);
                }
            }
        }
        if let Some(m) = &input.model
            && c.model
        {
            a.extend(["--model".into(), m.clone()]);
        }
        // `auto` routes internally and refuses any effort ("does not support
        // reasoning effort configuration"); only explicit models take one.
        if let Some(e) = &input.effort
            && c.effort
            && input.model.as_deref() != Some("auto")
        {
            a.extend(["--reasoning-effort".into(), e.clone()]);
        }
        a
    }

    fn stdin(&mut self, input: &RunInput) -> Option<String> {
        Some(format!("{}\n\n---\n\n{}", input.system.trim(), input.prompt))
    }

    fn plain(&self) -> bool {
        !caps().json
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
        let mut list = vec![ModelOption {
            id: "auto".into(),
            label: "Auto (Copilot picks)".into(),
            efforts: vec![],
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
                list.push(ModelOption { label: m.clone(), id: m, efforts: efforts.clone(), aliases: vec![] });
            }
        }
        list
    }
}
