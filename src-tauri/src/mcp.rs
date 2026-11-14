//! A minimal streamable-HTTP MCP server on 127.0.0.1: JSON responses only, no sessions, no SSE.
//! The path token identifies the calling run (and so the calling employee).

use crate::log::log;
use crate::model::AppError;
use crate::office::Office;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};

pub async fn serve(listener: tokio::net::TcpListener, office: Office) {
    let app = Router::new()
        .route("/mcp/{token}", post(handle).get(|| async { StatusCode::METHOD_NOT_ALLOWED }))
        .with_state(office);
    if let Err(e) = axum::serve(listener, app).await {
        log!("MCP server stopped: {e}");
    }
}

async fn handle(State(office): State<Office>, Path(token): Path<String>, Json(msg): Json<Value>) -> Response {
    match respond(&office, &token, msg).await {
        Some(r) => Json(r).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// `None` for notifications and responses, which get no reply.
async fn respond(office: &Office, token: &str, msg: Value) -> Option<Value> {
    let id = msg.get("id").cloned().filter(|i| !i.is_null())?;
    let method = msg.get("method").and_then(|m| m.as_str())?;
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": params.get("protocolVersion").cloned().unwrap_or(json!("2025-06-18")),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "omega", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "Omega Star office tools: your recurring schedules, putting decisions to the person you work for, showing them a page in your message, moving and retiring your skills, your team (delegate, await reports), proposing hires, and the people you can ask for help. Omega Star runs your schedules even when no Claude session is open, and watches your requests to people for replies.",
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => {
            let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let (text, is_error) = match call(office, token, name, &args).await {
                Ok(v) => (serde_json::to_string_pretty(&v).unwrap_or_default(), false),
                Err(e) => (e.message, true),
            };
            Ok(json!({ "content": [{ "type": "text", "text": text }], "isError": is_error }))
        }
        _ => Err(json!({ "code": -32601, "message": format!("method not found: {method}") })),
    };
    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
    })
}

async fn call(office: &Office, token: &str, name: &str, args: &Value) -> Result<Value, AppError> {
    let (run_id, me) = office.caller(token).ok_or_else(|| AppError::invalid("this run has ended; its office tools are closed"))?;
    let s = |k: &str| args.get(k).and_then(|v| v.as_str()).unwrap_or("");
    match name {
        "team" => office.team(&me),
        "delegate" => office.delegate(&run_id, &me, s("to"), s("task")),
        "colleagues" => office.colleagues(&me),
        "ask_colleague" => office.ask_colleague(&run_id, &me, s("to"), s("question"), s("kind")),
        "await_reports" => {
            let ids: Vec<String> = args
                .get("runIds")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();
            if ids.is_empty() {
                return Err(AppError::invalid("runIds is required"));
            }
            office.await_reports(&me, &ids).await
        }
        "people" => office.people_list(),
        "request" => office.request_tool(&run_id, &me, args),
        "my_requests" => office.my_requests(&me),
        "update_request" => office.update_request_tool(&run_id, &me, args),
        "sent" => office.sent_tool(&run_id, &me, args),
        "check_result" => office.check_result_tool(&run_id, args),
        "schedules" => office.my_schedules(&me),
        "schedule" => office.schedule_tool(&me, args),
        "unschedule" => office.unschedule_tool(&me, s("id")),
        "notify" => office.notify_tool(&run_id, &me, args),
        "close_ask" => office.close_ask_tool(&me, s("key"), s("reason")),
        "propose_company_line" => office.propose_company_line_tool(&run_id, &me, args),
        "report_number" => office.report_number_tool(&run_id, args),
        "catalog" => office.catalog_tool(),
        "propose_hire" => office.propose_hire_tool(&run_id, &me, args),
        "my_proposals" => office.my_proposals(&me),
        "withdraw_proposal" => office.withdraw_proposal_tool(&me, s("id")),
        "propose_changes" => office.propose_changes_tool(&run_id, s("plan")),
        "check_later" => office.check_later_tool(&run_id, &me, args),
        "my_checks" => office.my_checks(&me),
        "check_done" => office.check_done_tool(&me, s("id"), s("result")),
        "move_skill" => office.move_skill_tool(&me, s("from"), s("to")),
        "retire_skill" => office.retire_skill_tool(&run_id, &me, s("name")),
        "show" => office.show_tool(&run_id, args),
        "open" => office.open_tool(&run_id, &me, s("target")),
        "name_thread" => office.name_thread_tool(&run_id, s("title")),
        "request_access" => office.request_access_tool(&run_id, &me, args),
        _ => Err(AppError::invalid(format!("unknown tool '{name}'"))),
    }
}

fn tools() -> Value {
    let obj = |props: Value, required: &[&str]| json!({ "type": "object", "properties": props, "required": required });
    // Notify's schema and its task are built apart, so each stays within json!'s recursion limit.
    let mut task = obj(
        json!({
            "steps": { "type": "string", "description": "What to do, in order, at most 1000 characters." },
            "url": { "type": "string", "description": "Optional. The page to do it on. http or https only." }
        }),
        &["steps"],
    );
    task["description"] = "A to-do only they can do, e.g. approving a new payee in the bank's site. They press Done or Can't.".into();
    let notify = obj(
        json!({
            "message": { "type": "string", "description": "The question, at most 80 characters, e.g. 'Merge PR #12, the pricing page?'. For news, the headline. Never a code or password." },
            "why": { "type": "string", "description": "Optional. One line, at most 140 characters: what it changes or costs, e.g. 'Our busiest page. Built and reviewed.'" },
            "key": { "type": "string", "description": "What the card is about, e.g. 'pr:12' or 'spring-sale'. Required for all but a chore. Notify again with it to change your card; close it with close_ask once it is settled without them." },
            "file": { "type": "string", "description": "Optional, with any card but a chore. The markdown or text file (.md or .txt, up to 200 KB) it is about, e.g. 'workspace/drafts/supplier-reply.md', in your folder or one of your workspaces. The card shows it as it is when they read it, so finish it first." },
            "link": { "type": "string", "description": "Optional. The web page it is about, e.g. the pull request. http or https only." },
            "images": { "type": "array", "maxItems": 3, "items": { "type": "string" }, "description": "Optional. Up to 3 png, jpg, gif or webp files in your folder or one of your workspaces that show what it is about, e.g. a page before and after." },
            "choices": {
                "type": "array", "maxItems": 4,
                "description": "1 to 4 ways to go ahead with one decision, e.g. [{\"label\": \"Merge\", \"recommended\": true}]. Later and No are built in. Mark exactly one recommended.",
                "items": obj(json!({
                    "label": { "type": "string", "description": "A few words, e.g. 'Merge'." },
                    "recommended": { "type": "boolean" }
                }), &["label"])
            },
            "items": {
                "type": "array", "maxItems": 10,
                "description": "Things to approve one by one, up to 10. Each gets a tick box and an optional note.",
                "items": obj(json!({
                    "id": { "type": "string", "description": "A short id they and you can refer to, e.g. 'd1' or the name of who it answers." },
                    "text": { "type": "string", "description": "Exactly what would go out, in full, at most 2000 characters, e.g. the post. Put what it is about in about, not here." },
                    "about": { "type": "string", "description": "Optional. One line, at most 200 characters, on what it is about and anything that decides it, shown above text, e.g. the post it answers and how old it is." },
                    "recommended": { "type": "boolean", "description": "true for the ones you would approve. They start ticked." }
                }), &["id", "text"])
            },
            "itemVerb": { "type": "string", "description": "What a tick means, one word, e.g. 'Post'. Default 'Approve'." },
            "chore": { "type": "string", "description": "Something only they can do, as one of: 'chrome' (reconnect Claude in Chrome) or 'sign-in:<site>' (sign in to a site, e.g. 'sign-in:acme'). They press Done and every run waiting on it carries on. Needs no key." },
            "url": { "type": "string", "description": "Optional, with chore: the page it is done on, e.g. the site's sign-in page, which opens in their browser. http or https only." },
            "task": task,
            "question": { "type": "boolean", "description": "true: a fact only they know, e.g. which account a bill is paid from. They answer in their own words, so never guess options for it." },
            "news": { "type": "boolean", "description": "true: something material they should know that needs no decision, e.g. spend started or stopped, something shipped, a number crossed a line. A promise to tell them is kept with news." }
        }),
        &["message"],
    );
    json!([
        {
            "name": "schedules",
            "description": "List your recurring schedules (id, label, cron, instruction, quiet, enabled, next fire) and the current local time.",
            "inputSchema": obj(json!({}), &[]),
        },
        {
            "name": "schedule",
            "description": "Create a recurring schedule for yourself, or change one by id (only the fields you pass change). Omega Star starts a run with the instruction at every cron slot, whether or not any Claude session is open. Check schedules() first so you never create a duplicate. If Omega Star was closed, asleep or waiting at a slot, the routine runs once as soon as it can.",
            "inputSchema": obj(json!({
                "id": { "type": "string", "description": "Omit to create; pass to change an existing schedule." },
                "label": { "type": "string", "description": "Short name, e.g. 'Inbox watch'." },
                "cron": { "type": "string", "description": "Five-field cron in this computer's local time, e.g. '*/10 * * * *' or '0 8 * * 1-5'." },
                "instruction": { "type": "string", "description": "What you do each time it fires. Keep it short; the how lives in your skills." },
                "quiet": { "type": "boolean", "description": "true: a run's report reaches anyone only if the run calls notify or fails. Use for frequent watches. Default false: every report goes where your handbook's Your report section says." },
                "enabled": { "type": "boolean" },
                "activeHours": { "type": "boolean", "description": "true: it runs only inside the office's business hours, and slots outside them are skipped, not made up later. Use for routines that only matter while people are at work. Default false." },
                "model": { "type": "string", "description": "Optional. The model this routine runs on, e.g. 'haiku' for a frequent, simple check. Leave it out to use your usual model; pass 'default' to go back to it." },
                "effort": { "type": "string", "enum": ["low", "medium", "high", "xhigh", "max", "default"], "description": "Optional. How hard this routine thinks: 'low' for a quick check, 'high' or more for research. 'default' goes back to your usual effort." }
            }), &[]),
        },
        {
            "name": "unschedule",
            "description": "Delete one of your schedules by id.",
            "inputSchema": obj(json!({ "id": { "type": "string" } }), &["id"]),
        },
        {
            "name": "notify",
            "description": "Put one thing in front of the person you work for, as a card in their Inbox with the answer built in, and send a desktop notification. The card is all they read, so it stands alone: the question in message, one line of why, and one of choices (1 to 4 ways to go ahead, mark the one you recommend), items (a list they approve one by one), chore (reconnect Chrome or sign in to a site), task (a to-do only they can do), question (a fact only they know, answered in their own words) or news (something material they should know that needs no decision). One thing per card; a run may raise up to five. Name what it is about in key: the same key from you or a colleague is one item in their Inbox, and notifying again with your key changes your card in place with no new notification, which is how you edit a card; a list raised with the key of your open list adds to it. When it is about something you wrote, such as a draft to approve, pass its path in file, and for something online, such as a pull request, its link, so they read it on the card; leave paths and links out of message and why. Going ahead is never the answer you assume for spending, public posts, customer messages, deleting data, new vendors or payment details. Their answers come back to you in this conversation as one short fixed message, once every card of the run is answered. Choices and items cards have Later and No built in, and tasks and questions Later, so never add them: Later is not a no, and the card comes back to them by itself on the day they pick, then a second Later parks it in their Parked list; No means do not do it and do not ask again. News stays in their Inbox until they mark it seen, and nothing comes back. Never for routine updates or results: those go in your report.",
            "inputSchema": notify,
        },
        {
            "name": "show",
            "description": "Show a page in your message: a chart, table, comparison, diagram, timeline or mockup that says more than words. Write one complete, self-contained HTML document with inline <style> and <script>. It cannot load anything from the internet (no CDNs, web fonts or remote images), so draw charts with inline SVG or canvas. Local images written as full paths (src=\"/Users/me/shot.png\", url(/Users/me/bg.webp) or a string in a script) are embedded for you. Omega Star shows it above your message, as wide as the message column (450 to 700 pixels, so keep the layout fluid) and as tall as the page. Leave the page background transparent, and let the content set the height: never use vh units or height: 100%. Omega Star's look comes as CSS variables that follow light and dark mode: --ink (text), --ink-2, --muted and --faint (quieter text), --canvas, --surface, --raised and --sunken (backgrounds), --line and --line-strong (borders), --live (the one blue), --ok, --warn and --err (each with a -soft background), --font-ui, --font-mono, --r-card and --r-inner (corner radii). Then write your message without repeating what the page shows.",
            "inputSchema": obj(json!({
                "title": { "type": "string", "description": "What the page shows, at most 80 characters, e.g. 'Signups by week'." },
                "html": { "type": "string", "description": "The page, up to 2 MB." }
            }), &["title", "html"]),
        },
        {
            "name": "open",
            "description": "Open something on the screen of the person you work for, when they ask to see it: a file in its app, a folder in a window, or a web page in their browser. It works only in a conversation they are in; anywhere else, put the link on your card instead, such as a sign-in page in the chore's url. A file that could run a program is shown in its folder instead. Pass a full path in your folder, one of your workspaces or the folder that holds the company page, or an https link. Returns whether it opened.",
            "inputSchema": obj(json!({
                "target": { "type": "string", "description": "The full path of a file or folder, or an https link, e.g. 'https://acme.example/sign-in'." }
            }), &["target"]),
        },
        {
            "name": "name_thread",
            "description": "Name the conversation or thread this run is in, in a few words, so the person you work for can find it again. It replaces the name Omega Star gave it from its first message or its kind. Name a new conversation once you know what it is about; rename it only when its subject changes.",
            "inputSchema": obj(json!({
                "title": { "type": "string", "description": "At most 60 characters, e.g. 'Spring sale emails'." }
            }), &["title"]),
        },
        {
            "name": "request_access",
            "description": "Ask the person you work for to let you use a command or tool you were blocked on, or work in a folder outside your own. Use it instead of describing settings steps for them. Ask for the narrowest rule that does the job: Bash(npm test:*) rather than Bash(npm:*), never Bash(*) or *. When the run ends they get one card and allow it all in one click, once or always, and this conversation starts again with it allowed. Do not wait for it: finish what you can and say what is waiting.",
            "inputSchema": obj(json!({
                "rules": { "type": "array", "items": { "type": "string" }, "description": "Permission rules, e.g. Bash(git push:*) or mcp__linear__create_issue." },
                "folders": { "type": "array", "items": { "type": "string" }, "description": "Full paths of folders to work in, e.g. /Users/you/dev/site. Must already exist, inside the home folder." },
                "why": { "type": "string", "description": "One line, in plain words, of what it is for. It is what they read on the card." }
            }), &["why"]),
        },
        {
            "name": "close_ask",
            "description": "Close your open card about key: it was settled without the person you work for (merged, paid, no longer needed). It leaves their Inbox. Never use it to edit a card: notify again with the same key instead.",
            "inputSchema": obj(json!({
                "key": { "type": "string", "description": "The key you gave the card, e.g. 'pr:12'." },
                "reason": { "type": "string", "description": "One line on how it was settled." }
            }), &["key", "reason"]),
        },
        {
            "name": "propose_company_line",
            "description": "Propose one line for the company page, which every employee reads at the start of every run: something true for everyone, such as a tool or account you all use, how the person you work for likes to work, or a decision they made. It goes to them on a card, from any run, and when they tap Add it, Omega Star adds it as a bullet under that section. Use it whenever they teach you something like that, instead of changing the page yourself.",
            "inputSchema": obj(json!({
                "section": { "type": "string", "description": "The page's heading it goes under, e.g. 'Who to ask' or 'How things work here'. A heading the page lacks is added." },
                "line": { "type": "string", "description": "One fact, at most 300 characters, e.g. 'Refunds over 100 go to Bo.'" },
                "why": { "type": "string", "description": "One line, at most 140 characters: why everyone should know it." }
            }), &["section", "line", "why"]),
        },
        {
            "name": "report_number",
            "description": "Only in your weekly review: record the number you answer for, for the team's scoreboard. Call once, after you measured it.",
            "inputSchema": obj(json!({
                "metric": { "type": "string", "description": "What you count, at most 40 characters, e.g. 'Open support tickets'." },
                "value": { "type": "string", "description": "This week's value, at most 20 characters, e.g. '0' or '12'." },
                "change": { "type": "string", "description": "Against last week, at most 30 characters, e.g. '+3%', 'down 1' or 'first count'." },
                "note": { "type": "string", "description": "Optional. What explains it, at most 100 characters." }
            }), &["metric", "value", "change"]),
        },
        {
            "name": "propose_changes",
            "description": "Only in a tidy run: record the bigger changes you want to make (splitting, merging, renaming or retiring a skill, or removing or rewording a rule you were taught) so the person you work for can approve them. Do not make those changes yourself. Call once with everything, written so a later run can carry it out without your report: for each change, the skills involved, exactly what moves or goes, and why.",
            "inputSchema": obj(json!({ "plan": { "type": "string", "description": "The proposed changes, one per paragraph." } }), &["plan"]),
        },
        {
            "name": "check_later",
            "description": "Set yourself a check: something you must find out or do later and cannot now (a change that needs a day to show up, a follow-up you promised). On its due date Omega Star starts a quiet run of yours with what and how as its instruction, whether or not anything else runs; set in a conversation that is still open, it comes back into that conversation. Do what you can now: use it only for what must wait for time to pass, instead of writing 'I'll check' in a report or 'verify later' in a skill or notes file. Max 20 open at once.",
            "inputSchema": obj(json!({
                "what": { "type": "string", "description": "What you will find out, in one sentence." },
                "how": { "type": "string", "description": "Where or how you will look: the command, the page, who to ask. Specific enough that a future run can do it cold." },
                "inDays": { "type": "number", "description": "Days from now until it is worth checking (0.25, six hours, to 180; default 7). Checks are at least six hours away." }
            }), &["what", "how"]),
        },
        {
            "name": "my_checks",
            "description": "Your open checks with what, how, due date and whether they are overdue.",
            "inputSchema": obj(json!({}), &[]),
        },
        {
            "name": "check_done",
            "description": "Close one of your checks with what you found. Then write the finding into the skill it belongs to, dated, so it is not lost.",
            "inputSchema": obj(json!({
                "id": { "type": "string" },
                "result": { "type": "string", "description": "What you found. If you could not find out, say why and what would let you." }
            }), &["id", "result"]),
        },
        {
            "name": "move_skill",
            "description": "Move or rename one of your skills, or a skill waiting in imports/, to skills/<to>: the whole folder with its scripts and files. Sets the frontmatter name; you update every reference to the old name. Your role skill and omega-skill-craft never move.",
            "inputSchema": obj(json!({
                "from": { "type": "string", "description": "Current folder name, in imports/ or in your skills." },
                "to": { "type": "string", "description": "New name: your slug, a hyphen, then lowercase words joined by hyphens." }
            }), &["from", "to"]),
        },
        {
            "name": "retire_skill",
            "description": "Take one of your skills, or a skill waiting in imports/, out of use: for a skill merged into another, or one that is redundant. It is archived with this run, never destroyed, and the change can be reverted.",
            "inputSchema": obj(json!({ "name": { "type": "string" } }), &["name"]),
        },
        {
            "name": "team",
            "description": "List your direct reports (agents) with title, objective, presence, their last run summary and how many tasks you handed them today (tasksToday, at most tasksPerDay).",
            "inputSchema": obj(json!({}), &[]),
        },
        {
            "name": "delegate",
            "description": "Hand one direct report a self-contained task, at most 4 a day to each. Returns {runId} immediately. Their report always comes back to you: wait with await_reports only if it should take a few minutes; otherwise finish. Handed over in a conversation the person you work for started, it comes back as its next turn; any other report waits for your next run, or comes with your team's other reports in one run once 30 minutes pass without a new one. When a chain of hand-offs grows long, Omega Star pauses the task and sends it by itself later: never send it again.",
            "inputSchema": obj(json!({
                "to": { "type": "string", "description": "The report's name, slug or id." },
                "task": { "type": "string", "description": "What to do, why, and what done looks like." }
            }), &["to", "task"]),
        },
        {
            "name": "colleagues",
            "description": "List every other agent in the company with title, objective, description, who they report to and presence.",
            "inputSchema": obj(json!({}), &[]),
        },
        {
            "name": "ask_colleague",
            "description": "Ask another agent a question, or hand them a task, that falls in their area, not yours. They work on it in a run of their own. Returns {runId} immediately. The answer or report always comes back to you: wait with await_reports only if it should take a few minutes and you need it now; otherwise finish without it, and a new run brings it to you. If they have not started your earlier question or task yet, this one is added to it. With kind info, it leaves them news instead: no run, nothing comes back, and they read it at the top of their next run. When a chain of hand-offs grows long, Omega Star pauses the next one and sends it by itself later: never send it again. Only when it genuinely helps: never for what you can find or do yourself.",
            "inputSchema": obj(json!({
                "to": { "type": "string", "description": "The colleague's name, slug or id, or 'everyone' with kind info to tell every colleague." },
                "kind": { "type": "string", "enum": ["question", "task", "info"], "description": "question (default): they answer and change nothing. task: they do the work, e.g. set something up once it is approved, or act on news you pass on. info: news that needs no answer and no work; it starts no run. When you raise a card in work a colleague handed you, its outcome already reaches them, so never send a separate notice about it." },
                "question": { "type": "string", "description": "The question, task or news, self-contained: they see nothing of your run. For a task, what done looks like and who approved it." }
            }), &["to", "question"]),
        },
        {
            "name": "await_reports",
            "description": "Wait (up to 4 minutes) for runs you delegated or questions you asked to finish, and return each run's state and report or answer. Use it only for hand-offs that take a few minutes: anything still going comes back to you when it finishes.",
            "inputSchema": obj(json!({ "runIds": { "type": "array", "items": { "type": "string" } } }), &["runIds"]),
        },
        {
            "name": "catalog",
            "description": "List every skill in the company catalog (name, description, source) that a new hire could be given. Use it to choose the skills for propose_hire.",
            "inputSchema": obj(json!({}), &[]),
        },
        {
            "name": "propose_hire",
            "description": "Propose a new agent for your area. The person you work for reviews it in their Inbox, may change it, and approves (which hires them) or declines. Their decision comes back to you as a new run. Only when they ask what hires you need, or a recurring gap in your area needs a dedicated role; never for a single task. Returns the proposal.",
            "inputSchema": obj(json!({
                "name": { "type": "string", "description": "A first name for the new hire, e.g. 'Maya'." },
                "title": { "type": "string", "description": "The role, e.g. 'SEO Specialist'." },
                "description": { "type": "string", "description": "What they look after, in a sentence or two." },
                "objective": { "type": "string", "description": "What success looks like, in one sentence. Their role skill opens with it." },
                "reason": { "type": "string", "description": "Why the company needs this role now: the gap, what it costs today, and what they would own. The person you work for decides on this." },
                "skills": { "type": "array", "items": { "type": "string" }, "description": "Names of catalog skills they need (see catalog). Skills not in the catalog are listed as missing." },
                "reportsTo": { "type": "string", "description": "Omit to have them report to you, or name one of your direct reports." }
            }), &["name", "title", "objective", "reason"]),
        },
        {
            "name": "my_proposals",
            "description": "Your hire proposals, newest first, with state (open, approved, declined, withdrawn) and the note from the person you work for.",
            "inputSchema": obj(json!({}), &[]),
        },
        {
            "name": "withdraw_proposal",
            "description": "Take back one of your open hire proposals, e.g. to propose it differently.",
            "inputSchema": obj(json!({ "id": { "type": "string" } }), &["id"]),
        },
        {
            "name": "people",
            "description": "List the real people in the org with title, strengths, responsibilities and channels (each with when to use it).",
            "inputSchema": obj(json!({}), &[]),
        },
        {
            "name": "request",
            "description": "Record a question, a task or an FYI for a person before you send it. Returns the request and the chosen channel's details. Then deliver it through that channel if you can and call sent right after sending; otherwise the person you work for relays it from Omega Star. Their reply comes back to you as a follow-up run.",
            "inputSchema": obj(json!({
                "person": { "type": "string", "description": "The person's name or id." },
                "kind": { "type": "string", "enum": ["question", "task", "info"], "description": "info: an FYI that needs no reply. It is never watched or followed up." },
                "message": { "type": "string", "description": "One clear ask: what you need and by when." },
                "channelId": { "type": "string", "description": "Id of the person's channel whose guidance fits this ask." },
                "dueAt": { "type": "string", "description": "ISO date or date-time." }
            }), &["person", "kind", "message"]),
        },
        {
            "name": "sent",
            "description": "Record that you delivered one of your requests, right after sending it: pass the tool you sent it with and exactly what that tool returned. Omega Star then watches for the person's reply, nudges them once when it is due, and brings the reply back to you as a follow-up run. Do not watch for it yourself.",
            "inputSchema": obj(json!({
                "requestId": { "type": "string" },
                "tool": { "type": "string", "description": "The tool you sent it with, e.g. mcp__ms365-local__send-chat-message." },
                "result": { "type": "string", "description": "Exactly what that tool returned." }
            }), &["requestId", "tool", "result"]),
        },
        {
            "name": "check_result",
            "description": "Only in a reply check or a nudge that Omega Star started: record what you found, once. answered: it answers the question. done: they did the task. askedBack: they asked something that must be answered first. acknowledged: they saw it and will get to it. noReply: nothing there is about this ask.",
            "inputSchema": obj(json!({
                "requestId": { "type": "string" },
                "verdict": { "type": "string", "enum": ["answered", "done", "askedBack", "acknowledged", "noReply"] },
                "response": { "type": "string", "description": "Their words, as they wrote them." },
                "note": { "type": "string", "description": "One line on what they said about the ask." }
            }), &["requestId", "verdict"]),
        },
        {
            "name": "my_requests",
            "description": "Your requests to people, newest first, with state and any response.",
            "inputSchema": obj(json!({}), &[]),
        },
        {
            "name": "update_request",
            "description": "Record a person's reply you saw yourself (answered/done, with the response), or cancel a request you no longer need. No follow-up run comes for a reply you record: carry on with the work in this run.",
            "inputSchema": obj(json!({
                "id": { "type": "string" },
                "state": { "type": "string", "enum": ["answered", "done", "cancelled"] },
                "response": { "type": "string" }
            }), &["id", "state"]),
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Every string in `v`: the tools' names, descriptions and choices.
    fn strings(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => out.push(s.clone()),
            Value::Array(a) => a.iter().for_each(|x| strings(x, out)),
            Value::Object(o) => o.values().for_each(|x| strings(x, out)),
            _ => {}
        }
    }

    fn description(tool: &str) -> String {
        let tools = tools();
        let t = tools.as_array().unwrap().iter().find(|t| t["name"] == tool).unwrap();
        t["description"].as_str().unwrap().to_string()
    }

    #[test]
    fn no_tool_text_has_an_em_dash_or_the_old_wording() {
        let mut texts = vec![];
        strings(&tools(), &mut texts);
        assert!(texts.len() > 100, "every tool and argument: {}", texts.len());
        for t in &texts {
            assert!(!t.contains('\u{2014}'), "{t}");
            assert!(!t.to_lowercase().contains("anything you will do later"), "{t}");
        }
        assert!(description("check_later").contains("Do what you can now: use it only for what must wait for time to pass"));
        assert!(description("open").contains("It works only in a conversation they are in; anywhere else, put the link on your card instead, such as a sign-in page in the chore's url."));
    }

    #[test]
    fn every_listed_tool_is_dispatched() {
        let tools = tools();
        let names: Vec<&str> = tools.as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        let listed: BTreeSet<&str> = names.iter().copied().collect();
        assert_eq!(listed.len(), names.len(), "a tool is listed twice: {names:?}");
        let source = include_str!("mcp.rs");
        let call = &source[source.find("async fn call(").unwrap()..source.find("fn tools()").unwrap()];
        let dispatched: BTreeSet<&str> = call.lines().filter_map(|l| l.trim().strip_prefix('"')?.split_once("\" =>").map(|(name, _)| name)).collect();
        assert_eq!(listed, dispatched);
    }
}
