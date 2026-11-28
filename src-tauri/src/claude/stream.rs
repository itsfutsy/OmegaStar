//! Parsing Claude Code's stream-json output into run events.

use super::*;
use std::borrow::Cow;
use std::collections::HashMap;

#[derive(Clone)]
pub struct Outcome {
    pub ok: bool,
    pub text: String,
    pub subtype: String,
    pub turns: u32,
    pub duration_ms: u64,
    pub denials: Vec<Denied>,
    /// A limit stopped the run.
    pub limit: Option<Limit>,
    /// The segment answered a background task's notice that came after the run's turn, not a message.
    pub notice: bool,
}

/// A limit that stops a run, which OmegaStar waits out instead of failing the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// The subscription's usage limit, which resets at a set time.
    Usage,
    /// Claude, or a proxy in front of it, turning requests away for now (HTTP 429 or 529). Its result may state how
    /// long for (`eases_in`).
    Transient,
}

impl Limit {
    pub fn as_str(self) -> &'static str {
        match self {
            Limit::Usage => "the usage limit",
            Limit::Transient => "a rate limit",
        }
    }
}

/// What a result line says the run spent: its own cost since the previous result (each model's `costUSD` in
/// `model_usage_since_last_result`), and the session's running total (`total_cost_usd`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Spend {
    pub own: Option<f64>,
    pub total: Option<f64>,
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
    /// Requests were turned away for now: a rejected rate limit with no reset time, or Claude Code's error
    /// message for a rate limit or an overload.
    pub throttled: bool,
    pub denied: Option<DeniedReason>,
    /// The context size in tokens after this top-level assistant message.
    pub context: Option<u64>,
    /// From an init line: Claude in Chrome's tools are among the run's tools.
    pub chrome: Option<bool>,
    /// From a top-level result line.
    pub spend: Option<Spend>,
    /// From a top-level result line: the tokens used since the previous result, model by model.
    pub tokens: Vec<ModelTokens>,
    /// Claude Code took this message from stdin: the id it was sent with (`steer_line`). Its echo is not kept.
    pub replayed: Option<String>,
}

/// One observation of the subscription usage windows. Both `None` means the account has none.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UsageSample {
    pub five_hour: Option<UsageWindow>,
    pub seven_day: Option<UsageWindow>,
    /// A CLI proxy's accounts, one by one; empty for a single account.
    pub accounts: Vec<AccountUsage>,
}

fn percent_of(utilization: f64) -> f64 {
    ((utilization * 1000.0).round() / 10.0).clamp(0.0, 100.0)
}

/// Wraps a synthetic Office event as a transcript line, so replay needs only the transcript.
pub fn omega_line(ev: &RunEvent) -> String {
    json!({ "type": "omega", "event": ev }).to_string()
}

const BRIEF_KEYS: &[&str] = &[
    "command", "file_path", "notebook_path", "path", "pattern", "url", "query", "skill", "description", "prompt", "task", "person", "title",
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

fn suggested_rule(tool: &str, input: &Value) -> String {
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
const CHROME_TOOLS: &str = "mcp__claude-in-chrome__";

/// What a failed result says when Claude or a proxy is turning requests away for now, in older builds that send no
/// status with it.
const TURNED_AWAY: [&str; 2] = ["temporarily limiting requests", "cooling down"];

/// The wait a turned-away result states, as a proxy prints it in hours, minutes and seconds: `(reset after 12m30s)`,
/// `1m27.5s`. `None` when it states none, one in another unit (`500ms`), or one that rounds to no time. Pure.
pub fn eases_in(text: &str) -> Option<chrono::Duration> {
    let (_, after) = text.rsplit_once("reset after ")?;
    let mut rest = after.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.')).next()?.trim_end_matches('.');
    let mut secs = 0.0;
    while !rest.is_empty() {
        let (n, tail) = rest.split_at(rest.find(|c: char| c.is_ascii_alphabetic())?);
        let (unit, tail) = tail.split_at(tail.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(tail.len()));
        let scale = match unit {
            "h" => 3600.0,
            "m" => 60.0,
            "s" => 1.0,
            _ => return None,
        };
        secs += n.parse::<f64>().ok()? * scale;
        rest = tail;
    }
    let ms = (secs * 1000.0).round() as i64;
    (ms > 0).then(|| chrono::Duration::milliseconds(ms))
}

/// The line carries the HTTP status of a request turned away for now: 429 rate limited or 529 overloaded.
fn turned_away_status(v: &Value) -> bool {
    matches!(v["api_error_status"].as_u64(), Some(429 | 529))
}

/// The limit a result line says stopped the run. Pure.
fn limit_of(v: &Value, ok: bool) -> Option<Limit> {
    if v["api_error"] == "usage_limit_reached" {
        return Some(Limit::Usage);
    }
    let text = v["result"].as_str().unwrap_or("");
    let turned_away = turned_away_status(v) || TURNED_AWAY.iter().any(|t| text.contains(t));
    (!ok && turned_away).then_some(Limit::Transient)
}

/// What a result line says the run spent. An empty `model_usage_since_last_result` means nothing since the last one.
fn spend_of(v: &Value) -> Spend {
    let own = v["model_usage_since_last_result"].as_object().and_then(|m| m.values().map(|u| u["costUSD"].as_f64()).sum::<Option<f64>>());
    Spend { own, total: v["total_cost_usd"].as_f64() }
}

/// The tokens a result line says each model used since the previous result (`model_usage_since_last_result`), less
/// cache re-reads (`ModelTokens`). A model that used none is left out.
fn tokens_of(v: &Value) -> Vec<ModelTokens> {
    let count = |u: &Value| ["inputTokens", "cacheCreationInputTokens", "outputTokens"].iter().filter_map(|k| u[k].as_u64()).sum::<u64>();
    v["model_usage_since_last_result"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(model, u)| ModelTokens { model: model.clone(), tokens: count(u) })
        .filter(|m| m.tokens > 0)
        .collect()
}

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
    let nested = is_nested(&v);
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
            p.chrome = Some(v["tools"].as_array().into_iter().flatten().any(|t| t.as_str().is_some_and(|t| t.starts_with(CHROME_TOOLS))));
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
            p.throttled = v["error"] == "rate_limit" || turned_away_status(&v);
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
        "user" if v["isReplay"] == true => p.replayed = Some(str_at(&v, "uuid")),
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
            let ok = !v["is_error"].as_bool().unwrap_or(false) && v["subtype"] == "success";
            let o = Outcome {
                ok,
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
                limit: limit_of(&v, ok),
                notice: v["origin"]["kind"] == "task-notification",
            };
            p.events.push(RunEvent::Result {
                ok: o.ok,
                text: o.text.clone(),
                turns: o.turns,
                duration_ms: o.duration_ms,
            });
            p.outcome = Some(o);
            if !nested {
                p.spend = Some(spend_of(&v));
                p.tokens = tokens_of(&v);
            }
        }
        "rate_limit_event" => {
            let info = &v["rate_limit_info"];
            if info["status"] == "rejected" {
                p.limited = info["resetsAt"].as_i64().or(v["resetsAt"].as_i64()).and_then(|s| chrono::DateTime::from_timestamp(s, 0));
                p.throttled = p.limited.is_none();
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
            let sample = UsageSample { five_hour: window("five_hour"), seven_day: window("seven_day"), accounts: vec![] };
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
    agents: HashSet<String>,
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
    let mut newest = HashMap::new();
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
pub(super) fn json_lines(transcript: &str) -> impl DoubleEndedIterator<Item = Value> + '_ {
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
/// of its own, and so does a background task's notice the run answers after its turn. A trailing one-liner
/// is added to a real report instead of replacing it, and so is what answered a notice: the report that
/// answered you stays first. A notice that failed after the report succeeded, a limit included, leaves the run
/// as the report left it: its work was done.
pub fn merge_outcome(prev: Outcome, next: Outcome) -> Outcome {
    let mut denials = prev.denials;
    denials.extend(next.denials);
    let turns = prev.turns + next.turns;
    if next.notice && !next.ok && prev.ok {
        return Outcome { turns, denials, limit: None, ..prev };
    }
    let text = if next.text.trim().is_empty() {
        prev.text
    } else if (next.notice && !prev.text.trim().is_empty()) || (one_liner(&next.text) && !one_liner(&prev.text)) {
        format!("{}\n\n{}", prev.text, next.text)
    } else {
        next.text
    };
    Outcome {
        ok: next.ok,
        text,
        subtype: next.subtype,
        turns,
        duration_ms: next.duration_ms,
        denials,
        limit: next.limit,
        notice: next.notice,
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
    let mut failed = HashSet::new();
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
    omega_events(transcript)
        .filter_map(|ev| match ev {
            RunEvent::User { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

/// Your messages the employee never took, each with its id: the run stopped before its next step. Pure.
pub fn unheard(transcript: &str) -> Vec<(String, String)> {
    let mut sent = vec![];
    let mut heard = HashSet::new();
    for ev in omega_events(transcript) {
        match ev {
            RunEvent::User { text, id: Some(id) } => sent.push((id, text)),
            RunEvent::Heard { id } => {
                heard.insert(id);
            }
            _ => {}
        }
    }
    sent.into_iter().filter(|(id, _)| !heard.contains(id)).collect()
}

fn omega_events(transcript: &str) -> impl Iterator<Item = RunEvent> + '_ {
    json_lines_with(transcript, "\"omega\"").filter(|v| v["type"] == "omega").filter_map(|v| serde_json::from_value::<RunEvent>(v["event"].clone()).ok())
}

/// The session's running total that the transcript's last top-level result reported.
pub fn last_total(transcript: &str) -> Option<f64> {
    json_lines_with(transcript, "\"total_cost_usd\"").filter(|v| v["type"] == "result" && !is_nested(v)).filter_map(|v| v["total_cost_usd"].as_f64()).last()
}

/// The lines holding `needle` that parse as JSON, so a long transcript is not parsed whole.
fn json_lines_with<'a>(transcript: &'a str, needle: &'a str) -> impl Iterator<Item = Value> + 'a {
    transcript.lines().filter(move |l| l.contains(needle)).filter_map(|l| serde_json::from_str::<Value>(l).ok())
}

/// What a run's result segments cost in US dollars: each one's own cost when it reports it, else how far it took
/// the session's running total past the total before it, which for the first is `before` (0 for a new session,
/// `None` when unknown). That total runs on across segments and resumed runs, so it is never added up itself.
/// `None` when no segment's cost is known. Pure.
pub fn run_cost(segments: &[Spend], before: Option<f64>) -> Option<f64> {
    let mut last = before;
    let mut sum: Option<f64> = None;
    for s in segments {
        let rise = || -> Option<f64> { Some((s.total? - last?).max(0.0)) };
        if let Some(cost) = s.own.or_else(rise) {
            sum = Some(sum.unwrap_or(0.0) + cost);
        }
        last = s.total.or(last);
    }
    sum
}

/// A transcript line without the image data Claude Code keeps in tool results, which the app never shows: each
/// image's base64 becomes a short note of its media type and size, and the line stays valid JSON. Any other line
/// comes back as it was, and so does one already stripped.
pub fn without_images(line: &str) -> Cow<'_, str> {
    if !line.contains("base64") {
        return Cow::Borrowed(line);
    }
    let Ok(mut v) = serde_json::from_str::<Value>(line) else { return Cow::Borrowed(line) };
    if strip_images(&mut v) {
        Cow::Owned(v.to_string())
    } else {
        Cow::Borrowed(line)
    }
}

/// Replaces the data of every image in `v`: an image block's `source.data`, and the Read tool's own result's
/// `file.base64`. True when it replaced any.
fn strip_images(v: &mut Value) -> bool {
    match v {
        Value::Object(o) => {
            let mut changed = false;
            if o.get("type").and_then(Value::as_str) == Some("image") {
                for (inner, kind, data) in [("source", "media_type", "data"), ("file", "type", "base64")] {
                    if let Some(x) = o.get_mut(inner).and_then(Value::as_object_mut) {
                        changed |= note_image(x, kind, data);
                    }
                }
            }
            o.values_mut().fold(changed, |c, x| strip_images(x) | c)
        }
        Value::Array(a) => a.iter_mut().fold(false, |c, x| strip_images(x) | c),
        _ => false,
    }
}

/// Puts a note such as `[image/png, 12 KB, left out]` in place of base64 image data. A note is never base64,
/// which has no `[`, so a second pass leaves it alone.
fn note_image(o: &mut serde_json::Map<String, Value>, kind: &str, data: &str) -> bool {
    let Some(len) = o.get(data).and_then(Value::as_str).filter(|d| !d.starts_with('[')).map(str::len) else { return false };
    let media = o.get(kind).and_then(Value::as_str).unwrap_or("image").to_string();
    o.insert(data.into(), Value::String(format!("[{media}, {} KB, left out]", (len * 3 / 4).div_ceil(1024))));
    true
}

/// The line a run's report gets when Claude Code stopped helpers of the run (sub-agents, monitors, shells) that
/// were still working when its turn ended, naming each one stderr names. `None` when it stopped none.
pub fn stopped_helpers(stderr: &str) -> Option<String> {
    let lines: Vec<&str> = stderr.lines().filter(|l| l.contains("Background tasks still running")).collect();
    if lines.is_empty() {
        return None;
    }
    // "... after the last turn (subagent \"Check the totals\", shell \"npm test\"); stopping them."
    let names: Vec<String> = lines
        .iter()
        .filter_map(|l| l.split_once('(').and_then(|(_, rest)| rest.rsplit_once(')')).map(|(inside, _)| inside))
        .flat_map(|inside| inside.split('"').skip(1).step_by(2))
        .map(|n| format!("\"{}\"", clip(&one_line(n), 80)))
        .collect();
    Some(match names.len() {
        0 => "Claude Code stopped helpers that had not finished when the run ended.".to_string(),
        1 => format!("Claude Code stopped a helper that had not finished when the run ended: {}.", names[0]),
        n => format!("Claude Code stopped {n} helpers that had not finished when the run ended: {}.", names.join(", ")),
    })
}

/// What a Chrome tool returns when the browser extension is not connected (not marked as an error).
const CHROME_DOWN: &str = "Browser extension is not connected";

/// Chrome counts as down once its calls keep finding the extension not connected: this many in a row, or for this
/// many seconds, with none working since. The first call of a run often loses a race and the next one works.
const CHROME_FAILURES: u32 = 3;
const CHROME_GRACE_SECS: i64 = 45;

/// What a run's stream says beyond its events, gathered as it goes: which call each result answers,
/// why calls were denied, whether Chrome dropped, and whether requests were turned away.
#[derive(Default)]
pub struct RunSignals {
    /// tool_use_id -> (seq, tool)
    calls: HashMap<String, (usize, String)>,
    denied: HashMap<String, DeniedReason>,
    /// tool -> seq of its latest ok result
    last_ok: HashMap<String, usize>,
    seq: usize,
    chrome_told: bool,
    /// The Chrome calls that found the extension not connected since one last worked: when the first came back,
    /// and how many.
    chrome_failing: Option<(Ts, u32)>,
    /// What the run's first init line said: whether Claude in Chrome's tools loaded.
    chrome_loaded: Option<bool>,
    /// Requests were turned away (`Parsed::throttled`) since the model last answered.
    throttled: bool,
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
        if let Some(loaded) = p.chrome {
            self.chrome_loaded.get_or_insert(loaded);
        }
        self.throttled = (self.throttled || p.throttled) && p.context.is_none();
    }

    /// The limit that stopped a result segment: what its result says, or requests turned away just before a
    /// result that failed. Call `observe` on the result line first.
    pub fn limit(&mut self, o: &Outcome) -> Option<Limit> {
        let throttled = std::mem::take(&mut self.throttled);
        o.limit.or((!o.ok && throttled).then_some(Limit::Transient))
    }

    /// True, once per run, when Chrome is down: its calls keep finding the extension not connected, `CHROME_FAILURES`
    /// in a row or for `CHROME_GRACE_SECS`, and none worked since. A tab that fails to close is not Chrome being
    /// down. Call `observe` on the same line first, and on every line, since the time can run out on any.
    pub fn chrome_down(&mut self, events: &[RunEvent], at: Ts) -> bool {
        if self.chrome_told {
            return false;
        }
        for ev in events {
            let RunEvent::ToolResult { id, ok, brief } = ev else { continue };
            if !self.calls.get(id).is_some_and(|(_, tool)| tool.starts_with(CHROME_TOOLS) && !tool.contains("tabs_close")) {
                continue;
            }
            if brief.contains(CHROME_DOWN) {
                let (since, n) = self.chrome_failing.unwrap_or((at, 0));
                self.chrome_failing = Some((since, n + 1));
            } else if *ok {
                self.chrome_failing = None;
            }
        }
        self.chrome_told = self.chrome_failing.is_some_and(|(since, n)| n >= CHROME_FAILURES || at - since >= chrono::Duration::seconds(CHROME_GRACE_SECS));
        self.chrome_told
    }

    /// The employee may use Chrome, but the run's first init line listed none of Claude in Chrome's tools: the
    /// extension's tools never loaded, which reconnecting Chrome does not fix.
    pub fn chrome_missing(&self) -> bool {
        self.chrome_loaded == Some(false)
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

    // A CLI proxy in front of Claude turning requests away: Claude Code retries, the rate limit comes back
    // rejected with no reset time, and the run ends on a 429 that names no usage limit.
    const COOLING: &str = r#"{"type":"assistant","message":{"model":"claude-sonnet-4-5","role":"assistant","content":[{"type":"text","text":"Checking the order totals."}],"usage":{"input_tokens":6,"cache_read_input_tokens":41000,"cache_creation_input_tokens":800,"output_tokens":40}},"parent_tool_use_id":null,"session_id":"sess-7"}
{"type":"system","subtype":"api_retry","attempt":1,"max_retries":10,"retry_delay_ms":600,"error_status":429,"error":"rate_limit","session_id":"sess-7"}
{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","isUsingOverage":false},"session_id":"sess-7"}
{"type":"assistant","message":{"model":"<synthetic>","role":"assistant","content":[{"type":"text","text":"API Error: Server is temporarily limiting requests. Every account in the pool is cooling down."}],"usage":{"input_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"output_tokens":0}},"parent_tool_use_id":null,"session_id":"sess-7","error":"rate_limit","api_error_status":429,"is_api_error_message":true}
{"type":"result","subtype":"success","is_error":true,"num_turns":12,"duration_ms":81000,"result":"API Error: Server is temporarily limiting requests. Every account in the pool is cooling down.","permission_denials":[],"api_error_status":429,"terminal_reason":"api_error","total_cost_usd":1.25,"model_usage_since_last_result":{"claude-sonnet-4-5":{"costUSD":1.25}},"result_index":0,"session_id":"sess-7"}"#;

    const OVERLOADED: &str = r#"{"type":"result","subtype":"success","is_error":true,"num_turns":3,"duration_ms":2100,"result":"API Error: Repeated 529 Overloaded errors","permission_denials":[],"api_error_status":529,"terminal_reason":"api_error","session_id":"sess-8"}"#;

    fn home() -> &'static Path {
        Path::new("/tmp/office/employees/tester")
    }

    fn parsed(transcript: &str) -> Vec<Parsed> {
        transcript.lines().map(|l| parse_line(l, home())).collect()
    }

    fn last_outcome(transcript: &str) -> Outcome {
        parsed(transcript).into_iter().filter_map(|p| p.outcome).next_back().expect("a result")
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
        Outcome { ok: true, text: text.into(), subtype: "success".into(), turns, duration_ms: 10, denials: vec![], limit: None, notice: false }
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
        assert_eq!(events[1], RunEvent::User { text: "Add the totals too.".into(), id: None });
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
        assert_eq!(o.limit, Some(Limit::Usage));
        assert!(!o.ok);
        assert_eq!(last_outcome(MULTI_RESULT).limit, None);
    }

    #[test]
    fn the_synthetic_line_keeps_the_context() {
        let last = parsed(LIMIT).into_iter().filter_map(|p| p.context).next_back();
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
        assert_eq!(retried.limit, None);
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
        let transcript = format!("{MULTI_RESULT}\n{}", omega_line(&RunEvent::User { text: "One more thing.".into(), id: None }));
        assert_eq!(steered(&transcript), vec!["Add the totals too.".to_string(), "One more thing.".to_string()]);
    }

    #[test]
    fn a_message_counts_as_heard_once_claude_code_echoes_it() {
        let echo = r#"{"type":"user","message":{"role":"user","content":"Skip the archive."},"uuid":"m2","isReplay":true,"parent_tool_use_id":null}"#;
        let p = parse_line(echo, Path::new("/office/employees/ada"));
        assert_eq!(p.replayed.as_deref(), Some("m2"));
        assert!(p.events.is_empty(), "an echo is not a step");

        let said = |id: &str, text: &str| omega_line(&RunEvent::User { text: text.into(), id: Some(id.into()) });
        let transcript = [
            omega_line(&RunEvent::User { text: "From an older build.".into(), id: None }),
            said("m1", "Use last week's sheet."),
            omega_line(&RunEvent::Heard { id: "m1".into() }),
            said("m2", "Skip the archive."),
        ]
        .join("\n");
        assert_eq!(unheard(&transcript), vec![("m2".to_string(), "Skip the archive.".to_string())]);
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

    /// How many times Chrome is told down over `lines`, each read `secs` seconds into the run.
    fn chrome_downs(lines: &[(i64, String)]) -> usize {
        let start = chrono::Utc::now();
        let mut s = RunSignals::default();
        lines
            .iter()
            .filter(|(secs, line)| {
                let p = parse_line(line, home());
                s.observe(&p);
                s.chrome_down(&p.events, start + chrono::Duration::seconds(*secs))
            })
            .count()
    }

    /// A Chrome call `id` of `tool` and what it returned, both read at `secs`.
    fn chrome_call(secs: i64, id: &str, tool: &str, result: &str) -> Vec<(i64, String)> {
        vec![(secs, call(id, &format!("mcp__claude-in-chrome__{tool}"), json!({}))), (secs, answer(id, false, result))]
    }

    fn not_connected(secs: i64, id: &str) -> Vec<(i64, String)> {
        chrome_call(secs, id, "tabs_context_mcp", "Browser extension is not connected. Please ensure the extension is installed and running.")
    }

    #[test]
    fn chrome_is_down_only_once_it_stays_down() {
        let first: Vec<(i64, String)> = CHROME.lines().map(|l| (0, l.to_string())).collect();
        assert_eq!(chrome_downs(&first), 0, "a first call often loses a race");
        let three = [not_connected(0, "c1"), not_connected(2, "c2"), not_connected(4, "c3"), not_connected(6, "c4")].concat();
        assert_eq!(chrome_downs(&three), 1, "three in a row, told once");
        let waited = [not_connected(0, "c1"), vec![(46, call("b1", "Bash", json!({ "command": "ls" })))]].concat();
        assert_eq!(chrome_downs(&waited), 1, "still failing 45 seconds on");
        let recovered = [not_connected(0, "c1"), not_connected(2, "c2"), chrome_call(3, "c3", "navigate", "Navigated."), not_connected(50, "c4"), not_connected(51, "c5")].concat();
        assert_eq!(chrome_downs(&recovered), 0, "a call that worked starts the count again");
        let closing: Vec<(i64, String)> = (0..4).flat_map(|i| chrome_call(i * 30, &format!("t{i}"), "tabs_close_mcp", CHROME_DOWN)).collect();
        assert_eq!(chrome_downs(&closing), 0, "a tab that will not close is not Chrome down");
        let quoted = vec![(0, call("toolu_b1", "Bash", json!({ "command": "grep -r notes" }))), (60, answer("toolu_b1", false, "notes.md: Browser extension is not connected"))];
        assert_eq!(chrome_downs(&quoted), 0);
        let slow = [not_connected(0, "c1"), not_connected(2, "c2"), chrome_call(3, "c3", "tabs_context_mcp", "The browser did not respond within 8 seconds."), not_connected(4, "c4")].concat();
        assert_eq!(chrome_downs(&slow), 0, "a slow browser is still connected");
    }

    #[test]
    fn chrome_tools_missing_from_the_init_line_are_noticed() {
        assert!(signals(INIT).chrome_missing());
        let with = INIT.replace(r#""tools":["Bash","Read"]"#, r#""tools":["Bash","Read","mcp__claude-in-chrome__navigate"]"#);
        assert!(!signals(&with).chrome_missing());
        assert!(!signals(&format!("{with}\n{INIT}")).chrome_missing(), "the run's first init line says");
        assert!(!signals(MULTI_RESULT).chrome_missing(), "no init line, nothing to say");
    }

    #[test]
    fn a_rate_limit_with_no_reset_time_is_transient() {
        let lines = parsed(COOLING);
        assert!(lines[2].throttled && lines[2].limited.is_none(), "rejected with no reset time");
        assert!(lines[3].throttled && lines[3].context.is_none(), "the error message Claude Code writes for it");
        let o = last_outcome(COOLING);
        assert_eq!((o.ok, o.limit), (false, Some(Limit::Transient)));
        assert_eq!(last_outcome(OVERLOADED).limit, Some(Limit::Transient), "an overload too");
        assert_eq!(last_outcome(LIMIT).limit, Some(Limit::Usage), "the usage limit stays the usage limit");
        let worded = r#"{"type":"result","subtype":"success","is_error":true,"num_turns":1,"duration_ms":60,"result":"API Error: every account is cooling down","permission_denials":[]}"#;
        assert_eq!(last_outcome(worded).limit, Some(Limit::Transient), "a proxy's words, from a build that sends no status");
        let fine = r#"{"type":"result","subtype":"success","is_error":false,"num_turns":4,"duration_ms":900,"result":"The oven is cooling down, so the batch is done.","permission_denials":[]}"#;
        assert_eq!(last_outcome(fine).limit, None, "a run that worked hit no limit");
        // An older build's session limit says no more than a 429, but its rate limit names the reset.
        let older = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1800000000,"rateLimitType":"five_hour"},"session_id":"sess-9"}"#;
        let p = parse_line(older, home());
        assert!(p.limited.is_some() && !p.throttled);
    }

    #[test]
    fn a_turned_away_result_says_how_long_until_it_eases() {
        let proxy = |wait: &str| format!("API Error: Server is temporarily limiting requests (not your usage limit) · All credentials for model example-model are cooling down via provider claude (last error: rate_limit_error: This request would exceed your account's rate limit. Please try again later.) (reset after {wait})");
        let secs = |s: f64| Some(chrono::Duration::milliseconds((s * 1000.0) as i64));
        for (wait, expected) in [
            ("12m30s", secs(750.0)),
            ("45s", secs(45.0)),
            ("1h2m3s", secs(3723.0)),
            ("2h", secs(7200.0)),
            ("3m0s", secs(180.0)),
            ("1m27.5s", secs(87.5)),
            ("30s.", secs(30.0)),
            ("0s", None),
            ("0.0004s", None),
            ("500ms", None),
            ("12", None),
            ("soon", None),
            ("", None),
        ] {
            assert_eq!(eases_in(&proxy(wait)), expected, "{wait:?}");
        }
        assert_eq!(eases_in("API Error: Server is temporarily limiting requests."), None, "it states no wait");
    }

    #[test]
    fn requests_turned_away_just_before_a_failure_are_a_rate_limit() {
        let rejected = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","isUsingOverage":false},"session_id":"sess-7"}"#;
        let failed = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"num_turns":4,"duration_ms":900,"result":"","permission_denials":[]}"#;
        let answered = MULTI_RESULT.lines().next().unwrap();
        let limit_after = |lines: &[&str]| {
            let mut s = RunSignals::default();
            let mut limit = None;
            for p in parsed(&lines.join("\n")) {
                s.observe(&p);
                if let Some(o) = &p.outcome {
                    limit = s.limit(o);
                }
            }
            limit
        };
        assert_eq!(limit_after(&[rejected, failed]), Some(Limit::Transient));
        assert_eq!(limit_after(&[rejected, answered, failed]), None, "Claude answered after it");
        assert_eq!(limit_after(&[rejected, MULTI_RESULT.lines().nth(1).unwrap()]), None, "the run worked");
        assert_eq!(limit_after(&[rejected, failed, failed]), None, "it counts once");
    }

    #[test]
    fn a_notice_answered_after_the_turn_keeps_the_report_you_were_given() {
        let report = "Mostly yes, and here is why.\nThe totals match the bank.";
        let notice = Outcome { notice: true, ..outcome("That was the background check finishing.\nNothing else changed.", 1) };
        let m = merge_outcome(outcome(report, 3), notice);
        assert_eq!(m.text, format!("{report}\n\nThat was the background check finishing.\nNothing else changed."));
        assert_eq!(m.turns, 4);
        let long = "y".repeat(250);
        assert_eq!(merge_outcome(outcome(report, 3), outcome(&long, 1)).text, long, "a message you steered in gets its own answer");
        assert_eq!(merge_outcome(outcome("", 1), Outcome { notice: true, ..outcome("Only the notice.", 1) }).text, "Only the notice.");

        // A limit that stops the notice's answer after the report was written keeps the report, and requeues nothing.
        for limit in [Limit::Transient, Limit::Usage] {
            let failed = Outcome { ok: false, notice: true, limit: Some(limit), subtype: "error_during_execution".into(), ..outcome("API Error: 429", 1) };
            let m = merge_outcome(outcome(report, 3), failed);
            assert_eq!((m.ok, m.limit, m.text.as_str(), m.turns), (true, None, report, 4), "{limit:?}");
        }
        let steered = Outcome { ok: false, limit: Some(Limit::Transient), ..outcome("API Error: 429", 1) };
        assert_eq!(merge_outcome(outcome(report, 3), steered).limit, Some(Limit::Transient), "a message you sent in still needs its answer");

        let noticed = r#"{"type":"result","subtype":"success","is_error":false,"num_turns":1,"duration_ms":700,"result":"The sub-agent finished; the report above stands.\nNo changes.","permission_denials":[],"origin":{"kind":"task-notification","producer":"session-task","runId":"bg-1"},"result_index":1,"session_id":"sess-1"}"#;
        let first = MULTI_RESULT.lines().take(2).collect::<Vec<_>>().join("\n");
        let o = transcript_outcome(&format!("{first}\n{INIT}\n{noticed}"), home()).expect("an outcome");
        assert_eq!(o.text, "Report line one.\nReport line two.\n\nThe sub-agent finished; the report above stands.\nNo changes.");
    }

    #[test]
    fn spend_is_read_from_result_lines() {
        let line = |extra: &str| format!(r#"{{"type":"result","subtype":"success","is_error":false,"num_turns":1,"duration_ms":10,"result":"Done.","permission_denials":[],"total_cost_usd":4.5{extra}}}"#);
        let both = line(r#","model_usage_since_last_result":{"claude-sonnet-4-5":{"costUSD":0.5},"claude-haiku-4-5":{"costUSD":0.25}}"#);
        assert_eq!(parse_line(&both, home()).spend, Some(Spend { own: Some(0.75), total: Some(4.5) }));
        assert_eq!(parse_line(&line(""), home()).spend, Some(Spend { own: None, total: Some(4.5) }), "an older build reports only the total");
        assert_eq!(parse_line(&line(r#","model_usage_since_last_result":{}"#), home()).spend, Some(Spend { own: Some(0.0), total: Some(4.5) }), "nothing since the last result");
        let nested = line(r#","parent_tool_use_id":"toolu_sub""#);
        assert_eq!(parse_line(&nested, home()).spend, None);
        assert_eq!(last_total(&format!("{}\n{}\n{nested}", line(""), line(r#","model_usage_since_last_result":{}"#).replace("4.5", "6.25"))), Some(6.25));
        assert_eq!(last_total(MULTI_RESULT), None);
    }

    #[test]
    fn tokens_are_read_model_by_model_from_result_lines_without_cache_rereads() {
        let line = |extra: &str| format!(r#"{{"type":"result","subtype":"success","is_error":false,"num_turns":1,"duration_ms":10,"result":"Done.","permission_denials":[]{extra}}}"#);
        let used = line(r#","model_usage_since_last_result":{"claude-opus-4-5":{"inputTokens":10,"cacheCreationInputTokens":200,"cacheReadInputTokens":90000,"outputTokens":30,"costUSD":0.5},"claude-haiku-4-5":{"inputTokens":0,"outputTokens":0,"costUSD":0}}"#);
        assert_eq!(parse_line(&used, home()).tokens, vec![ModelTokens { model: "claude-opus-4-5".into(), tokens: 240 }], "a model that used nothing is left out");
        assert_eq!(parse_line(&line(""), home()).tokens, vec![], "an older build reports none");
        let nested = used.replace(r#""permission_denials":[]"#, r#""permission_denials":[],"parent_tool_use_id":"toolu_sub""#);
        assert_eq!(parse_line(&nested, home()).tokens, vec![], "a sub-agent's result is already in its parent's");
    }

    #[test]
    fn a_runs_cost_is_its_own_segments_never_the_running_total() {
        let spend = |own: Option<f64>, total: f64| Spend { own, total: Some(total) };
        // Each segment's own cost, however much the session cost before.
        assert_eq!(run_cost(&[spend(Some(1.5), 88.0), spend(Some(0.25), 88.25)], None), Some(1.75));
        // Without it, how far each took the running total.
        assert_eq!(run_cost(&[spend(None, 12.0), spend(None, 12.5)], Some(10.0)), Some(2.5));
        assert_eq!(run_cost(&[spend(None, 3.0)], Some(0.0)), Some(3.0), "a new session starts at nothing");
        assert_eq!(run_cost(&[spend(None, 12.0), spend(None, 12.5)], None), Some(0.5), "an unknown start never counts the whole session");
        assert_eq!(run_cost(&[spend(None, 12.0)], None), None);
        assert_eq!(run_cost(&[spend(Some(0.5), 12.0), spend(None, 12.75)], None), Some(1.25), "the two kinds mix");
        assert_eq!(run_cost(&[], Some(0.0)), None);
    }

    #[test]
    fn images_are_left_out_of_saved_lines() {
        let data = "iVBORw0KGgo".repeat(400);
        let line = json!({
            "type": "user", "parent_tool_use_id": null,
            "message": { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "toolu_img1", "content": [
                { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": data } },
                { "type": "text", "text": "Saved the chart." },
            ] }] },
            "tool_use_result": { "type": "image", "file": { "base64": data, "type": "image/png", "originalSize": 2048 } },
        })
        .to_string();
        let out = without_images(&line);
        assert!(out.len() < 500, "{out}");
        let v: Value = serde_json::from_str(&out).expect("still JSON");
        assert_eq!(v["message"]["content"][0]["content"][0]["source"]["data"], "[image/png, 4 KB, left out]");
        assert_eq!(v["tool_use_result"]["file"]["base64"], "[image/png, 4 KB, left out]");
        assert_eq!(replay(&out, home()), replay(&line, home()), "the app reads the same");
        assert!(matches!(without_images(&out), Cow::Borrowed(_)), "a stripped line stays as it is");
        assert!(matches!(without_images(INIT), Cow::Borrowed(_)));
        let mentions = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"The logo is inlined as base64 in the CSS."}]},"parent_tool_use_id":null}"#;
        assert!(matches!(without_images(mentions), Cow::Borrowed(_)));
    }

    #[test]
    fn helpers_stopped_at_the_end_are_named() {
        let tail = "; stopping them. Set CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS=0 to wait indefinitely.";
        let one = format!("Background tasks still running 10m after the last turn (subagent \"Check the totals\"){tail}");
        assert_eq!(stopped_helpers(&one).as_deref(), Some("Claude Code stopped a helper that had not finished when the run ended: \"Check the totals\"."));
        let two = format!("warning: a slow disk\nBackground tasks still running 10m after the last turn (subagent \"Draft the memo (short)\", shell \"npm test\"){tail}\n");
        assert_eq!(
            stopped_helpers(&two).as_deref(),
            Some("Claude Code stopped 2 helpers that had not finished when the run ended: \"Draft the memo (short)\", \"npm test\".")
        );
        let older = "Background tasks still running after 600s; terminating. Set CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS=0 to wait indefinitely.";
        assert_eq!(stopped_helpers(older).as_deref(), Some("Claude Code stopped helpers that had not finished when the run ended."));
        assert_eq!(stopped_helpers("warning: a slow disk"), None);
    }
}
