//! Parsing Claude Code's stream-json output into run events.

use super::*;
use std::collections::HashMap;

#[derive(Clone)]
pub struct Outcome {
    pub ok: bool,
    pub text: String,
    pub subtype: String,
    pub turns: u32,
    pub duration_ms: u64,
    pub denials: Vec<Denied>,
    /// The run stopped at the subscription's usage limit.
    pub limit: bool,
}

/// A denial from the result, with the call it stopped.
#[derive(Debug, Clone)]
pub struct Denied {
    pub tool_use_id: String,
    pub denial: Denial,
}

/// Why Claude Code denied a call (`system/permission_denied`), e.g. kind `safetyCheck` or `classifier`.
#[derive(Debug, Clone)]
pub struct DeniedReason {
    pub tool_use_id: String,
    pub kind: String,
    pub reason: String,
}

#[derive(Default)]
pub struct Parsed {
    pub events: Vec<RunEvent>,
    /// A `stream_event` partial: forwarded live, never stored.
    pub partial: bool,
    pub outcome: Option<Outcome>,
    /// Subscription usage reported by the run (`rate_limit_event`).
    pub usage: Option<UsageSample>,
    /// The Claude Code version, from the init line.
    pub claude_version: Option<String>,
    /// The usage limit was hit; it resets at this time.
    pub limited: Option<Ts>,
    pub denied: Option<DeniedReason>,
    /// The context size in tokens after this top-level assistant message.
    pub context: Option<u64>,
}

/// One observation of the subscription usage windows. Both `None` means the account has none.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UsageSample {
    pub five_hour: Option<UsageWindow>,
    pub seven_day: Option<UsageWindow>,
}

fn percent_of(utilization: f64) -> f64 {
    ((utilization * 1000.0).round() / 10.0).clamp(0.0, 100.0)
}

/// Wraps a synthetic Office event as a transcript line, so replay needs only the transcript.
pub fn omega_line(ev: &RunEvent) -> String {
    json!({ "type": "omega", "event": ev }).to_string()
}

const BRIEF_KEYS: &[&str] = &[
    "command", "file_path", "notebook_path", "path", "pattern", "url", "query", "skill", "description", "prompt", "task", "person",
];
const PATH_KEYS: &[&str] = &["file_path", "notebook_path", "path"];

/// A file path as the UI shows it: relative to the employee's home when inside it, forward slashes,
/// no `\\?\` verbatim prefix.
fn display_path(path: &str, home: &Path) -> String {
    let norm = |s: &str| s.strip_prefix(r"\\?\").unwrap_or(s).replace('\\', "/");
    let path = norm(path);
    let home = norm(&home.to_string_lossy());
    let home = home.trim_end_matches('/');
    if !home.is_empty() && path.len() > home.len() + 1 && path.is_char_boundary(home.len()) {
        let (head, rest) = path.split_at(home.len());
        let same = if cfg!(windows) { head.eq_ignore_ascii_case(home) } else { head == home };
        if same && rest.starts_with('/') {
            return rest[1..].to_string();
        }
    }
    path
}

fn tool_brief(input: &Value, home: &Path) -> String {
    let s = match BRIEF_KEYS.iter().find_map(|k| input.get(k).and_then(|v| v.as_str()).map(|v| (*k, v))) {
        Some((k, v)) if PATH_KEYS.contains(&k) => display_path(v, home),
        Some((_, v)) => v.to_string(),
        None if input.as_object().map(|o| o.is_empty()).unwrap_or(true) => String::new(),
        None => input.to_string(),
    };
    clip(&one_line(&s), 160)
}

pub fn suggested_rule(tool: &str, input: &Value) -> String {
    match (tool, input.get("command").and_then(|c| c.as_str()).and_then(|c| c.split_whitespace().next())) {
        ("Bash", Some(word)) => format!("Bash({word}:*)"),
        _ => tool.to_string(),
    }
}

/// The rule that allows every tool of an MCP server: `mcp__claude-in-chrome__*` for
/// `mcp__claude-in-chrome__javascript_tool`. `None` for a tool that is not an MCP tool.
pub fn mcp_server_rule(tool: &str) -> Option<String> {
    let rest = tool.strip_prefix("mcp__")?;
    let (server, name) = rest.split_once("__")?;
    (!server.is_empty() && !name.is_empty()).then(|| format!("mcp__{server}__*"))
}

/// Every tool of the Claude in Chrome extension, which `--chrome` connects.
pub const CHROME_RULE: &str = "mcp__claude-in-chrome__*";

/// The text of a tool result, whether its content is a string or a list of text blocks.
pub fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().filter_map(|b| b.get("text").and_then(|t| t.as_str())).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// Parses one stream-json line. `home` is the employee directory, used to shorten file paths.
pub fn parse_line(line: &str, home: &Path) -> Parsed {
    let mut p = Parsed::default();
    let Ok(v) = serde_json::from_str::<Value>(line) else { return p };
    let str_at = |v: &Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    // Subagent traffic (parent_tool_use_id set) stays in the raw transcript only.
    let nested = v.get("parent_tool_use_id").map(|x| !x.is_null()).unwrap_or(false);
    match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
        "stream_event" => {
            p.partial = true;
            let ev = &v["event"];
            if !nested && ev["type"] == "content_block_delta" && ev["delta"]["type"] == "text_delta" {
                p.events.push(RunEvent::TextDelta { text: str_at(&ev["delta"], "text") });
            }
        }
        "system" if v["subtype"] == "init" => {
            let skills = v["skills"].as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default();
            p.events.push(RunEvent::Init { model: str_at(&v, "model"), skills });
            p.claude_version = v["claude_code_version"].as_str().map(String::from);
        }
        "system" if v["subtype"] == "permission_denied" => {
            p.denied = Some(DeniedReason {
                tool_use_id: str_at(&v, "tool_use_id"),
                kind: str_at(&v, "decision_reason_type"),
                reason: str_at(&v, "decision_reason"),
            });
        }
        // Claude Code reports every background job as a task: sub-agents and shell commands alike, and
        // the ones a sub-agent starts too. Only the sub-agents the employee started are shown.
        "system" if v["subtype"] == "task_started" => {
            if v["task_type"] == "local_agent" && !v["owned_by_subagent"].as_bool().unwrap_or(false) {
                p.events.push(RunEvent::TaskStarted {
                    task_id: str_at(&v, "task_id"),
                    tool_use_id: str_at(&v, "tool_use_id"),
                    name: clip(&one_line(&str_at(&v, "description")), 120),
                });
            }
        }
        "system" if v["subtype"] == "task_progress" => {
            let u = &v["usage"];
            p.events.push(RunEvent::TaskProgress {
                task_id: str_at(&v, "task_id"),
                activity: clip(&one_line(&str_at(&v, "description")), 160),
                last_tool: v["last_tool_name"].as_str().filter(|t| !t.is_empty()).map(String::from),
                tool_uses: u["tool_uses"].as_u64().unwrap_or(0) as u32,
                tokens: u["total_tokens"].as_u64().unwrap_or(0),
                duration_ms: u["duration_ms"].as_u64().unwrap_or(0),
            });
        }
        "system" if v["subtype"] == "task_notification" => {
            let u = &v["usage"];
            p.events.push(RunEvent::TaskEnded {
                task_id: str_at(&v, "task_id"),
                outcome: match v["status"].as_str() {
                    Some("completed") => TaskOutcome::Done,
                    Some("failed" | "error") => TaskOutcome::Failed,
                    _ => TaskOutcome::Stopped,
                },
                tool_uses: u["tool_uses"].as_u64().unwrap_or(0) as u32,
                tokens: u["total_tokens"].as_u64().unwrap_or(0),
                duration_ms: u["duration_ms"].as_u64().unwrap_or(0),
            });
        }
        "assistant" if !nested => {
            // After a usage limit Claude Code writes a `<synthetic>` message with zero usage: it is not
            // the context's size.
            let m = &v["message"];
            let tokens = |k: &str| m["usage"][k].as_u64().unwrap_or(0);
            let size = tokens("input_tokens") + tokens("cache_read_input_tokens") + tokens("cache_creation_input_tokens");
            if m["model"] != "<synthetic>" && size > 0 {
                p.context = Some(size);
            }
            for b in v["message"]["content"].as_array().into_iter().flatten() {
                match b["type"].as_str() {
                    Some("text") if !str_at(b, "text").trim().is_empty() => p.events.push(RunEvent::Text { text: str_at(b, "text") }),
                    Some("tool_use") => p.events.push(RunEvent::ToolCall {
                        id: str_at(b, "id"),
                        tool: str_at(b, "name"),
                        brief: tool_brief(&b["input"], home),
                    }),
                    _ => {}
                }
            }
        }
        "user" if !nested => {
            for b in v["message"]["content"].as_array().into_iter().flatten().filter(|b| b["type"] == "tool_result") {
                p.events.push(RunEvent::ToolResult {
                    id: str_at(b, "tool_use_id"),
                    ok: !b["is_error"].as_bool().unwrap_or(false),
                    brief: clip(&one_line(&result_text(&b["content"])), 200),
                });
            }
        }
        "result" => {
            let o = Outcome {
                ok: !v["is_error"].as_bool().unwrap_or(false) && v["subtype"] == "success",
                text: str_at(&v, "result"),
                subtype: str_at(&v, "subtype"),
                turns: v["num_turns"].as_u64().unwrap_or(0) as u32,
                duration_ms: v["duration_ms"].as_u64().unwrap_or(0),
                denials: v["permission_denials"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|d| {
                        let tool = str_at(d, "tool_name");
                        let denial = Denial { brief: tool_brief(&d["tool_input"], home), suggested_rule: suggested_rule(&tool, &d["tool_input"]), tool };
                        Denied { tool_use_id: str_at(d, "tool_use_id"), denial }
                    })
                    .collect(),
                limit: v["api_error"] == "usage_limit_reached",
            };
            p.events.push(RunEvent::Result {
                ok: o.ok,
                text: o.text.clone(),
                turns: o.turns,
                duration_ms: o.duration_ms,
            });
            p.outcome = Some(o);
        }
        "rate_limit_event" => {
            let info = &v["rate_limit_info"];
            if info["status"] == "rejected" {
                p.limited = info["resetsAt"].as_i64().or(v["resetsAt"].as_i64()).and_then(|s| chrono::DateTime::from_timestamp(s, 0));
            }
            let windows = &info["unifiedWindows"];
            let window = |k: &str| {
                let w = &windows[k];
                Some(UsageWindow {
                    percent: percent_of(w["utilization"].as_f64()?),
                    resets_at: w["resetsAt"].as_i64().and_then(|s| chrono::DateTime::from_timestamp(s, 0)),
                    resets_text: None,
                })
            };
            let sample = UsageSample { five_hour: window("five_hour"), seven_day: window("seven_day") };
            if sample.five_hour.is_some() || sample.seven_day.is_some() {
                p.usage = Some(sample);
            }
        }
        "omega" => {
            if let Ok(ev) = serde_json::from_value::<RunEvent>(v["event"].clone()) {
                p.events.push(ev);
            }
        }
        _ => {}
    }
    p
}

/// Sorts the task events `parse_line` cannot judge from one line: a task's progress and end name only
/// its id, so whether it is a sub-agent is known from having seen it start.
#[derive(Default)]
pub struct TaskWatch {
    agents: std::collections::HashSet<String>,
}

impl TaskWatch {
    pub fn keep(&mut self, ev: &RunEvent) -> bool {
        match ev {
            RunEvent::TaskStarted { task_id, .. } => {
                self.agents.insert(task_id.clone());
                true
            }
            RunEvent::TaskProgress { task_id, .. } | RunEvent::TaskEnded { task_id, .. } => self.agents.contains(task_id),
            _ => true,
        }
    }
}

/// Keeps only the newest `TaskProgress` of each task: they arrive many times a second and each one
/// replaces the last.
pub fn coalesce_progress(events: &mut Vec<RunEvent>) {
    let mut newest = std::collections::HashMap::new();
    for (i, ev) in events.iter().enumerate() {
        if let RunEvent::TaskProgress { task_id, .. } = ev {
            newest.insert(task_id.clone(), i);
        }
    }
    let mut i = 0;
    events.retain(|ev| {
        let keep = match ev {
            RunEvent::TaskProgress { task_id, .. } => newest.get(task_id) == Some(&i),
            _ => true,
        };
        i += 1;
        keep
    });
}

/// Replays a stored transcript into the events the UI shows.
pub fn replay(transcript: &str, home: &Path) -> Vec<RunEvent> {
    let mut watch = TaskWatch::default();
    let mut events: Vec<RunEvent> = transcript
        .lines()
        .flat_map(|l| parse_line(l, home).events)
        .filter(|e| !matches!(e, RunEvent::TextDelta { .. }) && watch.keep(e))
        .collect();
    coalesce_progress(&mut events);
    events
}

/// The transcript's lines that parse as JSON: a crash or power loss can leave a half-written last line.
fn json_lines(transcript: &str) -> impl Iterator<Item = Value> + '_ {
    transcript.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok())
}

fn is_nested(v: &Value) -> bool {
    v.get("parent_tool_use_id").is_some_and(|x| !x.is_null())
}

/// One line of at most 200 characters.
fn one_liner(text: &str) -> bool {
    let t = text.trim();
    !t.contains('\n') && t.chars().count() <= 200
}

/// Joins two result segments of one run: a message steered into a working turn ends it with a result
/// of its own. A trailing one-liner is added to a real report instead of replacing it.
pub fn merge_outcome(prev: Outcome, next: Outcome) -> Outcome {
    let text = if next.text.trim().is_empty() {
        prev.text
    } else if one_liner(&next.text) && !one_liner(&prev.text) {
        format!("{}\n\n{}", prev.text, next.text)
    } else {
        next.text
    };
    let mut denials = prev.denials;
    denials.extend(next.denials);
    Outcome {
        ok: next.ok,
        text,
        subtype: next.subtype,
        turns: prev.turns + next.turns,
        duration_ms: next.duration_ms,
        denials,
        limit: next.limit,
    }
}

/// How a stored run ended, from its transcript: every result segment merged. `None` when the run did
/// not end its turn (it was killed mid-turn). A run the usage limit put back in the queue appends each
/// attempt to the same transcript, so a new session's init opens a turn the earlier result did not end.
pub fn transcript_outcome(transcript: &str, home: &Path) -> Option<Outcome> {
    let mut outcome: Option<Outcome> = None;
    let mut ended = false;
    for line in transcript.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        match v["type"].as_str() {
            _ if is_nested(&v) => {}
            Some("result") => {
                ended = true;
                if let Some(next) = parse_line(line, home).outcome {
                    outcome = Some(match outcome {
                        Some(prev) => merge_outcome(prev, next),
                        None => next,
                    });
                }
            }
            Some("assistant" | "user") => ended = false,
            Some("system") if v["subtype"] == "init" => ended = false,
            _ => {}
        }
    }
    outcome.filter(|_| ended)
}

/// The input of every top-level call of `tool` that did not come back as an error, in order.
pub fn office_calls(transcript: &str, tool: &str) -> Vec<Value> {
    let mut calls = vec![];
    let mut failed = std::collections::HashSet::new();
    for v in json_lines(transcript).filter(|v| !is_nested(v)) {
        let blocks = v["message"]["content"].as_array().cloned().unwrap_or_default();
        match v["type"].as_str() {
            Some("assistant") => {
                for b in blocks.into_iter().filter(|b| b["type"] == "tool_use" && b["name"] == tool) {
                    calls.push((b["id"].as_str().unwrap_or("").to_string(), b["input"].clone()));
                }
            }
            Some("user") => {
                for b in blocks.iter().filter(|b| b["type"] == "tool_result" && b["is_error"].as_bool().unwrap_or(false)) {
                    failed.insert(b["tool_use_id"].as_str().unwrap_or("").to_string());
                }
            }
            _ => {}
        }
    }
    calls.into_iter().filter(|(id, _)| !failed.contains(id)).map(|(_, input)| input).collect()
}

/// Your messages steered into the run while it worked, in order.
pub fn steered(transcript: &str) -> Vec<String> {
    json_lines(transcript)
        .filter(|v| v["type"] == "omega")
        .filter_map(|v| match serde_json::from_value::<RunEvent>(v["event"].clone()) {
            Ok(RunEvent::User { text }) => Some(text),
            _ => None,
        })
        .collect()
}

/// What a Chrome tool returns when the browser extension is not connected (not marked as an error).
pub const CHROME_DOWN: &str = "Browser extension is not connected";

/// What a run's stream says beyond its events, gathered as it goes: which call each result answers,
/// why calls were denied, and whether Chrome dropped.
#[derive(Default)]
pub struct RunSignals {
    /// tool_use_id -> (seq, tool)
    calls: HashMap<String, (usize, String)>,
    denied: HashMap<String, DeniedReason>,
    /// tool -> seq of its latest ok result
    last_ok: HashMap<String, usize>,
    seq: usize,
    chrome_told: bool,
}

impl RunSignals {
    pub fn observe(&mut self, p: &Parsed) {
        for ev in &p.events {
            self.seq += 1;
            match ev {
                RunEvent::ToolCall { id, tool, .. } => {
                    self.calls.insert(id.clone(), (self.seq, tool.clone()));
                }
                RunEvent::ToolResult { id, ok: true, .. } => {
                    if let Some((_, tool)) = self.calls.get(id) {
                        self.last_ok.insert(tool.clone(), self.seq);
                    }
                }
                _ => {}
            }
        }
        if let Some(d) = &p.denied {
            self.denied.insert(d.tool_use_id.clone(), d.clone());
        }
    }

    /// True, once per run, when a Chrome tool reports that the extension is not connected. Call
    /// `observe` on the same line first.
    pub fn chrome_down(&mut self, events: &[RunEvent]) -> bool {
        if self.chrome_told {
            return false;
        }
        self.chrome_told = events.iter().any(|ev| match ev {
            RunEvent::ToolResult { id, brief, .. } => {
                brief.contains(CHROME_DOWN) && self.calls.get(id).is_some_and(|(_, tool)| tool.starts_with("mcp__claude-in-chrome__"))
            }
            _ => false,
        });
        self.chrome_told
    }

    /// Sorts a run's denials into real blocks and report notes. Safety checks are tool errors; a
    /// safety-classifier stop in an unattended run is a note. Any other denial stays a block unless an
    /// allow rule already covers it or a later call of the same tool worked.
    pub fn sort_denials(&self, denied: Vec<Denied>, covered: impl Fn(&Denial) -> bool, unattended: bool) -> (Vec<Denial>, Vec<String>) {
        let mut blocks = vec![];
        let mut notes = vec![];
        for d in denied {
            let reason = self.denied.get(&d.tool_use_id);
            match reason.map_or("unknown", |r| r.kind.as_str()) {
                "safetyCheck" | "subcommandResults" => {}
                "classifier" => {
                    if unattended {
                        let why = reason.map(|r| r.reason.replace(['[', ']'], "")).unwrap_or_default();
                        notes.push(format!("Claude Code's safety check stopped one step: {}.", why.trim()));
                    }
                }
                _ => {
                    let got_past = self
                        .calls
                        .get(&d.tool_use_id)
                        .is_some_and(|(seq, tool)| self.last_ok.get(tool).is_some_and(|ok| ok > seq));
                    if !covered(&d.denial) && !got_past {
                        blocks.push(d.denial);
                    }
                }
            }
        }
        (blocks, notes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic lines shaped like Claude Code 2.1.296's stream-json output.
    const INIT: &str = r#"{"type":"system","subtype":"init","cwd":"/tmp/office/employees/tester","session_id":"sess-1","tools":["Bash","Read"],"mcp_servers":[{"name":"omega","status":"connected"}],"model":"claude-sonnet-4-5","permissionMode":"default","skills":["notes","weekly-report"],"apiKeySource":"none","claude_code_version":"2.1.296","uuid":"u-1"}"#;

    const MULTI_RESULT: &str = r#"{"type":"assistant","message":{"model":"claude-sonnet-4-5","role":"assistant","content":[{"type":"text","text":"Reading the sample sheet."}],"usage":{"input_tokens":3,"cache_read_input_tokens":1200,"cache_creation_input_tokens":40,"output_tokens":12}},"parent_tool_use_id":null,"session_id":"sess-1"}
{"type":"result","subtype":"success","is_error":false,"num_turns":3,"duration_ms":4200,"result":"Report line one.\nReport line two.","permission_denials":[],"result_index":0,"session_id":"sess-1"}
{"type":"omega","event":{"type":"user","text":"Add the totals too."}}
{"type":"assistant","message":{"model":"claude-sonnet-4-5","role":"assistant","content":[{"type":"text","text":"Adding the totals."}],"usage":{"input_tokens":2,"cache_read_input_tokens":1300,"cache_creation_input_tokens":30,"output_tokens":8}},"parent_tool_use_id":null,"session_id":"sess-1"}
{"type":"result","subtype":"success","is_error":false,"num_turns":2,"duration_ms":1800,"result":"Totals added.","permission_denials":[],"result_index":1,"session_id":"sess-1"}"#;

    const DENIAL: &str = r#"{"type":"assistant","message":{"model":"claude-sonnet-4-5","role":"assistant","content":[{"type":"tool_use","id":"toolu_den1","name":"mcp__claude-in-chrome__get_page_text","input":{"tabId":101}}],"usage":{"input_tokens":4,"cache_read_input_tokens":900,"cache_creation_input_tokens":10,"output_tokens":20}},"parent_tool_use_id":null,"session_id":"sess-2"}
{"type":"system","subtype":"permission_denied","tool_name":"mcp__claude-in-chrome__get_page_text","tool_use_id":"toolu_den1","decision_reason_type":"safetyCheck","decision_reason":"Claude in Chrome: the tab is not a web page","message":"Cannot use this tab.","session_id":"sess-2"}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_den1","is_error":true,"content":"Cannot use this tab."}]},"parent_tool_use_id":null,"session_id":"sess-2"}
{"type":"result","subtype":"success","is_error":false,"num_turns":2,"duration_ms":900,"result":"Could not read the page.","permission_denials":[{"tool_name":"mcp__claude-in-chrome__get_page_text","tool_use_id":"toolu_den1","tool_input":{"tabId":101}}],"result_index":0,"session_id":"sess-2"}"#;

    const CLASSIFIER: &str = r#"{"type":"assistant","message":{"model":"claude-sonnet-4-5","role":"assistant","content":[{"type":"tool_use","id":"toolu_cls1","name":"mcp__claude-in-chrome__get_page_text","input":{"tabId":102}}],"usage":{"input_tokens":4,"cache_read_input_tokens":900,"cache_creation_input_tokens":10,"output_tokens":20}},"parent_tool_use_id":null,"session_id":"sess-3"}
{"type":"system","subtype":"permission_denied","tool_name":"mcp__claude-in-chrome__get_page_text","tool_use_id":"toolu_cls1","decision_reason_type":"classifier","decision_reason":"[Irreversible Deletion (general)]","message":"Denied by the classifier.","session_id":"sess-3"}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_cls1","is_error":true,"content":"Denied by the classifier."}]},"parent_tool_use_id":null,"session_id":"sess-3"}
{"type":"result","subtype":"success","is_error":false,"num_turns":2,"duration_ms":900,"result":"Skipped one step.","permission_denials":[{"tool_name":"mcp__claude-in-chrome__get_page_text","tool_use_id":"toolu_cls1","tool_input":{"tabId":102}}],"result_index":0,"session_id":"sess-3"}"#;

    const LIMIT: &str = r#"{"type":"assistant","message":{"model":"claude-sonnet-4-5","role":"assistant","content":[{"type":"text","text":"Starting on the inbox."}],"usage":{"input_tokens":5,"cache_read_input_tokens":20000,"cache_creation_input_tokens":500,"output_tokens":30}},"parent_tool_use_id":null,"session_id":"sess-4"}
{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1800000000,"rateLimitType":"five_hour","isUsingOverage":false,"unifiedWindows":{"five_hour":{"utilization":1,"resetsAt":1800000000}}},"session_id":"sess-4"}
{"type":"assistant","message":{"model":"<synthetic>","role":"assistant","content":[{"type":"text","text":"You have hit your limit."}],"usage":{"input_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"output_tokens":0}},"parent_tool_use_id":null,"session_id":"sess-4","error":"rate_limit","api_error":"usage_limit_reached"}
{"type":"result","subtype":"success","is_error":true,"num_turns":1,"duration_ms":530,"result":"You have hit your limit.","permission_denials":[],"api_error_status":429,"api_error":"usage_limit_reached","result_index":0,"session_id":"sess-4"}"#;

    const RATE_LIMIT: &str = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":1800003600,"rateLimitType":"five_hour","unifiedWindows":{"five_hour":{"utilization":0.42,"resetsAt":1800003600},"seven_day":{"utilization":0.1,"resetsAt":1800500000}}},"session_id":"sess-5"}"#;

    const CHROME: &str = r#"{"type":"assistant","message":{"model":"claude-sonnet-4-5","role":"assistant","content":[{"type":"tool_use","id":"toolu_chr1","name":"mcp__claude-in-chrome__tabs_context_mcp","input":{}}],"usage":{"input_tokens":4,"cache_read_input_tokens":900,"cache_creation_input_tokens":10,"output_tokens":20}},"parent_tool_use_id":null,"session_id":"sess-6"}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_chr1","content":[{"type":"text","text":"Browser extension is not connected. Please ensure the extension is installed and running."}]}]},"parent_tool_use_id":null,"session_id":"sess-6"}"#;

    fn home() -> &'static Path {
        Path::new("/tmp/office/employees/tester")
    }

    fn parsed(transcript: &str) -> Vec<Parsed> {
        transcript.lines().map(|l| parse_line(l, home())).collect()
    }

    fn last_outcome(transcript: &str) -> Outcome {
        parsed(transcript).into_iter().filter_map(|p| p.outcome).last().expect("a result")
    }

    fn signals(transcript: &str) -> RunSignals {
        let mut s = RunSignals::default();
        for p in parsed(transcript) {
            s.observe(&p);
        }
        s
    }

    fn call(id: &str, tool: &str, input: Value) -> String {
        json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "model": "claude-sonnet-4-5", "content": [{ "type": "tool_use", "id": id, "name": tool, "input": input }] } }).to_string()
    }

    fn answer(id: &str, error: bool, text: &str) -> String {
        json!({ "type": "user", "parent_tool_use_id": null, "message": { "content": [{ "type": "tool_result", "tool_use_id": id, "is_error": error, "content": text }] } }).to_string()
    }

    fn outcome(text: &str, turns: u32) -> Outcome {
        Outcome { ok: true, text: text.into(), subtype: "success".into(), turns, duration_ms: 10, denials: vec![], limit: false }
    }

    #[test]
    fn init_names_model_skills_and_claude_version() {
        let p = parse_line(INIT, home());
        assert_eq!(p.events, vec![RunEvent::Init { model: "claude-sonnet-4-5".into(), skills: vec!["notes".into(), "weekly-report".into()] }]);
        assert_eq!(p.claude_version.as_deref(), Some("2.1.296"));
    }

    #[test]
    fn multi_result_runs_report_each_segment() {
        let events: Vec<RunEvent> = replay(MULTI_RESULT, home()).into_iter().filter(|e| !matches!(e, RunEvent::Text { .. })).collect();
        assert_eq!(events.len(), 3);
        assert!(matches!(&events[0], RunEvent::Result { turns: 3, .. }));
        assert_eq!(events[1], RunEvent::User { text: "Add the totals too.".into() });
        assert!(matches!(&events[2], RunEvent::Result { turns: 2, .. }));
    }

    #[test]
    fn a_denial_is_read_from_the_result() {
        let lines = parsed(DENIAL);
        assert!(lines[1].events.is_empty());
        let reason = lines[1].denied.as_ref().expect("a denial reason");
        assert_eq!((reason.tool_use_id.as_str(), reason.kind.as_str()), ("toolu_den1", "safetyCheck"));
        let o = last_outcome(DENIAL);
        assert_eq!(o.denials.len(), 1);
        assert_eq!(o.denials[0].tool_use_id, "toolu_den1");
        assert_eq!(o.denials[0].denial.suggested_rule, "mcp__claude-in-chrome__get_page_text");
    }

    #[test]
    fn rate_limit_event_reads_the_five_hour_window() {
        let p = parse_line(RATE_LIMIT, home());
        let usage = p.usage.expect("a usage sample");
        assert_eq!(usage.five_hour.expect("five hour").percent, 42.0);
        assert_eq!(usage.seven_day.expect("seven day").percent, 10.0);
        assert!(p.limited.is_none());
    }

    #[test]
    fn a_rejected_limit_sets_limited_and_limit() {
        let lines = parsed(LIMIT);
        assert_eq!(lines[1].limited, chrono::DateTime::from_timestamp(1_800_000_000, 0));
        assert!(lines[1].usage.is_some());
        let o = last_outcome(LIMIT);
        assert!(o.limit);
        assert!(!o.ok);
        assert!(!last_outcome(MULTI_RESULT).limit);
    }

    #[test]
    fn the_synthetic_line_keeps_the_context() {
        let last = parsed(LIMIT).into_iter().filter_map(|p| p.context).last();
        assert_eq!(last, Some(20_505));
    }

    #[test]
    fn subagent_lines_stay_out() {
        let line = r#"{"type":"assistant","message":{"model":"claude-sonnet-4-5","content":[{"type":"text","text":"Sub-agent work."}],"usage":{"input_tokens":7,"cache_read_input_tokens":5000,"cache_creation_input_tokens":0}},"parent_tool_use_id":"toolu_task1"}"#;
        let p = parse_line(line, home());
        assert!(p.events.is_empty());
        assert!(p.context.is_none());
    }

    #[test]
    fn context_sums_usage() {
        let line = r#"{"type":"assistant","message":{"model":"claude-sonnet-4-5","content":[],"usage":{"input_tokens":2,"cache_read_input_tokens":31557,"cache_creation_input_tokens":224,"output_tokens":90}},"parent_tool_use_id":null}"#;
        assert_eq!(parse_line(line, home()).context, Some(31_783));
    }

    #[test]
    fn a_one_liner_is_added_to_a_report() {
        let report = "Line one.\nLine two.";
        let m = merge_outcome(outcome(report, 3), outcome("Done.", 2));
        assert_eq!(m.text, "Line one.\nLine two.\n\nDone.");
        assert_eq!(m.turns, 5);
    }

    #[test]
    fn a_long_final_replaces_one_liners() {
        let long = "x".repeat(250);
        let first = (0..5).map(|i| outcome(&format!("Step {i}."), 1)).reduce(merge_outcome).unwrap();
        let m = merge_outcome(first, outcome(&long, 1));
        assert_eq!(m.text, long);
        assert_eq!(m.turns, 6);
    }

    #[test]
    fn an_empty_later_text_keeps_the_earlier() {
        let m = merge_outcome(outcome("All done.", 1), outcome("", 1));
        assert_eq!(m.text, "All done.");
    }

    #[test]
    fn transcript_outcome_needs_a_final_result() {
        let o = transcript_outcome(MULTI_RESULT, home()).expect("an outcome");
        assert_eq!(o.turns, 5);
        assert_eq!(o.text, "Report line one.\nReport line two.\n\nTotals added.");
        let mid_turn = format!("{INIT}\n{}", call("toolu_x1", "Bash", json!({ "command": "ls" })));
        assert!(transcript_outcome(&mid_turn, home()).is_none());
        assert!(transcript_outcome(&format!("{MULTI_RESULT}\n{{\"type\":\"assistant\",\"mess"), home()).is_some());
        let cut = format!("{INIT}\n{}\n{{\"type\":\"resu", MULTI_RESULT.lines().next().unwrap());
        assert!(transcript_outcome(&cut, home()).is_none());
        // A sub-agent still finishing after the final result does not reopen the turn.
        let nested = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"done"}]},"parent_tool_use_id":"toolu_sub"}"#;
        assert!(transcript_outcome(&format!("{MULTI_RESULT}\n{nested}"), home()).is_some());
        // A retry after the usage limit that stopped before its first message has not ended.
        let status = r#"{"type":"system","subtype":"status","status":"requesting"}"#;
        assert!(transcript_outcome(&format!("{LIMIT}\n{INIT}\n{status}"), home()).is_none());
        let retried = transcript_outcome(&format!("{LIMIT}\n{INIT}\n{MULTI_RESULT}"), home()).expect("an outcome");
        assert!(!retried.limit);
        assert_eq!(retried.turns, 6, "both attempts' turns");
    }

    #[test]
    fn office_calls_skips_errors_and_half_lines() {
        let notify = "mcp__omega__notify";
        let transcript = [
            call("t1", notify, json!({ "message": "First" })),
            answer("t1", false, "ok"),
            call("t2", notify, json!({ "message": "Second" })),
            answer("t2", true, "failed"),
            call("t3", "Bash", json!({ "command": "ls" })),
            call("t4", notify, json!({ "message": "Third" })),
            answer("t4", false, "ok"),
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t5","name":"mcp__omega__notify","input":{"mess"#.to_string(),
        ]
        .join("\n");
        assert_eq!(office_calls(&transcript, notify), vec![json!({ "message": "First" }), json!({ "message": "Third" })]);
    }

    #[test]
    fn steered_returns_your_messages() {
        let transcript = format!("{MULTI_RESULT}\n{}", omega_line(&RunEvent::User { text: "One more thing.".into() }));
        assert_eq!(steered(&transcript), vec!["Add the totals too.".to_string(), "One more thing.".to_string()]);
    }

    /// A Bash denial of kind `mode`, optionally followed by a Bash call that worked.
    fn mode_denial(later_ok: bool) -> String {
        let mut lines = vec![
            call("toolu_git1", "Bash", json!({ "command": "git push origin main" })),
            r#"{"type":"system","subtype":"permission_denied","tool_name":"Bash","tool_use_id":"toolu_git1","decision_reason_type":"mode","decision_reason":"Not allowed without asking"}"#.to_string(),
            answer("toolu_git1", true, "Permission denied."),
        ];
        if later_ok {
            lines.push(call("toolu_ls1", "Bash", json!({ "command": "ls" })));
            lines.push(answer("toolu_ls1", false, "notes.md"));
        }
        lines.push(r#"{"type":"result","subtype":"success","is_error":false,"num_turns":2,"duration_ms":700,"result":"Done.","permission_denials":[{"tool_name":"Bash","tool_use_id":"toolu_git1","tool_input":{"command":"git push origin main"}}]}"#.to_string());
        lines.join("\n")
    }

    fn sort(transcript: &str, covered: bool, unattended: bool) -> (Vec<Denial>, Vec<String>) {
        signals(transcript).sort_denials(last_outcome(transcript).denials, |_| covered, unattended)
    }

    #[test]
    fn safety_checks_are_not_blocks() {
        assert_eq!(sort(DENIAL, false, true), (vec![], vec![]));
    }

    #[test]
    fn a_classifier_stop_is_a_note_when_unattended() {
        let (blocks, notes) = sort(CLASSIFIER, false, true);
        assert!(blocks.is_empty());
        assert_eq!(notes, vec!["Claude Code's safety check stopped one step: Irreversible Deletion (general).".to_string()]);
        assert_eq!(sort(CLASSIFIER, false, false), (vec![], vec![]));
    }

    #[test]
    fn a_mode_denial_is_a_block_until_got_past_or_covered() {
        let (blocks, notes) = sort(&mode_denial(false), false, true);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].suggested_rule, "Bash(git:*)");
        assert!(notes.is_empty());
        assert!(sort(&mode_denial(true), false, true).0.is_empty());
        assert!(sort(&mode_denial(false), true, true).0.is_empty());
    }

    fn chrome_downs(transcript: &str) -> usize {
        let mut s = RunSignals::default();
        parsed(transcript)
            .into_iter()
            .filter(|p| {
                s.observe(p);
                s.chrome_down(&p.events)
            })
            .count()
    }

    #[test]
    fn chrome_down_is_told_once() {
        assert_eq!(chrome_downs(CHROME), 1);
        assert_eq!(chrome_downs(&format!("{CHROME}\n{CHROME}")), 1);
        let quoted = format!("{}\n{}", call("toolu_b1", "Bash", json!({ "command": "grep -r notes" })), answer("toolu_b1", false, "notes.md: Browser extension is not connected"));
        assert_eq!(chrome_downs(&quoted), 0);
        let slow = format!(
            "{}\n{}",
            call("toolu_c2", "mcp__claude-in-chrome__tabs_context_mcp", json!({})),
            answer("toolu_c2", false, "The browser did not respond within 8 seconds.")
        );
        assert_eq!(chrome_downs(&slow), 0);
    }
}
