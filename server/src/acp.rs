//! The server's only way to run an agent: the Agent Client Protocol (ACP),
//! JSON-RPC 2.0 over stdio. An agent is a launch command (`copilot --acp`,
//! `opencode acp`, or a bridge like `npx pi-acp`), not Rust code.
//!
//! Each turn is one process: spawn, `initialize`, `session/new` (or `session/load`
//! to resume), `session/prompt`, read notifications until the prompt response,
//! then let `kill_on_drop` end it. Sessions persist agent-side, like the old
//! `-r <id>` resumes. The server is self-hosted and Bots are pre-authorized,
//! so every `session/request_permission` is approved without asking the user —
//! the ACP equivalent of the old `--allow-all`.
//!
//! The prompt goes in as one text block with the Bot's system text prepended:
//! ACP has no system-prompt field, so this is the only way identity and memory
//! reach the agent. ACP also standardizes modes, not models: pickers stay
//! env-driven (`COPILOT_MODELS`, BYOK vars) and a chosen model is applied via
//! the agent's own env var (`COPILOT_MODEL`).

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Command, Child, ChildStdin};
use tokio::sync::watch;

/// Why a run was cut short.
pub type AbortReason = Option<&'static str>;

pub enum AgentEvent {
    /// Agent session to resume next time.
    Session(String),
    /// Streaming piece of the current text block.
    TextDelta(String),
    /// A finished text block: one chat bubble.
    Text(String),
    /// The agent started using a tool.
    Tool { name: String, detail: String },
    Error(String),
}

pub struct RunInput {
    pub prompt: String,
    /// Bot identity, rules, and memory. Rebuilt every run.
    pub system: String,
    /// None = start a new agent session.
    pub session_id: Option<String>,
    pub cwd: PathBuf,
    pub model: Option<String>,
    /// Reasoning effort, in the agent's own words. ACP has no field for it:
    /// kept so the Bot's choice survives, unused until agents expose it.
    #[allow(dead_code)]
    pub effort: Option<String>,
    /// Context window tier, in the agent's own words. Same as effort.
    #[allow(dead_code)]
    pub context: Option<String>,
    pub env: HashMap<String, String>,
}

#[derive(Clone)]
pub struct Agent {
    pub id: String,
    pub name: String,
    /// Launch command, e.g. `copilot --acp`.
    pub bin: String,
    pub args: Vec<String>,
    /// Env var this agent reads for its model, when the Bot picked one.
    pub model_env: Option<String>,
}

/// Every agent this server can run: the built-ins, then `ACP_AGENTS`
/// entries (`id:Name:command`, comma-separated), for bridges like
/// `ACP_AGENTS=pi:Pi:npx pi-acp@0.0.34`.
pub fn agents() -> Vec<Agent> {
    agents_from(std::env::var("ACP_AGENTS").ok().as_deref())
}

/// The registry, from an explicit `ACP_AGENTS` value (so it can be tested
/// without mutating the process environment).
pub fn agents_from(acp_agents: Option<&str>) -> Vec<Agent> {
    let built = |id: &str, name: &str, args: &[&str], model_env: &str| Agent {
        id: id.into(),
        name: name.into(),
        bin: id.into(),
        args: args.iter().map(|a| a.to_string()).collect(),
        model_env: Some(model_env.into()),
    };
    let mut list = vec![
        built("copilot", "Copilot", &["--acp"], "COPILOT_MODEL"),
        built("opencode", "OpenCode", &["acp"], "OPENCODE_MODEL"),
    ];
    for entry in acp_agents.unwrap_or_default().split(',') {
        let mut it = entry.splitn(3, ':');
        let (Some(id), Some(name), Some(cmd)) = (it.next(), it.next(), it.next()) else { continue };
        let (id, name, cmd) = (id.trim(), name.trim(), cmd.trim());
        if id.is_empty() || name.is_empty() || cmd.is_empty() {
            continue; // malformed entry: skip, don't break the working built-ins
        }
        let mut parts = cmd.split_whitespace().map(String::from);
        if let Some(bin) = parts.next() {
            list.push(Agent { id: id.into(), name: name.into(), bin, args: parts.collect(), model_env: None });
        }
    }
    list
}

pub fn get(id: &str) -> Option<Agent> {
    agents().into_iter().find(|a| a.id == id)
}

/// Agents whose launch command resolves, for the pickers.
pub fn installed() -> Vec<Agent> {
    agents().into_iter().filter(|a| find(&a.bin).is_some()).collect()
}

pub fn ids() -> Vec<String> {
    agents().into_iter().map(|a| a.id).collect()
}

/// Resolves a binary like a login shell would: `copilot` and `opencode` live in
/// user bin folders a systemd unit's PATH doesn't contain.
pub fn find(bin: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> =
        std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(user_bin_dirs().into_iter().map(PathBuf::from));
    dirs.into_iter().map(|d| d.join(bin)).find(|p| p.is_file())
}

/// Folders added to the Bots' PATH, so the agents and their tools resolve.
pub fn user_bin_dirs() -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut dirs: Vec<String> = [".local/bin", ".npm-global/bin", ".bun/bin", ".cargo/bin", ".volta/bin", ".deno/bin", ".opencode/bin"]
        .iter()
        .map(|d| format!("{home}/{d}"))
        .filter(|d| std::path::Path::new(d).is_dir())
        .collect();
    dirs.push("/usr/local/bin".into());
    dirs
}

/// Runs a command and returns its stdout, or "" if it fails.
pub fn output(bin: &str, args: &[&str]) -> String {
    let bin = find(bin).unwrap_or_else(|| PathBuf::from(bin));
    std::process::Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

#[derive(Clone, serde::Serialize)]
pub struct ModelOption {
    pub id: String,
    pub label: String,
    /// Efforts this model accepts. Empty = no effort setting.
    pub efforts: Vec<String>,
    /// Context window tiers this model accepts. Empty = no context setting.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub contexts: Vec<String>,
    /// Other names the agent accepts for this model (e.g. "opus").
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

/// Models an agent can run here, cached for 10 minutes. ACP standardizes
/// modes, not models: Auto plus env-declared lists. Efforts and context tiers
/// stay empty — over ACP there is no field to apply them.
pub fn models(name: &str) -> Vec<ModelOption> {
    static CACHE: Mutex<Option<HashMap<String, (Instant, Vec<ModelOption>)>>> = Mutex::new(None);
    if let Some((at, list)) = CACHE.lock().unwrap().get_or_insert_default().get(name) {
        if at.elapsed() < Duration::from_secs(600) && !list.is_empty() {
            return list.clone();
        }
    }
    let prefix = name.to_uppercase().replace('-', "_");
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let mut list = vec![ModelOption {
        id: "auto".into(),
        label: format!("Auto ({} picks)", get(name).map(|a| a.name.clone()).unwrap_or_default()),
        efforts: vec![],
        contexts: vec![],
        aliases: vec![],
    }];
    let configured = env(&format!("{prefix}_MODELS"))
        .map(|v| v.split(',').map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).collect::<Vec<_>>())
        .unwrap_or_default();
    let mut configured: Vec<String> = configured.into_iter().chain(env(&format!("{prefix}_MODEL"))).collect();
    // Copilot's CLI saves its model in its own settings; expose that too.
    if name == "copilot"
        && let Some(m) = output("copilot", &["config", "model"]).trim().to_string().into()
        && !m.is_empty()
    {
        configured.push(m);
    }
    for m in configured.into_iter().filter(|m| m != "auto") {
        if !list.iter().any(|o| o.id == m) {
            list.push(ModelOption { label: m.clone(), id: m, efforts: vec![], contexts: vec![], aliases: vec![] });
        }
    }
    CACHE.lock().unwrap().get_or_insert_default().insert(name.into(), (Instant::now(), list.clone()));
    list
}

/// What we sent and are waiting on. `Done` means the turn is over.
#[derive(PartialEq, Clone, Copy)]
enum Step {
    Initialize,
    New,
    Load,
    Prompt,
    Done,
}

/// Runs one Bot turn: one ACP process, `initialize`, `session/new` or
/// `session/load`, `session/prompt`, then events until the turn ends or abort.
pub async fn run(agent: Agent, input: RunInput, mut abort: watch::Receiver<AbortReason>, mut emit: impl FnMut(AgentEvent)) {
    let mut env = input.env.clone();
    if let (Some(m), Some(k)) = (&input.model, &agent.model_env) {
        if m != "auto" {
            env.insert(k.clone(), m.clone());
        }
    }

    let bin = find(&agent.bin).unwrap_or_else(|| PathBuf::from(&agent.bin));
    let mut child = match spawn(&bin, &agent, &input, &env) {
        Ok(c) => c,
        Err(e) => return emit(AgentEvent::Error(format!("Couldn't start {}: {e}", agent.name))),
    };
    let pid = child.id();
    let mut stdin = child.stdin.take().unwrap();

    // The prompt goes in as one text block. ACP has no system-prompt field, so
    // the Bot's identity, rules, and memory are prepended, as before.
    let prompt = format!("{}\n\n---\n\n{}", input.system.trim(), input.prompt);
    let mut id = 0u64;
    let mut step = Step::Initialize;
    write_msg(&mut stdin, &request(id, "initialize", json!({ "protocolVersion": 1, "clientCapabilities": {} }))).await;

    // JSON lines. Split only on "\n": an agent may put U+2028 inside JSON strings.
    let mut lines = BufReader::new(child.stdout.take().unwrap()).split(b'\n');
    let mut stderr = child.stderr.take().unwrap();
    let mut message = String::new();
    let mut message_id = String::new();
    let mut saw_error = false;
    let mut aborted = false;
    // Did the agent answer the prompt? If so, how it exits afterwards is our
    // doing (we terminate it) and says nothing about the turn.
    let mut finished = false;
    enum Sel {
        Line(std::io::Result<Option<Vec<u8>>>),
        Abort,
    }
    loop {
        // Nothing is awaited inside the branches: the select result (which holds
        // the watch guard) must not live across the processing below.
        let sel = tokio::select! {
            line = lines.next_segment() => Sel::Line(line),
            _ = abort.wait_for(|r| r.is_some()), if !aborted => Sel::Abort,
        };
        let bytes = match sel {
            Sel::Line(Ok(Some(bytes))) => bytes,
            Sel::Line(_) => break, // stream ended
            Sel::Abort => {
                aborted = true;
                // Best effort: tell the agent, then kill the tree like before.
                write_msg(&mut stdin, &json!({ "jsonrpc": "2.0", "method": "session/cancel", "params": {} })).await;
                if let Some(pid) = pid {
                    kill_group(pid, "TERM");
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        kill_group(pid, "KILL");
                    });
                }
                continue;
            }
        };
        let Ok(v) = serde_json::from_slice::<Value>(&bytes) else { continue };
        let mut out = Vec::new();
        if inbound(&v, &mut step, &mut id, &input, &prompt, &mut message, &mut message_id, &mut out, &mut stdin).await {
            for e in out.drain(..) {
                saw_error |= matches!(e, AgentEvent::Error(_));
                emit(e);
            }
            if !message.trim().is_empty() {
                emit(AgentEvent::Text(message.trim().into()));
            }
            finished = true;
            break;
        }
        for e in out.drain(..) {
            saw_error |= matches!(e, AgentEvent::Error(_));
            emit(e);
        }
    }
    // ACP agents are servers: after the turn they keep waiting for more input.
    // End the process ourselves, or the Bot would look busy forever.
    let mut status = None;
    if let Some(pid) = pid {
        kill_group(pid, "TERM");
        status = tokio::time::timeout(Duration::from_secs(3), child.wait()).await.ok().and_then(Result::ok);
        if status.is_none() {
            kill_group(pid, "KILL");
            status = child.wait().await.ok();
        }
    }
    let mut err = String::new();
    // Bounded: a stuck pipe must not keep the Bot's queue blocked.
    let _ = tokio::time::timeout(Duration::from_secs(2), stderr.read_to_string(&mut err)).await;
    // Only a turn that never finished is a failure: an agent that answered and
    // then exited (OpenCode exits 130 once we terminate it) is fine.
    if !finished && !saw_error && !aborted {
        let tail: Vec<&str> = err.trim().lines().rev().take(3).collect();
        let msg = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
        emit(AgentEvent::Error(if msg.is_empty() {
            format!("{} exited with {}", agent.name, status.map(|s| s.to_string()).unwrap_or_default())
        } else {
            msg
        }));
    }
}

fn spawn(bin: &PathBuf, agent: &Agent, input: &RunInput, env: &HashMap<String, String>) -> std::io::Result<Child> {
    Command::new(bin)
        .args(&agent.args)
        .current_dir(&input.cwd)
        .envs(env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0) // own group, so stop kills the whole tree
        .kill_on_drop(true)
        .spawn()
}

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

fn respond(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// ACP is line-delimited JSON: every message needs its newline, or the agent
/// waits forever for the rest of the line.
async fn write_msg(stdin: &mut ChildStdin, v: &Value) {
    let mut line = v.to_string();
    line.push('\n');
    let _ = stdin.write_all(line.as_bytes()).await;
}

/// One JSON-RPC message from the agent: a notification (events), a request we
/// must answer (permissions), or a response to ours. Appends the turn's events
/// to `out`. Returns true when the turn is over: the prompt response arrived,
/// or one of our requests failed.
#[allow(clippy::too_many_arguments)]
async fn inbound(
    v: &Value,
    step: &mut Step,
    id: &mut u64,
    input: &RunInput,
    prompt: &str,
    message: &mut String,
    message_id: &mut String,
    out: &mut Vec<AgentEvent>,
    stdin: &mut ChildStdin,
) -> bool {
    // A request from the agent: answer it. We are the self-hosted server, so
    // permissions are approved; anything else falls back to the agent's own tools.
    if v.get("method").is_some() && v.get("id").is_some() {
        let rid = v["id"].clone();
        let reply = match str_of(v, "method") {
            "session/request_permission" => {
                // Prefer an "allow once" style option, else the first one.
                let options = v["params"]["options"].as_array().cloned().unwrap_or_default();
                match options.iter().find(|o| str_of(o, "kind").starts_with("allow")).or_else(|| options.first()) {
                    Some(o) => respond(&rid, json!({ "outcome": { "outcome": "selected", "optionId": o["optionId"] } })),
                    None => respond(&rid, json!({ "outcome": { "outcome": "cancelled" } })),
                }
            }
            _ => json!({ "jsonrpc": "2.0", "id": rid, "error": { "code": -32601, "message": "Not supported" } }),
        };
        write_msg(stdin, &reply).await;
        return false;
    }

    // A notification: stream it to the web app. Only during the prompt: a
    // `session/load` replays the old conversation as updates, and that history
    // is already in the chat.
    if v.get("id").is_none() {
        if *step == Step::Prompt
            && let Some(update) = v["params"]["update"].as_object()
        {
            out.extend(map_update(update, message, message_id));
        }
        return false;
    }

    // A response to our request: the next step of the turn.
    if !matches!(*step, Step::Initialize | Step::New | Step::Load | Step::Prompt) {
        return false;
    }
    *id += 1;

    if let Some(e) = v.get("error") {
        // `session/load` on a wiped session: the runner recognizes this and
        // retries once with a fresh session, like the old adapters.
        let msg = e["message"].as_str().unwrap_or("agent failed").to_string();
        out.push(AgentEvent::Error(if matches!(*step, Step::Load) { format!("Session not found: {msg}") } else { msg }));
        *step = Step::Done;
        return true; // no reply ever came: end the turn
    }
    let result = &v["result"];
    match *step {
        Step::Initialize => {
            let (method, params, next) = match &input.session_id {
                Some(s) => ("session/load", json!({ "sessionId": s, "cwd": input.cwd, "mcpServers": [] }), Step::Load),
                None => ("session/new", json!({ "cwd": input.cwd, "mcpServers": [] }), Step::New),
            };
            write_msg(stdin, &request(*id, method, params)).await;
            *step = next;
        }
        Step::New => {
            let sid = str_of(result, "sessionId").to_string();
            if !sid.is_empty() {
                out.push(AgentEvent::Session(sid.clone())); // the runner saves it to resume
            }
            write_msg(
                stdin,
                &request(*id, "session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": prompt }] })),
            )
            .await;
            *step = Step::Prompt;
        }
        Step::Load => {
            // `session/load` often answers without a sessionId: the id we asked
            // to load is the one to prompt.
            let sid = input.session_id.clone().unwrap_or_default();
            write_msg(
                stdin,
                &request(*id, "session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": prompt }] })),
            )
            .await;
            *step = Step::Prompt;
        }
        Step::Prompt => {
            *step = Step::Done;
            return true;
        }
        Step::Done => {}
    }
    false
}

/// One `session/update` notification: text chunks and tool calls.
fn map_update(update: &serde_json::Map<String, Value>, message: &mut String, message_id: &mut String) -> Vec<AgentEvent> {
    let mut out = Vec::new();
    let kind = update.get("sessionUpdate").and_then(Value::as_str).unwrap_or_default();
    match kind {
        "agent_message_chunk" => {
            let chunk = &update["content"];
            let Some(text) = chunk.get("text").and_then(Value::as_str) else { return out };
            if str_of(chunk, "type") != "text" || text.is_empty() {
                return out;
            }
            // A new messageId starts a new bubble.
            let mid = update.get("messageId").and_then(Value::as_str).unwrap_or_default().to_string();
            if mid != *message_id {
                if !message.trim().is_empty() {
                    out.push(AgentEvent::Text(message.trim().into()));
                    message.clear();
                }
                *message_id = mid;
            }
            message.push_str(text);
            out.push(AgentEvent::TextDelta(text.into()));
        }
        "tool_call" => {
            // Fields are flat in the update, not nested under "toolCall".
            let title = update.get("title").and_then(Value::as_str).unwrap_or_default();
            let kind = update.get("kind").and_then(Value::as_str).unwrap_or_default();
            let status = update.get("status").and_then(Value::as_str).unwrap_or_default();
            let name = if title.is_empty() { kind } else { title };
            if name.is_empty() || status == "completed" {
                return out; // updates repeat tool_call with the same content
            }
            let detail = update.get("rawInput").map(describe).unwrap_or_default();
            out.push(AgentEvent::Tool { name: name.into(), detail })
        }
        _ => {} // thoughts, plans, usage, commands, config: not shown in chat
    }
    out
}

/// One short line describing a tool call, for the "working" status.
pub fn describe(input: &Value) -> String {
    let v = ["command", "cmd", "file_path", "path", "url", "query", "pattern", "description"]
        .iter()
        .find_map(|k| input.get(*k));
    let s = match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" "),
        _ => String::new(),
    };
    short(&s)
}

pub fn short(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(120).collect()
}

pub fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn kill_group(pid: u32, signal: &str) {
    let _ = std::process::Command::new("kill")
        .args([format!("-{signal}"), "--".into(), format!("-{pid}")])
        .stderr(Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// The fake agent is a small Python script, one mode per test. It speaks
    /// the same `session/update` shapes observed on `copilot --acp` and
    /// `opencode acp`.
    fn agent(mode: &str) -> Agent {
        Agent {
            id: "fake".into(),
            name: "Fake".into(),
            bin: "python3".into(),
            args: vec!["-c".into(), include_str!("fake_acp.py").into(), mode.into()],
            model_env: Some("FAKE_MODEL".into()),
        }
    }

    async fn run_fake(mode: &str, input: RunInput) -> Vec<String> {
        let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let (_tx, rx) = watch::channel(None);
        let agent = agent(mode);
        // No agent may hang the test suite.
        let r = tokio::time::timeout(Duration::from_secs(20), run(agent, input, rx, move |e| sink.lock().unwrap().push(show(&e)))).await;
        r.expect("fake agent must finish");
        let out = events.lock().unwrap().clone();
        assert!(out.iter().any(|e| !e.starts_with("error")), "unexpected error: {out:?}");
        out
    }

    fn show(e: &AgentEvent) -> String {
        match e {
            AgentEvent::Session(s) => format!("session:{s}"),
            AgentEvent::TextDelta(t) => format!("delta:{t}"),
            AgentEvent::Text(t) => format!("text:{t}"),
            AgentEvent::Tool { name, detail } => format!("tool:{name}:{detail}"),
            AgentEvent::Error(m) => format!("error:{m}"),
        }
    }

    fn input(session_id: Option<String>) -> RunInput {
        RunInput {
            prompt: "hello".into(),
            system: "You are Test.".into(),
            session_id,
            cwd: std::env::temp_dir(),
            model: Some("test-model".into()),
            effort: None,
            context: None,
            env: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn streams_a_reply_and_creates_a_session() {
        let out = run_fake("ok", input(None)).await;
        assert!(out.iter().any(|e| e == "session:s1"), "session id must be reported: {out:?}");
        assert!(out.iter().any(|e| e == "delta:prompt_for:s1"), "chunks must stream: {out:?}");
        assert!(out.iter().any(|e| e == "text:prompt_for:s1"), "the message must finish: {out:?}");
    }

    #[tokio::test]
    async fn resumes_with_session_load() {
        let out = run_fake("ok", input(Some("s1".into()))).await;
        // The prompt must go to the session we asked to load, even though
        // `session/load` answers without a sessionId.
        assert!(out.iter().any(|e| e == "delta:prompt_for:s1"), "must prompt the loaded session: {out:?}");
        // Loading replays history; that must not be re-posted to the chat.
        assert!(!out.iter().any(|e| e.contains("REPLAYED")), "replayed history leaked: {out:?}");
    }

    #[tokio::test]
    async fn load_failure_is_a_lost_session() {
        let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let (_tx, rx) = watch::channel(None);
        run(agent("load_fails"), input(Some("gone".into())), rx, move |e| sink.lock().unwrap().push(show(&e))).await;
        let out = events.lock().unwrap().clone();
        let err = out.iter().find(|e| e.starts_with("error")).expect("an error event");
        let msg = err.strip_prefix("error:").unwrap().to_lowercase();
        assert!(msg.contains("session") && msg.contains("not found"), "the runner must recognize it: {msg}");
    }

    #[tokio::test]
    async fn permission_requests_are_approved() {
        // The fake only finishes its reply after we approve.
        let out = run_fake("permission", input(Some("s1".into()))).await;
        assert!(out.iter().any(|e| e == "text:approved"), "must keep going after approving: {out:?}");
    }

    #[tokio::test]
    async fn tool_calls_map_to_tool_events() {
        let out = run_fake("tool", input(Some("s1".into()))).await;
        assert!(out.iter().any(|e| e.starts_with("tool:List files:ls /tmp")), "{out:?}");
    }

    #[tokio::test]
    async fn a_lingering_agent_does_not_hold_the_turn_open() {
        // Real agents stay alive after the turn; the run must still return.
        let started = std::time::Instant::now();
        let out = run_fake("lingers", input(None)).await;
        assert!(out.iter().any(|e| e == "text:prompt_for:s1"), "{out:?}");
        assert!(started.elapsed() < Duration::from_secs(15), "took {:?}", started.elapsed());
    }

    #[tokio::test]
    async fn a_nonzero_exit_after_the_turn_is_not_an_error() {
        // OpenCode exits 130 when we terminate it; the reply already arrived.
        let out = run_fake("exit_130", input(None)).await;
        assert!(out.iter().any(|e| e == "text:prompt_for:s1"), "{out:?}");
    }

    #[tokio::test]
    async fn handshake_failure_surfaces_an_error() {
        let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let (_tx, rx) = watch::channel(None);
        run(agent("bad_handshake"), input(Some("s1".into())), rx, move |e| sink.lock().unwrap().push(show(&e))).await;
        let out = events.lock().unwrap().clone();
        assert!(out.iter().any(|e| e.starts_with("error")), "{out:?}");
    }

    #[tokio::test]
    async fn missing_binary_surfaces_an_error() {
        let mut a = agent("ok");
        a.bin = "definitely-not-a-real-binary-xyz".into();
        let mut events = Vec::new();
        let (_tx, rx) = watch::channel(None);
        run(a, input(None), rx, |e| events.push(show(&e))).await;
        assert!(events.iter().any(|e| e.starts_with("error:Couldn't start")), "{events:?}");
    }

    #[test]
    fn parse_acp_agents_entries() {
        let list = agents_from(Some("pi:Pi:npx pi-acp@0.0.34, bad entry,zed:Zed:/zed --acp"));
        let pi = list.iter().find(|a| a.id == "pi").expect("pi parsed");
        assert_eq!((pi.name.as_str(), pi.bin.as_str(), pi.args.len()), ("Pi", "npx", 1));
        assert_eq!(pi.args[0], "pi-acp@0.0.34");
        assert!(list.iter().any(|a| a.id == "copilot"), "built-ins stay");
        assert!(list.iter().all(|a| a.id != "bad"), "malformed entries are skipped");
        let zed = list.iter().find(|a| a.id == "zed").expect("zed parsed");
        assert_eq!((zed.bin.as_str(), zed.args[0].as_str()), ("/zed", "--acp"));
    }
}
