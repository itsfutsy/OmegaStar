//! Threads: a report and the replies you exchanged on it. A thread is a root run plus every later turn
//! whose `Run::reply_to` is that root. These are pure functions over runs; the Office loads the runs and
//! the Inbox, the Archive, the sidebar and the conversation screen show the same threads. A conversation
//! you started is a thread too (a task under the hood): its root is your first message rather than a
//! report, an answer a colleague brings back into it is one of its turns, and it is never filed into an
//! Inbox row.
//!
//! Rows group threads that belong together, so you see one row per piece of work rather than one per run:
//! a report with everything it set off in conversation, or an ask a report made with everything that came
//! back from it (`file`), and a schedule's failures in a row or everyone waiting on the same chore
//! (`merge_rows`). A row is in the Inbox only while it needs you (`needs_you`): a card, a flag, a blocked
//! tool, breakage you can act on, or the report of a run you wrote into. Every other row that reached you is in
//! the Archive, which never asks anything, and so is what you marked done.

use crate::claude::clip;
use crate::model::*;
use std::collections::{HashMap, HashSet};

/// A task's name, cut from your first message.
const TASK_LABEL_CHARS: usize = 48;

/// Later turns grouped under their root's id, oldest first.
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

/// How long a due check's run may work and still be a quick look. Longer, it did real work.
pub const CHECK_QUIET_MS: u64 = 30 * 60 * 1000;

/// Reports that stay out of the Inbox unless they flag something or break: quiet schedules, tidies
/// and nudges. A due check's run is quiet too, unless it ran longer than `CHECK_QUIET_MS`: then it reports
/// like other work.
pub fn quiet(r: &Run) -> bool {
    let long_check = r.kind.check_id().is_some() && r.duration_ms.is_some_and(|ms| ms > CHECK_QUIET_MS);
    matches!(r.kind, RunKind::Scheduled { quiet: true, .. } | RunKind::Tidy { .. } | RunKind::Nudge { .. }) && !long_check
}

/// The run raised a card or a flag, or was blocked on a tool (a tidy trying a command it may not run is not).
fn flagged(r: &Run) -> bool {
    !r.needs_you.is_empty() || blocked(r)
}

/// The run was blocked on a tool you could allow (a tidy trying a command it may not run is not).
fn blocked(r: &Run) -> bool {
    !r.denials.is_empty() && !matches!(r.kind, RunKind::Tidy { .. })
}

pub fn broke(r: &Run) -> bool {
    matches!(r.state, RunState::Failed | RunState::Lost)
}

/// A schedule's failures in a row before they need you: a routine that breaks once or twice usually works
/// the next time, and you see it in the Archive meanwhile.
pub const STREAK: usize = 3;

/// A finished run that might need you, before its thread is read: it raised something, was blocked, broke,
/// you replied to it (a reply can raise something), or you wrote into it. Every other report goes straight to the
/// Archive. The Inbox's query, `Store::finished_root_runs`, does this in SQL; its test holds the two equal.
#[cfg(test)]
pub fn could_need_you(r: &Run) -> bool {
    flagged(r) || broke(r) || r.replied_at.is_some() || r.steered
}

/// The newest run in a thread that has finished: the root, or the last reply that ended.
fn latest_finished(t: &Thread) -> &Run {
    t.replies.iter().rev().find(|r| r.ended_at.is_some()).unwrap_or(&t.root)
}

/// The thread asks you something: a card you have not answered or news you have not seen anywhere in it, or its
/// newest finished run flagged something or was blocked on a tool. A flag on an earlier run was answered by what came
/// after it, and a block is no longer yours once a later turn (the retry you allowed) is on its way.
pub fn asks_you(t: &Thread) -> bool {
    let last = latest_finished(t);
    let retrying = t.replies.last().is_some_and(|r| r.ended_at.is_none());
    std::iter::once(&t.root).chain(&t.replies).any(|r| r.has_open_card() || r.has_news()) || last.flags().next().is_some() || (blocked(last) && !retrying)
}

/// The thread's newest finished run broke for a reason you can act on: not the usage limit, which Omega
/// Star waits out and retries itself.
pub fn broken(t: &Thread) -> bool {
    let last = latest_finished(t);
    broke(last) && !last.limited
}

/// Whether a finished run reaches your Inbox. `quiet_chain`: it is part of a hand-off a quiet report
/// started, so like that report it speaks up only when it flags something or breaks. `top`: its
/// employee reports to you; an agent below another agent reports to that manager unless it flags you.
/// A run you wrote into (`Run::steered`) always does.
/// A conversation you started never does, nor an answer that came back into one: you follow it as a
/// conversation of its own.
pub fn reaches_you(r: &Run, quiet_chain: bool, top: bool) -> bool {
    let turn = matches!(r.kind, RunKind::Reply { .. } | RunKind::ReplyCheck { .. } | RunKind::Task { .. } | RunKind::Answered { thread: Some(_), .. });
    if r.kind.lane() != Lane::Work || turn || !r.state.is_finished() {
        return false;
    }
    if matches!(r.kind, RunKind::Nudge { .. }) && r.state == RunState::Cancelled {
        return false;
    }
    if flagged(r) || r.steered {
        return true;
    }
    if (quiet(r) || quiet_chain) && r.state == RunState::Succeeded {
        return false;
    }
    match r.kind {
        // Work handed to a colleague reports to whoever asked, unless it breaks on the way.
        RunKind::Asked { .. } | RunKind::Delegated { .. } => broke(r),
        // A weekly review reaches you whoever the employee reports to.
        RunKind::Scheduled { review: true, .. } => true,
        _ => top,
    }
}

/// The run a conversation run continues: the run a reply answers, the ask an answer brings back, and
/// the run an ask or delegation came from when that run was itself part of a conversation. An ask a
/// report makes starts a row of its own, so one report's unrelated hand-offs don't share a row, and so does
/// one made in a conversation you started, which is never filed into a row.
fn continues_from<'a>(r: &'a Run, conversation: &HashMap<String, Run>) -> Option<&'a str> {
    match &r.kind {
        RunKind::Reply { to_run_id, .. } => Some(to_run_id),
        RunKind::Answered { asked_run_id, .. } => Some(asked_run_id),
        RunKind::Asked { parent_run_id, .. } | RunKind::Delegated { parent_run_id } => conversation
            .get(parent_run_id.as_str())
            .filter(|p| !matches!(p.kind, RunKind::Reply { task: true, .. }))
            .map(|_| parent_run_id.as_str()),
        _ => None,
    }
}

/// The first run of the row `r` belongs to: a report, or an ask a report made. Follows
/// `continues_from` through `conversation` (every conversation run by id); a link to a run that is not
/// in it is the report the conversation started from.
fn anchor<'a>(r: &'a Run, conversation: &'a HashMap<String, Run>) -> &'a str {
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
        let quiet_chain = origin.is_some_and(|o| quiet(&o));
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

/// Merges rows that belong together: one schedule's failures in a row, or everyone waiting on the same
/// chore. `key` names what a row belongs to, or None; rows with the same key become one, in the place of
/// the first.
pub fn merge_rows(rows: Vec<Row>, key: impl Fn(&Row) -> Option<String>) -> Vec<Row> {
    let mut out: Vec<Row> = vec![];
    let mut at: HashMap<String, usize> = HashMap::new();
    for row in rows {
        let key = key(&row);
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

/// The run in a thread with a card waiting on your answer, newest first.
pub fn open_card(t: &Thread) -> Option<&Run> {
    t.replies.iter().rev().chain(std::iter::once(&t.root)).find(|r| r.has_open_card())
}

/// Any of these threads has a card waiting on your answer.
pub fn card_open(threads: &[Thread]) -> bool {
    threads.iter().any(|t| open_card(t).is_some())
}

/// `chore:<subject>` for a row with a chore waiting on you, so everyone waiting on the same chore shares a row.
pub fn chore_key(row: &Row) -> Option<String> {
    row.members
        .iter()
        .flat_map(|(t, _)| std::iter::once(&t.root).chain(&t.replies))
        .flat_map(|r| r.open_cards())
        .find(|c| matches!(c.ask, CardAsk::Chore { .. }))
        .and_then(Card::topic)
}

/// `review@<year>-W<week>` for a weekly review, from its slot's local ISO week, so one week's reviews
/// across the team share a row.
pub fn review_week(r: &Run) -> Option<String> {
    let RunKind::Scheduled { slot, review: true, .. } = &r.kind else { return None };
    let iso = chrono::Datelike::iso_week(&slot.with_timezone(&chrono::Local));
    Some(format!("review@{}-W{:02}", iso.year(), iso.week()))
}

/// Where a row is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// What needs you, until it no longer does.
    Inbox,
    /// Every row that reached you and needs nothing from you now, one line each, and the rows you archived.
    Archive,
}

/// A member thread that reaches you and you have not archived.
fn is_open(m: &(Thread, bool)) -> bool {
    m.1 && m.0.root.archived_at.is_none()
}

/// The row needs you: an open thread in it asks you something, broke where breakage needs you, or is a run you
/// wrote into (`Run::steered`), whose report waits for you like a reply until you mark it done. A schedule's
/// failures need you once `STREAK` of them are in a row; anything else that breaks, at once.
fn needs_you(row: &Row) -> bool {
    let open = || row.members.iter().filter(|m| is_open(m)).map(|(t, _)| t);
    if open().any(|t| asks_you(t) || t.root.steered) {
        return true;
    }
    let scheduled = |t: &Thread| matches!(t.root.kind, RunKind::Scheduled { .. });
    let failures = row.members.iter().filter(|(t, _)| scheduled(t) && broke(&t.root) && !t.root.limited).count();
    open().any(|t| broken(t) && (!scheduled(t) || failures >= STREAK))
}

/// A conversation still going on: a thread with later turns that you have not archived. The sidebar and the
/// conversation screen show it until you mark it done. A reply typed into a report while it worked is no later turn.
fn going_on(t: &Thread) -> bool {
    t.root.archived_at.is_none() && !t.replies.is_empty()
}

/// A conversation you are in: one going on (`going_on`) that you started (an older build's chat message an
/// answer came back into too) or typed into. Turns Omega Star wrote from a tap (a card's answer, an approval)
/// alone do not make one, nor do questions you asked about a card (`Run::about_card`).
pub fn talking(t: &Thread) -> bool {
    going_on(t) && (matches!(t.root.kind, RunKind::Task { .. } | RunKind::Chat) || t.replies.iter().any(|r| !r.omega_wrote && !r.about_card))
}

/// A row as `view` shows it: the thread it is about, with its other threads in `related`, oldest first.
/// In the Inbox that is the newest open thread that asks you something or broke, else the newest open one;
/// None unless the row needs you. In the Archive it is the newest thread that reached you, leaving out any
/// conversation still going on (`going_on`); None while the row needs you, or if nothing else in it
/// reached you.
pub fn present(mut row: Row, view: View) -> Option<Thread> {
    if view == View::Archive {
        if needs_you(&row) {
            return None;
        }
        row.members.retain(|(t, _)| !going_on(t));
    }
    let newest = |pick: &dyn Fn(&(Thread, bool)) -> bool| {
        row.members
            .iter()
            .enumerate()
            .filter(|(_, m)| pick(m))
            .max_by_key(|(_, (t, _))| activity_at(&t.root, &t.replies))
            .map(|(i, _)| i)
    };
    let lead = match view {
        View::Inbox => {
            if !needs_you(&row) {
                return None;
            }
            newest(&|m| is_open(m) && (asks_you(&m.0) || broken(&m.0))).or_else(|| newest(&is_open))?
        }
        View::Archive => newest(&|m| m.1)?,
    };
    let mut members: Vec<Thread> = row.members.into_iter().map(|(t, _)| t).collect();
    let mut lead = members.remove(lead);
    lead.related = members;
    Some(lead)
}

/// When a row was archived: the latest of its threads.
fn row_archived_at(t: &Thread) -> Option<Ts> {
    std::iter::once(t).chain(&t.related).filter_map(|x| x.root.archived_at).max()
}

/// Where a row sits in the Archive, newest first: the later of when it was archived and when it last moved.
pub fn archive_at(t: &Thread) -> Ts {
    row_archived_at(t).into_iter().chain(std::iter::once(activity_at(&t.root, &t.replies))).max().unwrap_or(t.root.created_at)
}

/// What a report was, as a thread is named: a schedule's label, "Manual run", a task's name, or the kind.
pub fn report_label(kind: &RunKind) -> String {
    match kind {
        RunKind::Scheduled { label, .. } | RunKind::Reply { label, .. } | RunKind::Task { label, .. } => label.clone(),
        RunKind::Manual => "Manual run".into(),
        k => crate::claude::kind_label(k),
    }
}

/// A reply's label on the thread rooted at `root`, and whether that thread is a task: a conversation you started
/// (an older build's chat message an answer came back into too) is named as a task is, a report by its label.
pub fn reply_label(root: &Run) -> (String, bool) {
    let chat = matches!(root.kind, RunKind::Chat);
    let label = if chat { task_label(&root.prompt, &root.attachments) } else { report_label(&root.kind) };
    (label, chat || matches!(root.kind, RunKind::Task { .. }))
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

    fn chore(employee: &str, subject: &str, answered: bool) -> Run {
        let mut r = Run::new(employee, RunKind::Manual, "go".into(), chrono::Utc::now());
        r.state = RunState::Failed;
        let mut card = Card::new("Waiting on you.", None, None, CardAsk::Chore { subject: subject.into(), url: None });
        card.answer = answered.then(|| CardAnswer { at: chrono::Utc::now(), how: AnswerHow::Tapped, text: "Done.".into(), held: false });
        r.cards = vec![card];
        r
    }

    fn row(r: Run) -> Row {
        Row { members: vec![(Thread::new(r, vec![]), true)] }
    }

    #[test]
    fn a_thread_with_an_unanswered_card_is_open() {
        let plain = Thread::new(Run::new("e1", RunKind::Manual, "go".into(), chrono::Utc::now()), vec![]);
        let waiting = Thread::new(chore("e1", "chrome", false), vec![]);
        let answered = Thread::new(chore("e1", "chrome", true), vec![]);
        assert!(!card_open(&[plain.clone(), answered.clone()]));
        assert!(card_open(&[plain.clone(), waiting.clone()]));
        assert_eq!(open_card(&waiting).map(|r| &r.id), Some(&waiting.root.id));
        let mut reply = chore("e1", "chrome", false);
        reply.kind = RunKind::Reply { to_run_id: plain.root.id.clone(), label: "Manual run".into(), task: false, card: false };
        let with_reply = Thread::new(plain.root.clone(), vec![reply.clone()]);
        assert_eq!(open_card(&with_reply).map(|r| &r.id), Some(&reply.id));
    }

    #[test]
    fn everyone_waiting_on_one_chore_shares_a_row() {
        let rows = vec![
            row(chore("e1", "chrome", false)),
            row(chore("e2", "sign-in:acme", false)),
            row(chore("e2", "chrome", false)),
            row(chore("e3", "chrome", true)),
        ];
        let merged = merge_rows(rows, chore_key);
        let sizes: Vec<usize> = merged.iter().map(|r| r.members.len()).collect();
        assert_eq!(sizes, vec![2, 1, 1]);
        let employees: Vec<&str> = merged[0].members.iter().map(|(t, _)| t.root.employee_id.as_str()).collect();
        assert_eq!(employees, vec!["e1", "e2"]);
        assert_eq!(chore_key(&merged[1]).as_deref(), Some("chore:sign-in:acme"));
        assert_eq!(chore_key(&merged[2]), None, "an answered chore waits on nobody");
    }

    #[test]
    fn a_schedules_failures_still_merge() {
        let failed = |schedule: &str| {
            let kind = RunKind::Scheduled { schedule_id: schedule.into(), slot: chrono::Utc::now(), label: "Sweep".into(), quiet: true, model: None, effort: None, review: false };
            let mut r = Run::new("e1", kind, "go".into(), chrono::Utc::now());
            r.state = RunState::Failed;
            r
        };
        let streak = |row: &Row| match &row.members.first()?.0.root.kind {
            RunKind::Scheduled { schedule_id, .. } => Some(schedule_id.clone()),
            _ => None,
        };
        let merged = merge_rows(vec![row(failed("s1")), row(failed("s2")), row(failed("s1"))], streak);
        assert_eq!(merged.iter().map(|r| r.members.len()).collect::<Vec<_>>(), vec![2, 1]);
    }

    #[test]
    fn a_routine_needs_you_once_it_fails_three_times_in_a_row() {
        let failed = |limited: bool| {
            let kind = RunKind::Scheduled { schedule_id: "s1".into(), slot: chrono::Utc::now(), label: "Sweep".into(), quiet: true, model: None, effort: None, review: false };
            let mut r = Run::new("e1", kind, "go".into(), chrono::Utc::now());
            r.state = RunState::Failed;
            r.limited = limited;
            (Thread::new(r, vec![]), true)
        };
        let streak = |n: usize, limited: usize| Row { members: (0..n).map(|i| failed(i < limited)).collect() };
        assert!(!needs_you(&streak(1, 0)), "once is in the Archive");
        assert!(!needs_you(&streak(2, 0)));
        assert!(needs_you(&streak(3, 0)));
        assert!(!needs_you(&streak(3, 1)), "a failure from the usage limit does not count");
        let mut manual = Run::new("e1", RunKind::Manual, "go".into(), chrono::Utc::now());
        manual.state = RunState::Failed;
        assert!(needs_you(&row(manual.clone())), "anything else that breaks needs you at once");
        manual.limited = true;
        assert!(!needs_you(&row(manual)));
        assert!(present(streak(3, 0), View::Inbox).is_some());
        assert!(present(streak(2, 0), View::Inbox).is_none());
        assert!(present(streak(2, 0), View::Archive).is_some(), "the Archive shows what does not need you yet");
        assert!(present(streak(3, 0), View::Archive).is_none(), "nothing is in both");
    }

    #[test]
    fn a_thread_needs_you_for_cards_flags_and_blocks_not_reports() {
        let mut r = Run::new("e1", RunKind::Manual, "go".into(), chrono::Utc::now());
        r.state = RunState::Succeeded;
        r.ended_at = Some(chrono::Utc::now());
        r.summary = Some("Done.".into());
        let t = |r: &Run| Thread::new(r.clone(), vec![]);
        assert!(!asks_you(&t(&r)), "a plain report asks nothing");
        let mut card = Card::new("Merge it?", None, Some("ask:pr:1".into()), CardAsk::Choices { choices: vec!["Merge".into(), "Not now".into()], recommended: 0 });
        r.needs_you = vec!["Merge it?".into()];
        r.cards = vec![card.clone()];
        assert!(asks_you(&t(&r)));
        card.answer = Some(CardAnswer { at: chrono::Utc::now(), how: AnswerHow::Tapped, text: "Chose: Merge.".into(), held: false });
        r.cards = vec![card];
        assert!(!asks_you(&t(&r)), "an answered card asks nothing, though its message stays");
        r.needs_you.push("The tidy dropped a rule.".into());
        assert!(asks_you(&t(&r)), "a flag Omega Star raised");
        r.needs_you.truncate(1);
        r.denials = vec![Denial { tool: "Bash".into(), brief: "ls".into(), suggested_rule: "Bash(ls:*)".into(), ..Default::default() }];
        assert!(asks_you(&t(&r)));
        let mut retried = Run::new("e1", RunKind::Reply { to_run_id: r.id.clone(), label: "Manual run".into(), task: false, card: false }, "ok".into(), chrono::Utc::now());
        retried.state = RunState::Succeeded;
        retried.ended_at = Some(chrono::Utc::now());
        assert!(!asks_you(&Thread::new(r.clone(), vec![retried.clone()])), "a retry that went through is no longer blocked");
        let mut retrying = retried.clone();
        retrying.state = RunState::Queued;
        retrying.ended_at = None;
        assert!(!asks_you(&Thread::new(r.clone(), vec![retrying])), "nor while the retry you allowed is on its way");
        r.denials.clear();
        r.needs_you.push("The tidy dropped a rule.".into());
        assert!(!asks_you(&Thread::new(r, vec![retried])), "a reply after a flag answers it");
    }

    #[test]
    fn news_asks_you_until_seen_but_holds_up_no_card() {
        let mut r = Run::new("e1", RunKind::Manual, "go".into(), chrono::Utc::now());
        r.state = RunState::Succeeded;
        r.ended_at = Some(chrono::Utc::now());
        r.needs_you = vec!["The spring sale is live.".into()];
        r.cards = vec![Card::new("The spring sale is live.", None, Some("ask:sale".into()), CardAsk::News)];
        let t = Thread::new(r.clone(), vec![]);
        assert!(asks_you(&t) && needs_you(&row(r.clone())), "it keeps its row in the Inbox");
        assert!(open_card(&t).is_none(), "so nothing waits on it");
        r.cards[0].answer = Some(CardAnswer { at: chrono::Utc::now(), how: AnswerHow::Tapped, text: String::new(), held: false });
        assert!(!asks_you(&Thread::new(r, vec![])), "seen");
    }

    #[test]
    fn a_check_that_worked_for_long_is_not_quiet() {
        let at = chrono::Utc::now();
        let scheduled = |schedule: &str| RunKind::Scheduled { schedule_id: schedule.into(), slot: at, label: "Check: Did it ship".into(), quiet: true, model: None, effort: None, review: false };
        let mut check = Run::new("e1", scheduled(&format!("{CHECK_PREFIX}c1")), "go".into(), at);
        check.state = RunState::Succeeded;
        check.duration_ms = Some(CHECK_QUIET_MS);
        assert!(quiet(&check) && !reaches_you(&check, false, true), "half an hour is still a quick look");
        check.duration_ms = Some(CHECK_QUIET_MS + 1);
        assert!(!quiet(&check) && reaches_you(&check, false, true), "longer is work, which reports");
        let mut routine = check.clone();
        routine.kind = scheduled("s1");
        assert!(quiet(&routine), "a quiet routine stays quiet however long it takes");
    }

    #[test]
    fn a_run_you_wrote_into_reaches_you_and_waits_until_you_mark_it_done() {
        let at = chrono::Utc::now();
        let routine = RunKind::Scheduled { schedule_id: "s1".into(), slot: at, label: "Sweep".into(), quiet: true, model: None, effort: None, review: false };
        let handed = RunKind::Asked { parent_run_id: "r1".into(), asked_by: "e2".into(), task: true };
        for kind in [routine, handed] {
            let mut r = Run::new("e1", kind, "go".into(), at);
            r.state = RunState::Succeeded;
            r.ended_at = Some(at);
            assert!(!reaches_you(&r, false, false), "{:?}", r.kind);
            r.steered = true;
            assert!(reaches_you(&r, false, false) && reaches_you(&r, true, false), "{:?}: you wrote into it", r.kind);
            let row = |r: &Run| Row { members: vec![(Thread::new(r.clone(), vec![]), reaches_you(r, false, false))] };
            assert!(needs_you(&row(&r)) && present(row(&r), View::Inbox).is_some(), "it waits for you like a reply");
            assert!(present(row(&r), View::Archive).is_none(), "nothing is in both");
            r.archived_at = Some(at);
            assert!(!needs_you(&row(&r)) && present(row(&r), View::Archive).is_some(), "marked done, it is in the Archive");
        }
        let mut turn = Run::new("e1", RunKind::Reply { to_run_id: "r1".into(), label: "Sweep".into(), task: false, card: false }, "go".into(), at);
        turn.state = RunState::Succeeded;
        turn.steered = true;
        assert!(!reaches_you(&turn, false, true), "a turn belongs to its thread");
    }

    #[test]
    fn a_review_reaches_you_whoever_its_employee_reports_to() {
        // Wednesday noon UTC falls in the same ISO week in every time zone.
        let slot = "2026-10-14T12:00:00Z".parse::<Ts>().unwrap();
        let kind = |review: bool| RunKind::Scheduled { schedule_id: "s1".into(), slot, label: "Weekly review".into(), quiet: false, model: None, effort: None, review };
        let mut review = Run::new("e2", kind(true), "go".into(), slot);
        review.state = RunState::Succeeded;
        assert!(reaches_you(&review, false, false), "a second-level employee's review");
        let mut routine = review.clone();
        routine.kind = kind(false);
        assert!(!reaches_you(&routine, false, false));
        assert_eq!(review_week(&review).as_deref(), Some("review@2026-W42"));
        assert_eq!(review_week(&routine), None);
    }

    #[test]
    fn an_answer_in_a_conversation_joins_it_and_reaches_you_only_there() {
        let at = chrono::Utc::now();
        let task = Run::new("e1", RunKind::Task { label: "Draft it".into(), from: None, context: String::new() }, "Draft it.".into(), at);
        let came = |thread: Option<String>| {
            let mut r = Run::new("e1", RunKind::Answered { asked_run_id: "r1".into(), from: "e2".into(), task: false, chat: false, thread }, "Bo answered.".into(), at);
            r.state = RunState::Succeeded;
            r.needs_you = vec!["Look at this.".into()];
            r
        };
        let turn = came(Some(task.id.clone()));
        let grouped = group(vec![turn.clone()]);
        assert_eq!(grouped.get(&task.id).map(|v| v.len()), Some(1));
        assert!(!reaches_you(&turn, false, true), "it is a turn of its conversation, even when it flags something");
        assert!(reaches_you(&came(None), false, true), "an answer to background work is a report of its own");
    }

    #[test]
    fn an_ask_from_a_reply_in_a_conversation_is_a_row_of_its_own() {
        let at = chrono::Utc::now();
        let task = Run::new("e1", RunKind::Task { label: "Draft it".into(), from: None, context: String::new() }, "Draft it.".into(), at);
        let report = Run::new("e1", RunKind::Manual, "Count the stock.".into(), at);
        let reply = |to: &Run, task: bool| Run::new("e1", RunKind::Reply { to_run_id: to.id.clone(), label: "Draft it".into(), task, card: false }, "Ask Bo.".into(), at);
        let ask = |from: &Run| {
            let mut r = Run::new("e2", RunKind::Asked { parent_run_id: from.id.clone(), asked_by: "e1".into(), task: false }, "What are the numbers?".into(), at);
            r.state = RunState::Failed;
            r
        };
        let (in_task, in_report) = (reply(&task, true), reply(&report, false));
        let (a, b) = (ask(&in_task), ask(&in_report));
        let known = [task.clone(), report.clone()];
        let lookup = |id: &str| known.iter().find(|r| r.id == id).cloned();
        let top = HashSet::from(["e1".to_string()]);
        let rows = file(&[a.clone(), b.clone()], vec![in_task, in_report, a.clone(), b.clone()], lookup, &top);
        let ids: Vec<Vec<&str>> = rows.iter().map(|r| r.members.iter().map(|(t, _)| t.root.id.as_str()).collect()).collect();
        assert_eq!(ids[0], vec![a.id.as_str()], "the conversation stays out of the row, so marking the ask done leaves it alone");
        assert_eq!(ids[1].len(), 2, "an ask from your reply to a report still joins the report's row");
        assert!(ids[1].contains(&report.id.as_str()) && ids[1].contains(&b.id.as_str()));
    }
}
