//! Threads: a report and the replies you exchanged on it. A thread is a root run plus every Reply run
//! whose `to_run_id` is that root. These are pure functions over runs; the Office loads the runs and
//! the Inbox, the Archive and each employee's chips show the same threads. A task is a thread too: its root
//! is your first message rather than a report, and it is never filed into an Inbox row.
//!
//! Inbox rows group threads that belong together, so you see one row per piece of work rather than
//! one per run: a report with everything it set off in conversation, or an ask a report made with
//! everything that came back from it (`file`), and a schedule's failures in a row (`merge_streaks`).

use crate::claude::{clip, one_line};
use crate::model::*;
use std::collections::{HashMap, HashSet};

/// Turns a new task is told about from the conversation it started in.
const CONTEXT_TURNS: usize = 8;
/// A task's name, cut from your first message.
const TASK_LABEL_CHARS: usize = 48;

/// Replies grouped under their root's id, oldest first.
pub fn group(replies: Vec<Run>) -> HashMap<String, Vec<Run>> {
    let mut out: HashMap<String, Vec<Run>> = HashMap::new();
    for r in replies {
        let root = r.reply_to().unwrap_or(&r.id).to_string();
        out.entry(root).or_default().push(r);
    }
    for v in out.values_mut() {
        v.sort_by_key(|r| r.created_at);
    }
    out
}

/// Each root with its replies, in the roots' order. Replies to roots not in `roots` are dropped.
fn assemble(roots: Vec<Run>, replies: Vec<Run>) -> Vec<Thread> {
    let mut grouped = group(replies);
    roots
        .into_iter()
        .map(|root| {
            let replies = grouped.remove(&root.id).unwrap_or_default();
            Thread::new(root, replies)
        })
        .collect()
}

/// When a thread last moved: its newest reply (sent or finished), else the report itself.
pub fn activity_at(root: &Run, replies: &[Run]) -> Ts {
    replies
        .iter()
        .map(|r| r.ended_at.unwrap_or(r.created_at))
        .chain(std::iter::once(root.ended_at.unwrap_or(root.created_at)))
        .max()
        .unwrap_or(root.created_at)
}

/// Runs that continue a conversation rather than start one: your replies, colleagues' asks and their
/// answers, and delegated work. Every other run is a report.
pub fn continues(kind: &RunKind) -> bool {
    matches!(kind, RunKind::Reply { .. } | RunKind::Answered { .. } | RunKind::Asked { .. } | RunKind::Delegated { .. })
}

/// Reports that stay out of the Inbox unless they flag something or break: quiet schedules, tidies
/// and nudges.
pub fn quiet(kind: &RunKind) -> bool {
    matches!(kind, RunKind::Scheduled { quiet: true, .. } | RunKind::Tidy { .. } | RunKind::Nudge { .. })
}

/// The run called notify, or was blocked on a tool (a tidy trying a command it may not run is not).
pub fn flagged(r: &Run) -> bool {
    !r.needs_you.is_empty() || (!r.denials.is_empty() && !matches!(r.kind, RunKind::Tidy { .. }))
}

pub fn broke(r: &Run) -> bool {
    matches!(r.state, RunState::Failed | RunState::Lost)
}

/// Whether a finished run reaches your Inbox. `quiet_chain`: it is part of a hand-off a quiet report
/// started, so like that report it speaks up only when it flags something or breaks. `top`: its
/// employee reports to you; an agent below another agent reports to that manager unless it flags you.
/// A task never does: you follow it as a conversation of its own.
pub fn reaches_you(r: &Run, quiet_chain: bool, top: bool) -> bool {
    if r.kind.lane() != Lane::Work || matches!(r.kind, RunKind::Reply { .. } | RunKind::ReplyCheck { .. } | RunKind::Task { .. }) || !r.state.is_finished() {
        return false;
    }
    if matches!(r.kind, RunKind::Nudge { .. }) && r.state == RunState::Cancelled {
        return false;
    }
    if flagged(r) {
        return true;
    }
    if (quiet(&r.kind) || quiet_chain) && r.state == RunState::Succeeded {
        return false;
    }
    match r.kind {
        // Work handed to a colleague reports to whoever asked, unless it breaks on the way.
        RunKind::Asked { .. } | RunKind::Delegated { .. } => broke(r),
        _ => top,
    }
}

/// The run a conversation run continues: the run a reply answers, the ask an answer brings back, and
/// the run an ask or delegation came from when that run was itself part of a conversation. An ask a
/// report makes starts a row of its own, so one report's unrelated hand-offs don't share a row.
fn continues_from<'a>(r: &'a Run, conversation: &HashMap<String, Run>) -> Option<&'a str> {
    match &r.kind {
        RunKind::Reply { to_run_id, .. } => Some(to_run_id),
        RunKind::Answered { asked_run_id, .. } => Some(asked_run_id),
        RunKind::Asked { parent_run_id, .. } | RunKind::Delegated { parent_run_id } => {
            conversation.contains_key(parent_run_id.as_str()).then_some(parent_run_id.as_str())
        }
        _ => None,
    }
}

/// The first run of the row `r` belongs to: a report, or an ask a report made. Follows
/// `continues_from` through `conversation` (every conversation run by id); a link to a run that is not
/// in it is the report the conversation started from.
pub fn anchor<'a>(r: &'a Run, conversation: &'a HashMap<String, Run>) -> &'a str {
    let mut cur = r;
    for _ in 0..64 {
        match continues_from(cur, conversation) {
            Some(next) => match conversation.get(next) {
                Some(n) => cur = n,
                None => return next,
            },
            None => break,
        }
    }
    &cur.id
}

/// One Inbox row before it is shown: every thread filed under it, oldest first, each with whether it
/// reaches you on its own.
#[derive(Debug, Clone)]
pub struct Row {
    pub members: Vec<(Thread, bool)>,
}

/// Files runs into rows. A row is a report with everything it set off in conversation (your replies,
/// colleagues' asks made inside that conversation, and their answers), or an ask a report made with
/// everything that came back from it. `starts` are the runs rows are wanted for; `conversation` is every
/// conversation run; `lookup` loads any other run; `top` holds the employees who report to you. Rows
/// come in the order their first run appears in `starts`.
pub fn file(starts: &[Run], conversation: Vec<Run>, lookup: impl Fn(&str) -> Option<Run>, top: &HashSet<String>) -> Vec<Row> {
    let conversation: HashMap<String, Run> = conversation.into_iter().map(|r| (r.id.clone(), r)).collect();
    let mut by_anchor: HashMap<String, Vec<&Run>> = HashMap::new();
    for r in conversation.values() {
        by_anchor.entry(anchor(r, &conversation).to_string()).or_default().push(r);
    }
    let mut seen = HashSet::new();
    let mut rows = vec![];
    for s in starts {
        let a = if continues(&s.kind) { anchor(conversation.get(&s.id).unwrap_or(s), &conversation).to_string() } else { s.id.clone() };
        if !seen.insert(a.clone()) {
            continue;
        }
        let first = conversation.get(&a).cloned().or_else(|| (s.id == a).then(|| s.clone())).or_else(|| lookup(&a));
        // Where the hand-off started: the report itself, or the run that made the first ask.
        let origin = match first.as_ref().map(|f| &f.kind) {
            Some(RunKind::Asked { parent_run_id, .. } | RunKind::Delegated { parent_run_id }) => lookup(parent_run_id),
            _ => first.clone(),
        };
        let quiet_chain = origin.is_some_and(|o| quiet(&o.kind));
        let mut runs: Vec<Run> = by_anchor.get(&a).map(|v| v.iter().map(|r| (*r).clone()).collect()).unwrap_or_default();
        if let Some(f) = first.filter(|f| !conversation.contains_key(&f.id)) {
            runs.push(f);
        }
        if runs.is_empty() {
            runs.push(s.clone());
        }
        let (replies, mut roots): (Vec<Run>, Vec<Run>) = runs.into_iter().partition(|r| matches!(r.kind, RunKind::Reply { .. }));
        roots.sort_by(|x, y| x.created_at.cmp(&y.created_at).then_with(|| x.id.cmp(&y.id)));
        let members = assemble(roots, replies)
            .into_iter()
            .map(|t| {
                let reach = reaches_you(&t.root, quiet_chain, top.contains(&t.root.employee_id));
                (t, reach)
            })
            .collect();
        rows.push(Row { members });
    }
    rows
}

/// Merges rows that are one schedule's failures in a row. `streak` names the streak a row's first run
/// belongs to (a scheduled run that failed), or None; rows naming the same streak become one, in the
/// place of the first.
pub fn merge_streaks(rows: Vec<Row>, streak: impl Fn(&Run) -> Option<String>) -> Vec<Row> {
    let mut out: Vec<Row> = vec![];
    let mut at: HashMap<String, usize> = HashMap::new();
    for row in rows {
        let key = row.members.first().and_then(|(t, _)| streak(&t.root));
        match key.as_ref().and_then(|k| at.get(k)) {
            Some(&i) => {
                out[i].members.extend(row.members);
                out[i].members.sort_by_key(|(t, _)| t.root.created_at);
            }
            None => {
                if let Some(k) = key {
                    at.insert(k, out.len());
                }
                out.push(row);
            }
        }
    }
    out
}

/// A row as you see it: the thread it is about, with its other threads in `related`, oldest first.
/// In the Inbox that is the newest open thread (it reaches you and is not archived) that flags you or
/// broke, else the newest open one; None when nothing in the row is open. In the Archive (`archive`)
/// it is the newest thread that reached you; None while the row is open, or if it never reached you.
pub fn present(row: Row, archive: bool) -> Option<Thread> {
    let open = |m: &(Thread, bool)| m.1 && m.0.root.archived_at.is_none();
    if row.members.iter().any(open) == archive {
        return None;
    }
    let newest = |pick: &dyn Fn(&(Thread, bool)) -> bool| {
        row.members
            .iter()
            .enumerate()
            .filter(|(_, m)| pick(m))
            .max_by_key(|(_, (t, _))| activity_at(&t.root, &t.replies))
            .map(|(i, _)| i)
    };
    let lead = if archive {
        newest(&|m| m.1)?
    } else {
        newest(&|m| open(m) && (flagged(&m.0.root) || broke(&m.0.root))).or_else(|| newest(&open))?
    };
    let mut members: Vec<Thread> = row.members.into_iter().map(|(t, _)| t).collect();
    let mut lead = members.remove(lead);
    lead.related = members;
    Some(lead)
}

/// When a row was archived: the latest of its threads.
pub fn row_archived_at(t: &Thread) -> Option<Ts> {
    std::iter::once(t).chain(&t.related).filter_map(|x| x.root.archived_at).max()
}

/// What a report was, as a thread is named: a schedule's label, "Manual run", a task's name, or the kind.
pub fn report_label(kind: &RunKind) -> String {
    match kind {
        RunKind::Scheduled { label, .. } | RunKind::Reply { label, .. } | RunKind::Task { label, .. } => label.clone(),
        RunKind::Manual => "Manual run".into(),
        k => crate::claude::kind_label(k),
    }
}

/// One turn of a conversation as a task's context quotes it: what you said, then the
/// employee's answer, or `pending` while it is still coming. A message Omega Star wrote is never
/// quoted as yours.
fn exchange(ceo: &str, r: &Run, pending: &str) -> String {
    let mut text = String::new();
    if !r.prompt.trim().is_empty() {
        let who = match &r.kind {
            RunKind::Answered { .. } => "A colleague's report came back".to_string(),
            _ if r.omega_wrote => format!("Omega Star, from what {ceo} did in the app"),
            _ => ceo.to_string(),
        };
        text += &format!("- {who}: {}\n", clip(&one_line(&r.prompt), 300));
    }
    match r.summary.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(s) => text += &format!("- You: {}\n", clip(&one_line(s), 400)),
        None if r.state.is_finished() => text += &format!("- You: (your reply did not finish: {})\n", r.error.as_deref().unwrap_or("no report")),
        None => text += &format!("- You: ({pending})\n"),
    }
    text
}

/// The first line of a report with markdown marks stripped, so an excerpt reads as plain text.
fn first_line(s: &str) -> String {
    let marks: &[char] = &['#', '>', '-', '*', ' '];
    let emphasis: &[char] = &['*', '_', '`'];
    s.lines()
        .map(|l| l.trim().trim_start_matches(marks).replace(emphasis, ""))
        .find(|l| !l.is_empty())
        .unwrap_or_default()
}

/// What a task you start mid-conversation is told about that conversation: its newest turns, oldest
/// first, each clipped. `runs` is the conversation in order; a report opening it is quoted as one.
pub fn context(ceo: &str, runs: &[Run]) -> String {
    let skipped = runs.len().saturating_sub(CONTEXT_TURNS);
    let mut text = String::new();
    if skipped > 0 {
        text += &format!("- ({skipped} earlier {} omitted)\n", if skipped == 1 { "turn" } else { "turns" });
    }
    for r in runs.iter().skip(skipped) {
        if r.kind.is_conversation() {
            text += &exchange(ceo, r, "still working on this in that conversation");
        } else if let Some(s) = r.summary.as_deref().filter(|s| !s.trim().is_empty()) {
            text += &format!("- Your report: {}\n", clip(&one_line(s), 400));
        }
    }
    text
}

/// A task's name: the first sentence of your first message without the words that open a request, so
/// "Also, can you draft the Acme reply? Keep it short." is named "Draft the Acme reply". `files` name a
/// message that is only attachments.
pub fn task_label(text: &str, files: &[String]) -> String {
    const OPENERS: [&str; 9] = ["also", "and", "hey", "ok", "please", "can you", "could you", "would you", "i need you to"];
    const ENDS: [char; 6] = ['?', '.', '!', ',', ':', ';'];
    let line = first_line(text);
    let mut s = line.as_str();
    'strip: loop {
        s = s.trim_start_matches([',', ':', ' ']);
        for o in OPENERS {
            let at_boundary = s.get(o.len()..).is_some_and(|rest| rest.is_empty() || rest.starts_with([',', ':', ' ']));
            if at_boundary && s[..o.len()].eq_ignore_ascii_case(o) {
                s = &s[o.len()..];
                continue 'strip;
            }
        }
        break;
    }
    let sentence = ["? ", ". ", "! "].iter().filter_map(|end| s.find(end)).min().map_or(s, |i| &s[..i]);
    let s = match sentence.trim_end_matches(ENDS).trim() {
        "" => line.trim(),
        s => s,
    };
    if s.is_empty() {
        return if files.is_empty() { "New task".into() } else { clip(&files.join(", "), TASK_LABEL_CHARS) };
    }
    let mut label: String = s.chars().take(TASK_LABEL_CHARS).collect();
    if s.chars().count() > TASK_LABEL_CHARS {
        // Cut at a word, so the name never ends mid-word.
        if let Some(i) = label.rfind(' ') {
            label.truncate(i);
        }
        label = format!("{}…", label.trim_end_matches(ENDS));
    }
    let mut chars = label.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_never_quotes_omega_stars_words_as_yours() {
        let at = chrono::Utc::now();
        let mut turn = Run::new("e1", RunKind::Chat, "Count the stock.".into(), at);
        turn.summary = Some("On it.".into());
        let came = RunKind::Answered { asked_run_id: "r1".into(), from: "e2".into(), task: false, chat: true };
        let mut answered = Run::new("e1", came, "Bo answered your question. Their answer: 40 boxes.".into(), at);
        answered.omega_wrote = true;
        answered.summary = Some("Bo counted 40 boxes.".into());
        let reply = RunKind::Reply { to_run_id: "r2".into(), label: "Manual run".into(), task: false };
        let mut tap = Run::new("e1", reply, "Pat approved what was blocked.".into(), at);
        tap.omega_wrote = true;
        assert_eq!(
            context("Pat", &[turn, answered, tap]),
            "- Pat: Count the stock.\n- You: On it.\n\
- A colleague's report came back: Bo answered your question. Their answer: 40 boxes.\n- You: Bo counted 40 boxes.\n\
- Omega Star, from what Pat did in the app: Pat approved what was blocked.\n- You: (still working on this in that conversation)\n"
        );
    }
}
