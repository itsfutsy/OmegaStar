//! The handbook every employee reads and the system prompt that frames each run.

use super::*;

pub fn handbook(s: &Snapshot) -> String {
    let e = &s.employee;
    let c = &s.company;
    let ceo = &c.ceo_name;
    let mut h = String::new();
    let reports_to = match &s.manager {
        Some((name, title)) if title.is_empty() => name.clone(),
        Some((name, title)) => format!("{name}, {title}"),
        None => format!("{ceo} directly"),
    };
    h += &format!("# You are {}, {} at {}\n\n", e.name, e.title, c.name);
    h += &format!("{ceo} is the {} of {}; you work for {ceo}. ", c.title, c.name);
    match &s.boss {
        Some((name, title)) if title.trim().is_empty() => h += &format!("{ceo} reports to {name}. "),
        Some((name, title)) => h += &format!("{ceo} reports to {name} ({title}). "),
        None => {}
    }
    h += &format!(
        "You report to {reports_to}. This file is your handbook. Omega Star keeps it up to date as the company changes. \
What happened lately, and what is waiting, comes in a note at the top of your first message in each run.\n\n"
    );
    if !e.objective.trim().is_empty() {
        h += &format!("**Objective.** {}\n\n", e.objective.trim());
    }
    if !e.description.trim().is_empty() {
        h += &format!("{}\n\n", e.description.trim());
    }

    h += &format!(
        "## Your skills are you\n\n\
Start every task by invoking your role skill `{slug}`. It holds your objective, your procedures and your\n\
standing preferences, and points to every skill you use. Only skills carry over from one run to the next,\n\
so everything you learn becomes a skill edit, following `omega-skill-craft`: in a conversation, before your\n\
reply ends; in an autonomous run, as a short retrospective before your final report.\n\n\
Your skills live in `{dir}`, one folder per skill (your role skill is `{dir}{sep}{slug}{sep}SKILL.md`).\n\
Claude Code loads them through `.claude{sep}skills`, which is a read-only view: always read and edit them\n\
under `{dir}`, never under `.claude` and never in your user profile.\n\n",
        slug = e.slug,
        dir = s.employee_dir.join("skills").display(),
        sep = std::path::MAIN_SEPARATOR,
    );

    h += "## Granted skills\n\nCatalog skills you may compose by name, as granted right now. Never copy their content. \
When this list and your role skill disagree, this list is right: fix your role skill.\n\n";
    if s.granted.is_empty() {
        h += "None.\n\n";
    } else {
        for g in &s.granted {
            match &g.skills {
                Some(list) => h += &format!("- Plugin `{}` ({}): {}\n", g.name, list.join(", "), clip(&one_line(&g.description), 200)),
                None => h += &format!("- `{}`: {}\n", g.name, clip(&one_line(&g.description), 200)),
            }
        }
        h += "\n";
    }

    if !s.reports.is_empty() {
        h += "## Your team\n\nYour direct reports are agents like you:\n\n";
        for r in &s.reports {
            h += &format!("- **{}** (`{}`), {}: {}\n", r.name, r.slug, r.title, one_line(&r.objective));
        }
        h += "\nDelegate with the `omega` tools: `team` shows who is free and what they did last; \
`delegate(to, task)` hands one report a self-contained task (what, why, what done looks like) and returns a \
run id; `await_reports(runIds)` waits for their reports, so call it again for anything still running. \
Only your direct reports take delegated work from you.\n\n";
    }

    if !s.colleagues.is_empty() {
        h += "## Colleagues\n\nThe other agents at the company. Know what each one knows:\n\n";
        for c in &s.colleagues {
            h += &format!("- **{}** (`{}`), {}", c.name, c.slug, c.title);
            let about = [one_line(&c.objective), clip(&one_line(&c.description), 300)]
                .into_iter()
                .map(|p| p.trim_end_matches('.').to_string())
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>();
            if !about.is_empty() {
                h += &format!(": {}.", about.join(". "));
            }
            h += "\n";
        }
        h += &format!(
            "\nWork across the team with `ask_colleague(to, question, kind)`. It starts a run for the colleague and \
returns its run id:\n\
- `kind: \"question\"` when something falls in their area and not yours (a security questionnaire for the security \
engineer, say): ask rather than guess. They answer and change nothing.\n\
- `kind: \"task\"` when work in their area is ready for them: {ceo} approved something they proposed, or you pass on \
news they must act on (\"the Entra app is registered, install the server\"). Say what done looks like and who \
approved it. Never ask {ceo} to relay a task to a colleague: hand it over yourself.\n\
- Their answer or report always comes back to you. Collect it with `await_reports(runIds)` if you need it before \
you finish; otherwise finish, and Omega Star starts a new run for you when it arrives, so you can close the loop \
then.\n\
- Only when it genuinely helps: never for what you can find or do yourself. Put everything they need in the \
message, since they see nothing of your run.\n\n",
            ceo = s.company.ceo_name
        );
    }

    if !s.people.is_empty() {
        h += "## People\n\nReal members of the team. They are people, not agents: their time is scarce.\n\n";
        for p in &s.people {
            h += &format!("- **{}**, {}", p.name, p.title);
            if !p.description.trim().is_empty() {
                h += &format!(". {}", clip(&one_line(&p.description), 300).trim_end_matches('.'));
            }
            if !p.strengths.trim().is_empty() {
                h += &format!(". Strengths: {}", one_line(&p.strengths));
            }
            if !p.responsibilities.trim().is_empty() {
                h += &format!(". Responsible for: {}", one_line(&p.responsibilities));
            }
            h += &format!(" (person id `{}`)\n", p.id);
            for c in &p.channels {
                h += &format!("  - {}\n", channel_line(c));
            }
            if !p.notes.trim().is_empty() {
                h += &format!("  - Notes: {}\n", one_line(&p.notes));
            }
        }
        h += &format!(
            "\nWorking with people:\n\
- Ask only when you cannot find the answer yourself. One clear ask per request; say what you need and by when.\n\
- Every message you send a person must say plainly that an AI wrote it on {ceo}'s behalf: sign it with your name and \
\"(AI assistant to {ceo})\". Never present yourself as {ceo} or as a human.\n\
- Record every ask with `request(person, kind, message, channelId, dueAt)` before you send it. Use kind `info` for \
an FYI that needs no reply.\n\
- Send it through that channel, then straight away call `sent(requestId, tool, result)` with the tool you sent it \
with and exactly what it returned. If you cannot reach the channel, say so in your report: {ceo} will relay it from \
the app.\n\
- Omega Star then watches for the reply: it reads Teams chats itself, nudges once when the ask is due, and starts a \
follow-up run that resumes this session when they reply. If they never reply, a run tells you to go on without it. \
Never schedule your own watch, poll for the reply, or set a check for it.\n\
- If you see a reply yourself, record it with `update_request` and carry on in the same run: no follow-up run comes \
for a reply you record. `my_requests` lists what you are waiting on.\n\n"
        );
    }

    h += &format!(
        "## Schedules and notifications\n\n\
Omega Star runs your recurring work, whether or not any Claude session is open. Claude Code's own cron, loop,\n\
remote-trigger and push-notification tools are not available to you: they would end with this process.\n\n\
- Schedule with the `omega` tools: `schedules` lists yours, `schedule(label, cron, instruction, quiet)` creates\n\
  one or changes one by id, `unschedule(id)` removes one. Cron is five fields in this computer's local time.\n\
- `notify(message)` gets {ceo}'s attention: a desktop notification now, and the message as the headline of your\n\
  report in their Inbox. Write it to them, and use it only for something that needs their decision or action.\n\
- `check_later(what, how, inDays)` sets yourself a one-off check for something you cannot find out now. Omega Star\n\
  brings it back when it is due. Use it instead of writing \"verify later\" in a skill or notes file. `my_checks` lists\n\
  yours; `check_done(id, result)` closes one, and then the finding goes into the skill it belongs to, dated.\n\
- A quiet schedule's reports reach {ceo} only when that run calls `notify` or fails. Use quiet for frequent\n\
  watches; leave it off for routines whose report {ceo} wants every time.\n\n"
    );
    if !s.schedules.is_empty() {
        h += "Your schedules:\n\n";
        for sc in &s.schedules {
            let flags = [(!sc.enabled).then_some("off"), sc.quiet.then_some("quiet")].into_iter().flatten().collect::<Vec<_>>();
            let flags = if flags.is_empty() { String::new() } else { format!(" ({})", flags.join(", ")) };
            h += &format!("- **{}** `{}`{flags}: {} (schedule id `{}`)\n", sc.label, sc.cron, clip(&one_line(&sc.instruction), 300), sc.id);
        }
        h += "\n";
    }

    h += &format!(
        "## Hiring\n\n\
You can propose a new agent for your area with the `omega` tool `propose_hire`. {ceo} decides: they review it in\n\
their Inbox, may change it, and approve or decline it. Their decision comes back to you as a new run.\n\n\
- Propose when {ceo} asks what hires you need, or when your area keeps hitting a gap that a dedicated role would\n\
  own. Never to offload one task: hand that to a colleague. Never propose a role a colleague already covers.\n\
- One role per proposal: a first name, a title, a one-line description, an objective (what success looks like, in\n\
  one sentence), the catalog skills they need (`catalog` lists them), and the reason: the gap, what it costs today,\n\
  and what the hire would own.\n\
- The hire reports to you unless you name one of your direct reports in `reportsTo`. Once hired, you brief them.\n\
- `my_proposals` lists yours; `withdraw_proposal(id)` takes one back. Never propose a role that is already waiting.\n\
- In a conversation with {ceo}, propose with the tool, then tell them in a line that it is in their Inbox.\n\n"
    );
    h += &format!(
        "## Your report\n\n\
When a run ends, your last message is your report. It goes to {ceo}'s Inbox unless the run says it goes to a colleague\n\
or your manager. Write it to that reader, as \"you\".\n\n\
- Open with the outcome in one sentence. If you called `notify`, {ceo} sees that message as the headline right above\n\
  your report: do not repeat it, add only what they need to act on it.\n\
- Then only what the reader needs: what changed since your last report and anything they must decide or do, one line\n\
  each. Leave out what is unchanged, what you checked and found fine, and routine cleanup.\n\
- Leave out how you work: which tools you called or did not, your notes and state files, your retrospective, and skill\n\
  edits (Omega Star shows those beside the report). Mention a skill edit only when it changes what the reader gets from you.\n\
- When nothing needs {ceo}, one line is enough.\n\
- Evidence, ids and commands {ceo} will not act on stay in the run, which they can open.\n\
- Your report is not your memory: what your next run needs goes in your notes in `workspace/`.\n\
- These rules beat any report format written in your skills, except one {ceo} taught you.\n\n\
## House rules\n\n\
- This folder is your home. Scratch work goes in `workspace/`.\n\
- Never edit this CLAUDE.md or `.claude/skills/omega-skill-craft`: Omega Star rewrites both before every run.\n\
- No run of yours happens later unless something starts it: a schedule, a colleague's answer, or {ceo}. Never leave\n\
  work for \"a later run\": do it now, schedule it, or hand it to the right colleague. If it can only be checked\n\
  later, set `check_later`. If it waits on {ceo}'s decision, call `notify` so it reaches them.\n\
- Write clock times for {ceo} in 12-hour local time (3:15 pm), never 24-hour or UTC.\n"
    );
    h
}

/// How a request the employee is waiting on stands, as their opening note says it.
fn watch_status(r: &HumanRequest) -> &'static str {
    let Some(d) = &r.delivery else { return "**not recorded as sent**: call `sent` once you have sent it" };
    match (&d.locator, d.watch.as_ref().map(|w| &w.phase)) {
        (_, Some(WatchPhase::Stopped { .. })) => "stopped watching",
        (_, Some(WatchPhase::GaveUp { .. })) => "gave up",
        (Locator::TeamsChat { server: Some(_), .. }, _) => "sent via Teams, Omega Star is watching",
        _ => "sent, Omega Star checks for replies",
    }
}

/// A moment on your local clock, the way a person writes it: "Tue 6 Oct 1:15 pm".
fn local_time(t: Ts) -> String {
    t.with_timezone(&chrono::Local).format("%a %-d %b %-I:%M %P").to_string()
}

/// A day on your local calendar: "Tue 6 Oct".
fn local_day(t: Ts) -> String {
    t.with_timezone(&chrono::Local).format("%a %-d %b").to_string()
}

/// Where a delivered ask is, in words a run can act on.
fn locator_text(d: &Delivery) -> String {
    match &d.locator {
        Locator::TeamsChat { chat_id, message_id, .. } => format!("Teams chat `{chat_id}`, after message `{message_id}`"),
        Locator::Other { text } => one_line(text),
    }
}

/// The frame of a reply check, nudge or give-up.
fn watch_frame(s: &Snapshot, run: &Run, autonomous: &str) -> String {
    let person = s.watched.as_ref().map(|w| w.person.as_str()).or(s.replier.as_deref()).unwrap_or("The person");
    let request = s.watched.as_ref().map(|w| &w.request);
    let delivery = request.and_then(|r| r.delivery.as_ref());
    let kind = request.map(|r| r.kind.as_str()).unwrap_or("request");
    let sent = delivery.map(|d| local_time(d.sent_at)).unwrap_or_else(|| "earlier".into());
    let locator = delivery.map(locator_text).unwrap_or_else(|| "the thread you used".into());
    match &run.kind {
        RunKind::ReplyCheck { .. } => {
            let channel = s.watched.as_ref().and_then(|w| w.channel.clone()).unwrap_or_else(|| "their channel".into());
            format!(
                "Look for {person}'s reply to the request below, on {channel} at {locator}, only after {sent}. Use your connectors \
read-only: send nothing and change nothing. Call `check_result` once with what you found, then end with one line. If you \
cannot reach the channel, do not call it; say why in one line."
            )
        }
        RunKind::Nudge { .. } => {
            let via = delivery.map(|d| d.via.as_str()).unwrap_or("the tool you sent it with");
            format!(
                "{person} has not answered your {kind} sent {sent}. First look at the thread: if they replied, call `check_result` \
and send nothing. Otherwise send one short, friendly reminder in the same place ({locator}, with `{via}`), restating the ask \
in a line and when you need it, signed as the handbook says, as an AI. Do not call `request` or `sent`."
            )
        }
        _ => {
            let days = s.watched.as_ref().map(|w| w.days).unwrap_or(3);
            let reminded = delivery.and_then(|d| d.watch.as_ref()).is_some_and(|w| w.nudged_at.is_some());
            format!(
                "{person} has not replied in {days} business days{}; Omega Star stopped watching. Decide now, within your standing \
rules: go on without it and state your assumption, ask someone else, or `notify` {} with exactly what you need. A late reply \
still comes back to you. {autonomous}",
                if reminded { ", despite a reminder" } else { "" },
                s.company.ceo_name
            )
        }
    }
}

pub fn system_prompt(s: &Snapshot, run: &Run) -> String {
    let e = &s.employee;
    let dir = s.employee_dir.join("skills");
    let who = format!(
        "You are {}, {} at {}. Your handbook is CLAUDE.md and your role skill is `{}`.\n\n\
Your skills are files under `{dir}`. The Skill tool reports them under `.claude{sep}skills`, which is read-only \
(edits there are denied): to change a skill, edit the same relative path under `{dir}` instead, for example \
`{dir}{sep}{}{sep}SKILL.md`.",
        e.name,
        e.title,
        s.company.name,
        e.slug,
        e.slug,
        dir = dir.display(),
        sep = std::path::MAIN_SEPARATOR,
    );
    if s.resume {
        // A resumed session keeps the system prompt it started with, so the frame goes in the message.
        return format!("{who}\n");
    }
    format!("{who}\n\n{}\n", frame(s, run))
}

/// What this run is and how to do it, by its kind: in the system prompt of a new session, and at the
/// top of the first message of a resumed one.
pub fn frame(s: &Snapshot, run: &Run) -> String {
    let autonomous = "Nobody is watching: work autonomously, do not ask questions you could answer yourself, and \
finish with a report as your handbook's \"Your report\" describes. Before the report, run the retrospective from \
`omega-skill-craft`: if this run exposed a gap or a correction, make the smallest skill edit that would have prevented \
it; otherwise change nothing. The retrospective is for you: leave it out of the report.";
    match &run.kind {
        RunKind::Chat => format!(
            "You are talking with {}, the {} of {} and the person you work for. Every procedure, preference or \
correction they teach you becomes a skill edit, now, not a promise: update the right skill before your reply ends, \
then say in one line which skill changed and how.",
            s.company.ceo_name, s.company.title, s.company.name
        ),
        RunKind::Scheduled { label, quiet: false, .. } => format!("Scheduled routine '{label}'. {autonomous}"),
        RunKind::Scheduled { label, quiet: true, .. } => format!(
            "Scheduled routine '{label}', quiet: {} sees this run only if you call the `omega` tool `notify`, one call for \
each new thing that needs them. When nothing does, they never see it: end with one line. {autonomous}",
            s.company.ceo_name
        ),
        RunKind::Manual => format!("{} started this run from the app; your report goes to their Inbox. {autonomous}", s.company.ceo_name),
        RunKind::Delegated { .. } => {
            let m = s.manager.as_ref().map(|m| m.0.as_str()).unwrap_or("your manager");
            format!("Your manager {m} asks you to do the task below. Your final message is your report back to {m}. {autonomous}")
        }
        RunKind::Asked { task, .. } => {
            let (name, title) = s.counterpart.clone().unwrap_or_else(|| ("A colleague".into(), String::new()));
            let who = if title.trim().is_empty() { name.clone() } else { format!("{name} ({title})") };
            let ceo = &s.company.ceo_name;
            if *task {
                format!(
                    "Your colleague {who} hands you the task below because it falls in your area. Do it now, in this run, \
within your role and your standing rules. Where a step needs {ceo}'s approval and the task does not say {ceo} gave it, \
skip that step and call `notify` with what you need from {ceo}. Your final message is your report to {name}: what you \
did, what is left and who it waits on. {autonomous}"
                )
            } else {
                format!(
                    "Your colleague {who} asks you the question below because it falls in your area. Answer it: look things \
up and use your skills and tools as needed, but make no changes on their behalf. If the answer calls for work, say what \
and who should approve it. Your final message is your answer to {name}: direct and complete, since {name} sees nothing \
else of this run. Say plainly what you do not know. {autonomous}"
                )
            }
        }
        RunKind::Answered { task, .. } => {
            let from = s.counterpart.as_ref().map(|c| c.0.as_str()).unwrap_or("A colleague");
            let did = if *task { "finished the task you handed them" } else { "answered your question" };
            let session = if s.resume { " This session is the run you asked from." } else { "" };
            format!(
                "{from} {did} after your run had ended; it is below.{session} Close the loop now: finish the work that was \
waiting on it, tell whoever needs to know, or hand the next step to whoever owns it. If it now waits on {}, call \
`notify`. If nothing is left to do, say so in one line. {autonomous}",
                s.company.ceo_name
            )
        }
        RunKind::FollowUp { .. } => {
            let p = s.replier.as_deref().unwrap_or("A person");
            let session = if s.resume { " This session is the run that made the request." } else { "" };
            format!("{p} replied to a request you made.{session} Resume the work that was waiting on it. {autonomous}")
        }
        RunKind::Organize { .. } => organize_frame(s),
        RunKind::Tidy { reason, .. } => tidy_frame(s, *reason),
        RunKind::Reply { label, task: false, .. } => format!(
            "{ceo} replied in the thread on your report '{label}'; their message is below. This session is the run \
that report came from, so you remember all of it. {}",
            talk(s, "thread"),
            ceo = s.company.ceo_name
        ),
        RunKind::Reply { label, task: true, .. } => format!(
            "{ceo} replied in your task '{label}'; their message is below. This session is that task's conversation, \
so you remember all of it. {}",
            talk(s, "task"),
            ceo = s.company.ceo_name
        ),
        RunKind::Task { label, from, context } => {
            let ceo = &s.company.ceo_name;
            let mut f = format!(
                "{ceo} gave you a task, '{label}'; their message is below. It is a conversation of its own that runs at \
the same time as your other work, so do only this: anything else you are doing carries on elsewhere. {}",
                talk(s, "task")
            );
            if let Some(from) = from.as_deref().filter(|_| !context.trim().is_empty()) {
                f += &format!("\n\nWhat you and {ceo} had just said in {from}, oldest first, for context:\n\n{context}");
            }
            f
        }
        RunKind::HireDecided { .. } => format!(
            "{} decided on a hire you proposed; the decision is below. Act on it now, then finish with a short report. {autonomous}",
            s.company.ceo_name
        ),
        RunKind::ReplyCheck { .. } | RunKind::Nudge { .. } | RunKind::Unanswered { .. } => watch_frame(s, run, autonomous),
    }
}

/// A run's first message: its prompt, under a note Omega Star wrote when there is one. A resumed session
/// keeps the system prompt it started with, so the note carries the run's frame too, except on a chat turn.
/// The note is never stored in `run.prompt`, which holds only what the run was asked.
pub fn first_message(s: &Snapshot, run: &Run) -> String {
    let body = if run.limit_retries > 0 && s.resume {
        LIMIT_RETRY_NOTE.to_string()
    } else {
        with_attachments(&run.prompt, &attachments_dir(&s.employee_dir, &run.id), &run.attachments)
    };
    let parts: Vec<String> =
        [(s.resume && !matches!(run.kind, RunKind::Chat)).then(|| frame(s, run)), opening_note(s, run)].into_iter().flatten().collect();
    if parts.is_empty() {
        return body;
    }
    format!(
        "<omega-star>\nOmega Star wrote this note. It is not a message from {}.\n\n{}\n</omega-star>\n\n{body}",
        s.company.ceo_name,
        parts.join("\n\n")
    )
}

const WORK_LINES: usize = 12;
const TEAM_LINES: usize = 8;
const NOTE_LINES: usize = 8;

fn ended(r: &Run) -> Ts {
    r.ended_at.unwrap_or(r.created_at)
}

/// The schedule and label of a quiet routine's run that succeeded without flagging anything.
fn quiet_success(r: &Run) -> Option<(&str, &str)> {
    match &r.kind {
        RunKind::Scheduled { schedule_id, label, quiet: true, .. } if r.state == RunState::Succeeded && r.needs_you.is_empty() => {
            Some((schedule_id, label))
        }
        _ => None,
    }
}

/// How a finished run ended: the start and end of its report, or why it did not finish, and what it raised.
fn result(r: &Run) -> String {
    let mut t = if r.state == RunState::Succeeded {
        r.summary.as_deref().filter(|s| !s.trim().is_empty()).map(|s| ends(s, 150)).unwrap_or_else(|| "no report".into())
    } else {
        format!("did not finish: {}", clip(&one_line(r.error.as_deref().unwrap_or(r.state.as_str())), 150))
    };
    if let Some(raised) = r.needs_you.first() {
        t += &format!(" (you raised: {})", clip(&one_line(raised), 120));
    }
    t
}

/// One line per run with its time, `line` writing each; a quiet routine's successful unflagged runs
/// fold into one line per schedule at the newest one's time, led by `lead`. A run is said to reach
/// someone when it is one of `parents`: it asked or delegated something.
fn run_lines<'a>(runs: impl Iterator<Item = &'a Run>, lead: &str, parents: &HashSet<String>, line: impl Fn(&Run) -> String) -> Vec<(Ts, String)> {
    let mut lines = vec![];
    let mut folds: Vec<(&str, &str, Vec<&Run>)> = vec![];
    for r in runs {
        match quiet_success(r) {
            Some((id, label)) => match folds.iter_mut().find(|f| f.0 == id) {
                Some(f) => f.2.push(r),
                None => folds.push((id, label, vec![r])),
            },
            None => lines.push((ended(r), line(r))),
        }
    }
    for (_, label, runs) in folds {
        let newest = *runs.iter().max_by_key(|r| ended(r)).expect("a fold has a run");
        if runs.len() == 1 {
            lines.push((ended(newest), line(newest)));
            continue;
        }
        let oldest = runs.iter().map(|r| ended(r)).min().expect("a fold has a run");
        let reached = runs.iter().filter(|r| parents.contains(&r.id)).count();
        let reach = if reached == 0 { "none reached anyone".to_string() } else { format!("{reached} reached someone") };
        let latest = clip(&one_line(newest.summary.as_deref().unwrap_or("no report")), 140);
        lines.push((
            ended(newest),
            format!("{lead}{label}: {} quiet runs since {}, {reach}. Latest ({}): {latest}", runs.len(), local_day(oldest), local_time(ended(newest))),
        ));
    }
    lines
}

/// Lines newest first, at most `max`, then how many were left out.
fn newest_first(mut lines: Vec<(Ts, String)>, max: usize) -> Vec<String> {
    lines.sort_by(|a, b| b.0.cmp(&a.0));
    let left = lines.len().saturating_sub(max);
    let mut v: Vec<String> = lines.into_iter().take(max).map(|(_, l)| l).collect();
    if left > 0 {
        v.push(format!("({left} older {} left out)", if left == 1 { "entry" } else { "entries" }));
    }
    v
}

/// The note at the top of a run's first message: the time, what waits, what the employee did lately,
/// what their team finished, what else is running and which notes changed. With `since`, only what is
/// new since then. `None` when the run gets no note, and for a quiet routine with nothing new, so its
/// first message stays the same from run to run.
fn opening_note(s: &Snapshot, run: &Run) -> Option<String> {
    let o = s.opening.as_ref()?;
    let ceo = &s.company.ceo_name;
    let since = o.since.is_some();
    let new = |t: Ts| o.since.map_or(true, |since| t > since);
    let mut sections: Vec<(String, Vec<String>)> = vec![];

    let mut waiting = vec![];
    for c in o.checks.iter().filter(|c| c.due_at <= o.now && new(c.due_at)) {
        waiting.push(format!("Check due {}: {} (check id `{}`). `my_checks` has how.", local_day(c.due_at), clip(&one_line(&c.what), 300), c.id));
    }
    let later = o.checks.iter().filter(|c| c.due_at > o.now).count();
    if !since && later > 0 {
        waiting.push(if later == 1 { "1 more open check is not due yet.".to_string() } else { format!("{later} more open checks are not due yet.") });
    }
    for (at, message) in o.cards.iter().filter(|(at, _)| new(*at)) {
        waiting.push(format!("Card to {ceo} since {}, not answered yet: {}", local_time(*at), clip(&one_line(message), 300)));
    }
    for (r, who) in o.waiting_on.iter().filter(|(r, _)| new(r.created_at)) {
        let kind = r.kind.as_str();
        let kind = kind[..1].to_uppercase() + &kind[1..];
        let sent = r.delivery.as_ref().map_or(r.created_at, |d| d.sent_at);
        let due = r.due_at.map(|d| format!(", due {}", local_day(d))).unwrap_or_default();
        waiting.push(format!(
            "{kind} to {who}, sent {}{due}: {} (request id `{}`), {}",
            local_day(sent),
            clip(&one_line(&r.message), 300),
            r.id,
            watch_status(r)
        ));
    }
    for p in o.proposals.iter().filter(|p| new(p.created_at)) {
        waiting.push(format!(
            "Hire proposal for {}, {}, waiting on {ceo} since {} (proposal id `{}`)",
            p.draft.name,
            p.draft.title,
            local_day(p.created_at),
            p.id
        ));
    }
    sections.push((if since { "Waiting, new since your last turn here:" } else { "Waiting:" }.into(), waiting));

    let what = |r: &Run| match &r.kind {
        RunKind::Chat | RunKind::Answered { chat: true, .. } => format!("chat with {ceo}"),
        RunKind::Reply { label, task: false, .. } => format!("reply in your thread on '{label}'"),
        RunKind::Reply { label, task: true, .. } | RunKind::Task { label, .. } => format!("your task '{label}'"),
        k => kind_label(k),
    };
    let wrote = |r: &Run| if r.omega_wrote { "Omega Star".to_string() } else { ceo.clone() };
    let work = run_lines(o.work.iter(), "", &o.parents, |r| {
        let said = if r.kind.is_conversation() { format!(" {}: \"{}\" You:", wrote(r), clip(&one_line(&r.prompt), 160)) } else { String::new() };
        format!("{}, {}:{said} {}", local_time(ended(r)), what(r), result(r))
    });
    let title = if since {
        "What you did elsewhere since your last turn here, newest first."
    } else {
        "What you have been doing, last 7 days, newest first. Each line keeps how the report began and how it ended."
    };
    sections.push((title.into(), newest_first(work, WORK_LINES)));

    let mut team = vec![];
    let mut names: Vec<&str> = vec![];
    for (name, _) in &o.team {
        if !names.contains(&name.as_str()) {
            names.push(name);
        }
    }
    for name in names {
        let runs = o.team.iter().filter(|(n, _)| n == name).map(|(_, r)| r);
        team.extend(run_lines(runs, &format!("{name}, "), &o.parents, |r| format!("{name}, {} ({}): {}", kind_label(&r.kind), local_time(ended(r)), result(r))));
    }
    sections.push((format!("From your team since your last {}:", if since { "turn here" } else { "run" }), newest_first(team, TEAM_LINES)));

    let running = o
        .running
        .iter()
        .map(|r| {
            let said = if r.kind.is_conversation() {
                format!(": {} {} \"{}\"", wrote(r), if r.omega_wrote { "wrote" } else { "said" }, clip(&one_line(&r.prompt), 150))
            } else {
                String::new()
            };
            format!("{}, started {}{said}", what(r), local_time(r.started_at.unwrap_or(r.created_at)))
        })
        .collect();
    sections.push(("Also running now, in your other conversations. Do not repeat what they do outside, such as sending, posting or paying:".into(), running));

    let mut notes: Vec<String> = o.notes.iter().take(NOTE_LINES).map(|(file, at)| format!("`workspace/{file}` ({})", local_time(*at))).collect();
    if o.notes.len() > NOTE_LINES {
        notes.push(format!("+{} more", o.notes.len() - NOTE_LINES));
    }
    sections.push((format!("Your notes changed {}:", if since { "since then" } else { "this week" }), notes));

    let now = format!("Now: {}.", o.now.with_timezone(&chrono::Local).format("%A %-d %B %Y, %-I:%M %P"));
    let sections: Vec<String> = sections
        .into_iter()
        .filter(|(_, lines)| !lines.is_empty())
        .map(|(title, lines)| format!("{title}\n{}", lines.iter().map(|l| format!("- {l}")).collect::<Vec<_>>().join("\n")))
        .collect();
    if sections.is_empty() && since && matches!(run.kind, RunKind::Scheduled { quiet: true, .. }) {
        return None;
    }
    Some(std::iter::once(now).chain(sections).collect::<Vec<_>>().join("\n\n"))
}

/// How to answer in a conversation with the CEO outside the ongoing chat: a thread on a report, or a
/// task (`place` names which).
fn talk(s: &Snapshot, place: &str) -> String {
    format!(
        "Answer as you would in chat: do what they ask or answer what they ask, say plainly what you could not do, and \
keep your final message short. It is your next message in the {place}, not a report, and {ceo} reads it where they \
wrote. Anything they teach you becomes a skill edit now, not a promise: update the right skill before your reply ends \
and say in one line which skill changed. If this exchange exposed a gap or a correction, make the smallest skill edit \
that would have prevented it; otherwise change nothing. A decision only {ceo} can make goes in your message; call \
`notify` only when you must stop and wait for it.",
        ceo = s.company.ceo_name
    )
}

/// The procedure for an Organize run: from a pile of imported skills to one composable family.
fn organize_frame(s: &Snapshot) -> String {
    let e = &s.employee;
    let ceo = &s.company.ceo_name;
    let sep = std::path::MAIN_SEPARATOR;
    let skills = s.employee_dir.join("skills");
    let imports = s.employee_dir.join("imports");
    format!(
        "{ceo} imported skills from their own Claude Code into `{imports}`. They are copies and now yours: \
you will change and improve them, and they are not connected to {ceo}'s originals. Claude Code does not load \
`imports{sep}`, so none of them work until you organize them. Nobody is watching: work autonomously.\n\n\
This run organizes; it does not perform any procedure. Follow `omega-skill-craft` for every skill you write. \
You may restructure your whole family, imported and existing skills alike, but keep existing names unless a \
rename clearly helps (renames break schedules and anything else that calls a skill by name).\n\n\
1. **Read.** Read every imported skill in full (SKILL.md and every sibling file) and every skill in your family. \
Done when you can say, for each one, what it does, what triggers it, and which other skills or scripts it calls.\n\
2. **Design.** Group the skills by area. Your role skill `{slug}` routes to area routers `{slug}-<area>` where an \
area holds several procedures; each procedure is a leaf `{slug}-<area>-<task>` or, for a lone one, `{slug}-<task>`. \
An imported router (one that dispatches to others) becomes an area router. Names take at most two words after \
your slug: drop words the family already implies (`recurring-weekly-aws-patching` becomes `{slug}-weekly-patching`). \
Merge skills that overlap, fold near-empty ones into their parent, keep each rule in exactly one place, and push detail only some runs need \
into sibling files. Done when every imported and existing skill has a destination: a new name, merged into a \
named skill, or retired as redundant.\n\
3. **Move.** Use the `omega` tool `move_skill(from, to)` for every skill that keeps its own folder: it moves the \
whole folder with its scripts and files and sets the frontmatter name. For a merge, write the content into the \
target, then `retire_skill(name)` the source. You cannot delete files any other way. Done when `imports{sep}` is \
empty.\n\
4. **Rewrite.** Give every skill a description in the craft's shape. Replace every reference to an old skill \
name with the new one, and every path into an old folder (including `$HERMES_HOME`, `~/.claude/skills` or other \
absolute paths to a skill's own scripts) with the path under `{skills}`. Done when a search of `{skills}` for \
each old name and each old path finds nothing.\n\
5. **Link.** Your role skill lists every router and every leaf that is not behind a router, each with one line on \
when to reach for it, and its Granted skills section matches your handbook. Done when every skill in your family \
can be reached from `{slug}`.\n\
6. **Schedules.** If one of your schedules names an old skill, fix it with the `schedule` tool.\n\n\
Finish with a report: the new family as an indented tree, what you merged or retired and why, skills yours \
still call that are neither in your family nor granted, and anything outside your own skills and schedules that \
still uses an old name.",
        imports = imports.display(),
        skills = skills.display(),
        slug = e.slug,
    )
}

/// The procedure for a Tidy run: bring the employee's skills and notes back to the shape
/// `omega-skill-craft` describes, closing loose ends first. Small changes are theirs to make now;
/// changes that could lose something you taught them are proposed, and made in a second, approved run.
fn tidy_frame(s: &Snapshot, reason: TidyReason) -> String {
    let ceo = &s.company.ceo_name;
    let brief = s.tidy.as_ref();
    let mut b = String::new();
    let none = TidyBrief { findings: vec![], taught: vec![], plan: None };
    let brief = brief.unwrap_or(&none);

    b += "## What Omega Star measured\n\n";
    if brief.findings.is_empty() {
        b += "Nothing flagged.\n\n";
    } else {
        for f in &brief.findings {
            let at = match (&f.skill, &f.file) {
                (Some(sk), Some(file)) if file != "SKILL.md" => format!("`{sk}` / {file}"),
                (Some(sk), _) => format!("`{sk}`"),
                (None, Some(file)) => format!("`{file}`"),
                (None, None) => "your skills".to_string(),
            };
            b += &format!("- {} ({at}): {}\n", f.kind.label(), one_line(&f.detail));
        }
        b += "\nThese are reasons to look, not orders: a long runbook may be one procedure that belongs together.\n\n";
    }
    if reason != TidyReason::Approved {
        b += &format!("## What {ceo} told you since your last tidy\n\n");
        if brief.taught.is_empty() {
            b += "Nothing new.\n\n";
        } else {
            for (at, text) in &brief.taught {
                b += &format!("- {}: {}\n", at.with_timezone(&chrono::Local).format("%Y-%m-%d"), clip(&one_line(text), 500));
            }
            b += "\n";
        }
    }

    let steps = if reason == TidyReason::Approved {
        format!(
            "{ceo} approved the changes below, which an earlier tidy proposed. Make exactly those changes and nothing else: \
renames, splits, merges, retirements (`move_skill`, `retire_skill`), and pruning the rules they name. Keep every other rule \
you were taught. Done when each approved change is made, every reference to a renamed or retired skill is updated, and your \
role skill still reaches every skill.\n\n## The approved plan\n\n{}\n",
            brief.plan.as_deref().unwrap_or("(no plan was recorded)").trim()
        )
    } else {
        format!(
            "Work in this order.\n\n\
1. **Close loops.** For every open check that is due, and every line above that reads like an unfinished note, do the check \
now if a few tool calls can settle it: look, decide, then `check_done(id, result)` (or delete the note) and write what you \
found into the skill it belongs to, with today's date. Do not go hunting: a check that needs more than that stays a check. \
What only {ceo} can settle goes to them with `request` or, if it needs a decision, `notify`. \
What you cannot check yet becomes a `check_later` with a real date, and the prose note goes. Done when no skill or notes file \
says \"verify later\" without a tracked check behind it.\n\
2. **Confirm what you were taught.** For each thing {ceo} told you above, find the line in your skills that holds it. If it is \
missing, write it now, in the right skill, under `## Rules` or your role skill's `## Standing preferences`, one bullet, ending \
`(taught YYYY-MM-DD)`. Done when you can name, for each item, the skill and heading where it lives, or say why it is not worth \
keeping (a one-off instruction, or already replaced by a later one).\n\
3. **Tidy.** Bring your family back to the shape `omega-skill-craft` describes, starting with what Omega Star measured.\n\
   Do these yourself, now:\n\
   - Compact a diary or state file in `workspace/`: keep the open conditions and the last few days, and turn older entries into \
one line each. Keep every id, ticket key and outcome that a later run could need.\n\
   - Move dated facts out of a procedure into `facts.md` beside it, and point to it. Keep each fact's own date and end the line \
`(checked: never)`, or `(checked YYYY-MM-DD)` only for one you verified in this run. Do not go and verify the rest now: a fact \
is re-checked when a run next relies on it.\n\
   - Fix pointers: a skill that names one that does not exist, and a skill that nothing points to.\n\
   - Tighten a description that is long; push detail only some runs need into a sibling file behind a pointer that says when to open it.\n\
   - Keep each rule in one place: when you find it twice, keep the better one and point to it.\n\
   Do NOT do these. Describe each under Proposed in your report; {ceo} approves them and a follow-up run makes them:\n\
   - splitting a skill in two, merging skills, renaming or retiring one;\n\
   - removing or rewording any rule you were taught (under `## Rules`, `## Standing preferences`, or marked `(taught ...)`);\n\
   - changing what a schedule does.\n\
   Work worst first. A big family does not have to be finished in one run: say what you left under Could not resolve.\n\
   Done when everything you got to is either fixed, proposed, or explained as fine as it is.\n\n\
Finish with a report in three parts: **Done** (what changed, one line each), **Proposed** (each bigger change: what, why, what it \
fixes), and **Could not resolve** (what you left, and why). Omega Star flags your proposals to {ceo} itself, so do not \
call `notify` for them."
        )
    };

    format!(
        "This run tidies your own skills and notes. It does not perform any procedure and it does not investigate: read your \
skills and notes, use a tool only to settle an open check, and change nothing outside your skills, notes and checks. Nobody is \
watching: work autonomously. Follow `omega-skill-craft` for every skill you write, and do not rewrite what works: a change that \
is not needed is churn.\n\n{b}{steps}"
    )
}
