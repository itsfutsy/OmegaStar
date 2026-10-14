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
            "instructions": "Omega Star office tools: your recurring schedules, notifying the person you work for, moving and retiring your skills, your team (delegate, await reports), proposing hires, and the people you can ask for help. Omega Star runs your schedules even when no Claude session is open, and watches your requests to people for replies.",
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
        "ask_colleague" => office.ask_colleague(&run_id, &me, s("to"), s("question"), s("kind") == "task"),
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
        _ => Err(AppError::invalid(format!("unknown tool '{name}'"))),
    }
}

fn tools() -> Value {
    let obj = |props: Value, required: &[&str]| json!({ "type": "object", "properties": props, "required": required });
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
            "description": "Ask the person you work for for something only they can give: a decision, an approval, or a chore. Sends a desktop notification now and puts your message, with the answer built in, at the top of this run's report in their Inbox. Add choices for a decision (up to four buttons, mark the one you recommend), items for a list they approve one by one, or chore for something only they can do. Your message says what you recommend and the safe default you will follow if they dismiss it; going ahead is never the default for spending, public posts, customer messages, deleting data, new vendors or payment details. Their answer comes back to you in this conversation as a short fixed message. One card per run. Never for updates or routine results: those belong in your report.",
            "inputSchema": obj(json!({
                "message": { "type": "string", "description": "One or two sentences, written to them: what needs them, what you recommend, and the safe default if they dismiss it. Never a code or password." },
                "choices": {
                    "type": "array", "maxItems": 4,
                    "description": "2 to 4 buttons for one decision, e.g. [{\"label\": \"Not now\", \"recommended\": true}, {\"label\": \"Try A$5 a day\"}]. Mark exactly one recommended.",
                    "items": obj(json!({
                        "label": { "type": "string", "description": "A few words, e.g. 'Not now'." },
                        "recommended": { "type": "boolean" }
                    }), &["label"])
                },
                "items": {
                    "type": "array", "maxItems": 10,
                    "description": "Things to approve one by one, up to 10. Each gets a tick box and an optional note.",
                    "items": obj(json!({
                        "id": { "type": "string", "description": "A short id they and you can refer to, e.g. 'd1'." },
                        "text": { "type": "string", "description": "One line: what it is and anything that decides it, e.g. how old the post it answers is." },
                        "recommended": { "type": "boolean", "description": "true for the ones you would approve." }
                    }), &["id", "text"])
                },
                "itemVerb": { "type": "string", "description": "What a tick means, one word, e.g. 'Post'. Default 'Approve'." },
                "chore": { "type": "string", "description": "Something only they can do, as one of: 'chrome' (reconnect Claude in Chrome) or 'sign-in:<site>' (sign in to a site, e.g. 'sign-in:wise'). They press Done and every run waiting on it carries on. Your message names the one action, in plain words." }
            }), &["message"]),
        },
        {
            "name": "propose_changes",
            "description": "Only in a tidy run: record the bigger changes you want to make (splitting, merging, renaming or retiring a skill, or removing or rewording a rule you were taught) so the person you work for can approve them. Do not make those changes yourself. Call once with everything, written so a later run can carry it out without your report: for each change, the skills involved, exactly what moves or goes, and why.",
            "inputSchema": obj(json!({ "plan": { "type": "string", "description": "The proposed changes, one per paragraph." } }), &["plan"]),
        },
        {
            "name": "check_later",
            "description": "Set yourself a check: something you must find out or do later and cannot now (a change that needs a day to show up, a follow-up you promised). On its due date Omega Star starts a quiet run of yours with what and how as its instruction, whether or not anything else runs. Use it for anything you will do later, instead of writing 'I'll check' in a report or 'verify later' in a skill or notes file. Max 20 open at once.",
            "inputSchema": obj(json!({
                "what": { "type": "string", "description": "What you will find out, in one sentence." },
                "how": { "type": "string", "description": "Where or how you will look: the command, the page, who to ask. Specific enough that a future run can do it cold." },
                "inDays": { "type": "number", "description": "Days from now until it is worth checking (0 to 180, default 7)." }
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
            "description": "List your direct reports (agents) with title, objective, presence and their last run summary.",
            "inputSchema": obj(json!({}), &[]),
        },
        {
            "name": "delegate",
            "description": "Hand one direct report a self-contained task. Returns {runId} immediately. Their report always comes back to you: wait with await_reports only if it should take a few minutes; otherwise finish, and a new run brings it to you.",
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
            "description": "Ask another agent a question, or hand them a task, that falls in their area, not yours. They work on it in a run of their own. Returns {runId} immediately. The answer or report always comes back to you: wait with await_reports only if it should take a few minutes and you need it now; otherwise finish without it, and a new run brings it to you. Only when it genuinely helps: never for what you can find or do yourself.",
            "inputSchema": obj(json!({
                "to": { "type": "string", "description": "The colleague's name, slug or id." },
                "kind": { "type": "string", "enum": ["question", "task"], "description": "question (default): they answer and change nothing. task: they do the work, e.g. set something up once it is approved, or act on news you pass on." },
                "question": { "type": "string", "description": "The question or task, self-contained: they see nothing of your run. For a task, what done looks like and who approved it." }
            }), &["to", "question"]),
        },
        {
            "name": "await_reports",
            "description": "Wait (up to 4 minutes) for runs you delegated or questions you asked to finish, and return each run's state and report or answer. Use it only for hand-offs that take a few minutes: anything still going comes back to you in a new run when it finishes.",
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
