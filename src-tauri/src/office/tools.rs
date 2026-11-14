//! The office tools employees call over MCP (dispatched from `mcp.rs`).

use super::*;
use super::handoffs::{day_start, TASKS_PER_DAY};
use crate::cards;

/// How many cards one run may have waiting on you.
const CARDS_PER_RUN: usize = 5;
/// The longest name `name_thread` gives a thread.
const TITLE_CHARS: usize = 60;
/// A card raised within this long after its employee closed one about the same thing is an edit: no new notification.
const QUIET_AFTER_CLOSE: chrono::Duration = chrono::Duration::minutes(15);
/// A card raised in a conversation you sent a message in within this long shows there, with no notification.
const LIVE: chrono::Duration = chrono::Duration::minutes(15);

impl Office {
    /// The live run a token belongs to, and its employee.
    pub fn caller(&self, token: &str) -> Option<(String, Employee)> {
        let run_id = lock(&self.0.active).iter().find(|(_, a)| a.token == token).map(|(id, _)| id.clone())?;
        let db = self.db();
        let run = db.run(&run_id).ok()??.run;
        let e = db.employee(&run.employee_id).ok()??;
        Some((run_id, e))
    }

    pub fn team(&self, caller: &Employee) -> Result<Value> {
        let db = self.db();
        let now = now();
        let today = day_start(now);
        let reports: Vec<Value> = db
            .employees()?
            .iter()
            .filter(|e| e.reports_to(&caller.id))
            .map(|e| {
                let v = Self::presence(&db, e, now, false)?;
                Ok(json!({
                    "id": e.id, "name": e.name, "slug": e.slug, "title": e.title, "objective": e.objective,
                    "presence": v.presence,
                    "lastSummary": v.last_run.and_then(|r| r.summary),
                    "tasksToday": db.delegated_since(&caller.id, &e.id, today)?,
                }))
            })
            .collect::<Result<_>>()?;
        Ok(json!({ "reports": reports, "tasksPerDay": TASKS_PER_DAY }))
    }

    /// Every other agent, with what they are for, who they report to and whether they are free.
    pub fn colleagues(&self, caller: &Employee) -> Result<Value> {
        let db = self.db();
        let now = now();
        let employees = db.employees()?;
        let people = db.people()?;
        let manager_name = |id: &Option<String>| {
            id.as_ref().and_then(|m| {
                employees.iter().find(|x| &x.id == m).map(|x| x.name.clone()).or_else(|| people.iter().find(|p| &p.id == m).map(|p| p.name.clone()))
            })
        };
        let list: Vec<Value> = employees
            .iter()
            .filter(|e| e.id != caller.id)
            .map(|e| {
                let v = Self::presence(&db, e, now, false)?;
                Ok(json!({
                    "id": e.id, "name": e.name, "slug": e.slug, "title": e.title,
                    "objective": e.objective, "description": e.description,
                    "reportsTo": manager_name(&e.manager_id),
                    "presence": v.presence,
                }))
            })
            .collect::<Result<_>>()?;
        Ok(json!({ "colleagues": list }))
    }

    pub fn people_list(&self) -> Result<Value> {
        let people: Vec<Value> = self
            .db()
            .people()?
            .iter()
            .map(|p| {
                json!({
                    "id": p.id, "name": p.name, "title": p.title, "description": p.description,
                    "strengths": p.strengths, "responsibilities": p.responsibilities,
                    "channels": p.channels, "notes": p.notes,
                })
            })
            .collect();
        Ok(json!({ "people": people }))
    }

    pub fn request_tool(&self, caller_run: &str, caller: &Employee, args: &Value) -> Result<Value> {
        let (req, person) = self.create_request(caller_run, caller, args)?;
        let channel = req.channel_id.as_ref().and_then(|c| person.channels.iter().find(|x| &x.id == c));
        let relay = ceo_of(&self.db())?;
        let then = "then call `sent` with the request id, the tool you sent it with and exactly what that tool returned";
        let hint = match channel {
            Some(c) => format!(
                "Recorded. Now deliver it: {}. Use a connector or skill you have for that channel, {then}. If you cannot reach it, say so in your report and {relay} will relay it from Omega Star.",
                claude::channel_line(c)
            ),
            None => format!("Recorded without a channel. Deliver it through whichever of their channels fits the ask if you can reach one, {then}; otherwise say so in your report and {relay} will relay it from Omega Star."),
        };
        Ok(json!({
            "request": req,
            "person": { "id": person.id, "name": person.name, "title": person.title, "channels": person.channels, "notes": person.notes },
            "channel": channel,
            "hint": hint,
        }))
    }

    pub fn my_requests(&self, caller: &Employee) -> Result<Value> {
        let watch = self.settings()?.watch;
        let mut reqs = self.db().requests(&RequestFilter { employee_id: Some(caller.id.clone()), ..Default::default() })?;
        reqs.truncate(50);
        for r in &mut reqs {
            Self::present_with(&watch, r);
        }
        Ok(json!({ "requests": reqs }))
    }

    /// Records that the caller delivered one of its requests, from what the send tool returned.
    pub fn sent_tool(&self, caller_run: &str, caller: &Employee, args: &Value) -> Result<Value> {
        let text = |k: &str| args.get(k).and_then(|v| v.as_str()).map(str::trim).unwrap_or("").to_string();
        let (id, tool) = (text("requestId"), text("tool"));
        let result = match args.get("result") {
            Some(Value::String(s)) => s.clone(),
            Some(v) if !v.is_null() => v.to_string(),
            _ => String::new(),
        };
        if id.is_empty() || tool.is_empty() {
            return Err(AppError::invalid("requestId and tool are required: the request you sent and the tool you sent it with"));
        }
        if result.trim().is_empty() {
            return Err(AppError::invalid("result is required: exactly what the send tool returned"));
        }
        let (r, person, lane) = {
            let db = self.db();
            let mut r = match db.request(&id)? {
                Some(r) if r.employee_id == caller.id => r,
                _ => return Err(AppError::not_found(format!("'{id}' is not one of your requests. Call my_requests() for the list."))),
            };
            if r.state == RequestState::Cancelled {
                return Err(AppError::invalid("that request was cancelled, so there is nothing to watch"));
            }
            if r.delivery.is_some() {
                return Err(AppError::conflict("that request is already recorded as sent"));
            }
            r.delivery = Some(Delivery::new(watch::parse_sent(&tool, &result, now()), DeliverySource::Employee, r.kind));
            db.save_request(&r)?;
            let person = Self::person_name(&db, &r.person_id)?;
            (r, person, db.run(caller_run)?.map(|x| x.run.kind.lane()))
        };
        self.announce(&r);
        let back = if lane == Some(Lane::Work) { "starts a follow-up run that resumes this session" } else { "starts a follow-up run for you" };
        let note = match &r.delivery {
            Some(Delivery { watch: None, .. }) => "Recorded as sent. An FYI needs no reply, so nothing watches it.".to_string(),
            _ if !r.state.is_open() => "Recorded as sent.".to_string(),
            Some(Delivery { locator: Locator::TeamsChat { server: Some(_), .. }, .. }) => format!(
                "Omega Star now watches that Teams chat for {person}'s reply, nudges them once when it is due, and {back} when they reply. Do not watch for it yourself."
            ),
            _ => format!("Omega Star now checks for {person}'s reply in business hours, nudges them once when it is due, and {back} when they reply. Do not watch for it yourself."),
        };
        Ok(json!({ "request": self.present(r), "note": note }))
    }

    /// What a reply check or a nudge found about its own request, recorded once per run. A reply
    /// resolves the request as recorded by nobody, so the employee's one follow-up run starts. On a
    /// Teams chat, a finding that keeps watching leaves the native reader's cursor where it was: the
    /// run may have missed a message, so the reader still judges everything after the cursor.
    pub fn check_result_tool(&self, caller_run: &str, args: &Value) -> Result<Value> {
        let text = |k: &str| args.get(k).and_then(|v| v.as_str()).map(str::trim).unwrap_or("").to_string();
        let id = text("requestId");
        let verdict: Verdict = serde_json::from_value(json!(text("verdict")))
            .map_err(|_| AppError::invalid("verdict must be answered, done, askedBack, acknowledged or noReply"))?;
        let kind = self.run(caller_run)?.kind;
        let nudging = matches!(kind, RunKind::Nudge { .. });
        if !matches!(&kind, RunKind::ReplyCheck { request_id, .. } | RunKind::Nudge { request_id, .. } if *request_id == id) {
            return Err(AppError::invalid("check_result is only for a reply check or a nudge that Omega Star started, about its own request"));
        }
        {
            let mut active = lock(&self.0.active);
            let a = active.get_mut(caller_run).ok_or_else(|| AppError::invalid("this run has ended"))?;
            if a.verdict {
                return Err(AppError::conflict("you already recorded what you found in this run"));
            }
            a.verdict = true;
        }
        let at = now();
        let (words, note) = (text("response"), text("note"));
        let (applied, r) = {
            let db = self.db();
            let mut r = db.request(&id)?.ok_or_else(|| AppError::not_found("request not found"))?;
            let said = if !words.is_empty() { words.clone() } else if !note.is_empty() { note.clone() } else { text("verdict") };
            let heard = Heard { at, from: Self::person_name(&db, &r.person_id)?, text: said.clone(), verdict, note };
            let chat = matches!(r.delivery, Some(Delivery { locator: Locator::TeamsChat { .. }, .. }));
            let seen = watch_of(&mut r).and_then(|w| w.seen.clone());
            let applied = watch::apply_verdict(&mut r, &Seen { id: caller_run.to_string(), at }, heard, &said);
            if chat && applied == Applied::Kept {
                if let Some(w) = watch_of(&mut r) {
                    w.seen = seen;
                }
            }
            if applied != Applied::Stale {
                db.save_request(&r)?;
            }
            (applied, r)
        };
        let next = match (&applied, verdict) {
            (Applied::Stale, _) => "Nothing changed: the request was already answered or is no longer watched. Send nothing; end now with one line.",
            (Applied::Kept, Verdict::NoReply) if nudging => "Recorded: no reply yet. Send the reminder now, as your instructions say.",
            (Applied::Kept, _) if nudging => "Recorded. Send nothing; end now with one line.",
            (Applied::Kept, _) => "Recorded. Omega Star keeps watching. End now with one line.",
            (Applied::Resolve { .. }, _) => "Recorded. A follow-up run carries on with their reply. Send nothing; end now with one line.",
        };
        if applied != Applied::Stale {
            self.announce(&r);
        }
        if let Applied::Resolve { state, response } = applied {
            self.resolve_request(&id, state, Some(response), None)?;
        }
        Ok(json!({ "recorded": verdict, "note": next }))
    }

    /// `update_request` from an employee's run. A reply check or a nudge records with `check_result`
    /// instead: recording as the requester would start no follow-up run.
    pub fn update_request_tool(&self, caller_run: &str, caller: &Employee, args: &Value) -> Result<Value> {
        if matches!(self.run(caller_run)?.kind, RunKind::ReplyCheck { .. } | RunKind::Nudge { .. }) {
            return Err(AppError::invalid("in a reply check or a nudge, record what you found with check_result"));
        }
        let text = |k: &str| args.get(k).and_then(|v| v.as_str()).unwrap_or("");
        let response = args.get("response").and_then(|v| v.as_str()).map(String::from);
        let r = self.resolve_request(text("id"), RequestState::parse(text("state"))?, response, Some(&caller.id))?;
        Ok(json!({ "request": self.present(r) }))
    }

    fn own_schedule(&self, caller: &Employee, id: &str) -> Result<Schedule> {
        match self.db().schedule(id)? {
            Some((s, _)) if s.employee_id == caller.id => Ok(s),
            _ => Err(AppError::not_found(format!("'{id}' is not one of your schedules. Call schedules() for the list."))),
        }
    }

    pub fn my_schedules(&self, caller: &Employee) -> Result<Value> {
        Ok(json!({
            "schedules": self.schedules(&caller.id)?,
            "localTime": Local::now().format("%Y-%m-%d %-I:%M %P (UTC%:z)").to_string(),
        }))
    }

    /// Creates a schedule, or with `id` changes only the fields given.
    pub fn schedule_tool(&self, caller: &Employee, args: &Value) -> Result<Value> {
        let id = args.get("id").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty());
        let mut d = match id {
            Some(id) => self.own_schedule(caller, id)?.draft(),
            None => ScheduleDraft {
                id: None,
                employee_id: caller.id.clone(),
                label: String::new(),
                cron: String::new(),
                instruction: String::new(),
                enabled: true,
                quiet: false,
                model: None,
                effort: None,
                active_hours: false,
            },
        };
        let text = |k: &str| args.get(k).and_then(|v| v.as_str());
        let flag = |k: &str| args.get(k).and_then(|v| v.as_bool());
        if let Some(v) = text("label") {
            d.label = v.to_string();
        }
        if let Some(v) = text("cron") {
            d.cron = v.to_string();
        }
        if let Some(v) = text("instruction") {
            d.instruction = v.to_string();
        }
        if let Some(v) = flag("quiet") {
            d.quiet = v;
        }
        if let Some(v) = flag("enabled") {
            d.enabled = v;
        }
        if let Some(v) = flag("activeHours") {
            d.active_hours = v;
        }
        if let Some(v) = text("model") {
            d.model = Some(v.trim()).filter(|m| !m.is_empty() && *m != "default").map(String::from);
        }
        if let Some(v) = text("effort") {
            d.effort = effort_arg(v)?;
        }
        if d.cron.trim().is_empty() {
            return Err(AppError::invalid("cron is required"));
        }
        if d.instruction.trim().is_empty() {
            return Err(AppError::invalid("instruction is required: what you do each time it fires"));
        }
        let s = self.save_schedule(d)?;
        let when = match schedule::preview(&s.cron, now(), &Local) {
            CronPreview::Ok { description, .. } => description,
            CronPreview::Err { error } => error,
        };
        let mut out = json!({ "schedule": s, "when": when });
        if let Some(tip) = super::schedules::light_model_tip(&s) {
            out["tip"] = tip.into();
        }
        Ok(out)
    }

    pub fn unschedule_tool(&self, caller: &Employee, id: &str) -> Result<Value> {
        let s = self.own_schedule(caller, id)?;
        self.delete_schedule(&s.id)?;
        Ok(json!({ "deleted": s.id }))
    }

    /// Puts something in front of you as a card on the calling run: an item in your Inbox with the answer built in, and a
    /// desktop notification (`ask_card`). A question someone already asked you (same key, or the same chore) joins that
    /// item and notifies nobody again, and so does an edit of the caller's own card about the same thing. A list raised
    /// while the caller's list about the same thing is open grows it (`cards::grow`).
    pub fn notify_tool(&self, caller_run: &str, caller: &Employee, args: &Value) -> Result<Value> {
        let mut card = cards::card(args)?;
        let home = self.employee_dir(&caller.id);
        if let Some(raw) = &card.file {
            card.file = Some(card_file_path(&home, &caller.workspaces, raw)?.to_string_lossy().into_owned());
        }
        card.images = card.images.iter().map(|raw| Ok(card_image_path(&home, &caller.workspaces, raw)?.to_string_lossy().into_owned())).collect::<Result<_>>()?;
        let ceo = {
            let db = self.db();
            // A turn of an older build's chat: a card there would never reach them.
            if db.run(caller_run)?.map(|r| r.run.kind.lane()) == Some(Lane::Chat) {
                return Err(AppError::invalid(format!("Ask {} in your reply instead: a card from this run would not reach them.", ceo_of(&db)?)));
            }
            ceo_of(&db)?
        };
        let mine = match card.topic() {
            Some(t) => self.own_pending(caller, &t)?,
            None => vec![],
        };
        let open_lists: Vec<Vec<CardItem>> = mine
            .iter()
            .filter_map(|c| match &c.ask {
                CardAsk::Items { items, .. } => Some(items.clone()),
                _ => None,
            })
            .collect();
        let grown = match &mut card.ask {
            CardAsk::Items { items, .. } if !open_lists.is_empty() => {
                *items = cards::grow(open_lists.into_iter().chain([std::mem::take(items)]))?;
                Some(items.len())
            }
            _ => None,
        };
        let tip = cards::wants_attachment(&card).then(|| format!("Attach what it is about (file for a draft, link for a pull request) so {ceo} can read it on the card."));
        let news = card.ask == CardAsk::News;
        let chore = matches!(card.ask, CardAsk::Chore { .. });
        let (first, notified) = self.ask_card(caller_run, caller, card, !mine.is_empty())?;
        let note = match (grown, chore, first) {
            (Some(n), ..) => format!("Added to your open list about this: the card now holds {n} items, one item in {ceo}'s Inbox, and they were not notified again. Finish this run now; do not wait for it."),
            (_, true, true) => format!("Recorded. {ceo} has been asked. When they press Done, this conversation carries on with a short message. Carry on without it now, or finish."),
            (_, true, false) => format!("{ceo} has already been asked to do this for another run. When they press Done, this conversation carries on too. Carry on without it now, or finish."),
            _ if news => format!("Recorded. {ceo} sees it in their Inbox until they mark it seen. Nothing comes back to you."),
            (_, _, true) => "Recorded. They answer on the card, and their answers to this run's cards come back to you in this conversation as one short message. Finish this run now; do not wait for it.".to_string(),
            (_, _, false) => format!("{ceo} already has this question from another run, so it is one item in their Inbox and their answer comes back to you too. Finish this run now; do not wait for it."),
        };
        let mut out = json!({ "notified": notified, "note": note });
        if let Some(tip) = tip {
            out["tip"] = tip.into();
        }
        Ok(out)
    }

    /// Puts a line for the company page to you on a card from any run (`cards::company_line_card`), as `notify` puts a
    /// card. Going ahead adds it to the page (`carry_out`).
    pub fn propose_company_line_tool(&self, caller_run: &str, caller: &Employee, args: &Value) -> Result<Value> {
        let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("");
        let card = cards::company_line_card(s("section"), s("line"), s("why"))?;
        let ceo = ceo_of(&self.db())?;
        let edit = match card.topic() {
            Some(t) => !self.own_pending(caller, &t)?.is_empty(),
            None => false,
        };
        let (_, notified) = self.ask_card(caller_run, caller, card, edit)?;
        let note = format!(
            "Recorded. {ceo} sees the line on a card, and Omega Star adds it to the page when they tap Add it, so do not write it there \
yourself. Their answer comes back to you in this conversation. Finish this run now; do not wait for it."
        );
        Ok(json!({ "notified": notified, "note": note }))
    }

    /// Raises `card` on `who`'s run `run_id` (`raise_card`) with a desktop notification, unless someone already asked you
    /// the same thing, or it shows quietly: an `edit` of their own open card about the same thing, a card raised again
    /// within QUIET_AFTER_CLOSE of their closing one about the same thing, or a turn of a conversation you sent a message
    /// in within LIVE, where it shows anyway. Returns whether this is the first time you are asked, and whether you were
    /// notified.
    pub(super) fn ask_card(&self, run_id: &str, who: &Employee, card: Card, edit: bool) -> Result<(bool, bool)> {
        let at = now();
        let topic = card.topic();
        let since = at - QUIET_AFTER_CLOSE;
        let closed = |c: &Card| c.topic() == topic && c.answer.as_ref().is_some_and(|a| a.how == AnswerHow::Withdrawn && a.at > since);
        let reclosed = match &topic {
            Some(t) => self.db().withdrew_since(&who.id, t, since)? || lock(&self.0.active).values().any(|a| a.employee_id == who.id && a.cards.iter().any(closed)),
            None => false,
        };
        let live = {
            let db = self.db();
            let root = db.run(run_id)?.and_then(|r| r.run.conversation_root().map(String::from));
            match root.map(|id| db.run(&id)).transpose()?.flatten() {
                Some(root) => root.run.replied_at.into_iter().chain([root.run.created_at]).max().is_some_and(|t| at - t < LIVE),
                None => false,
            }
        };
        let message = card.message.clone();
        let first = self.raise_card(run_id, card)?;
        let notified = first && !edit && !reclosed && !live;
        if notified {
            self.sink().notify(&format!("{} needs you", who.name), &message);
        }
        Ok((first, notified))
    }

    /// The caller's own cards about `topic` still waiting on you (`Card::waiting`), oldest first: on their runs that
    /// finished (or went back to the queue) and on those still working, this one included.
    fn own_pending(&self, caller: &Employee, topic: &str) -> Result<Vec<Card>> {
        let about = |c: &Card| c.waiting() && c.topic().as_deref() == Some(topic);
        let mut out: Vec<Card> = self.db().open_topic_runs(topic)?.into_iter().filter(|r| r.employee_id == caller.id).flat_map(|r| r.cards).filter(about).collect();
        out.extend(lock(&self.0.active).values().filter(|a| a.employee_id == caller.id).flat_map(|a| a.cards.iter().filter(|c| about(c)).cloned()));
        Ok(out)
    }

    /// Puts `card` on a run: a live run carries it until it ends, a finished one shows it in the Inbox again. A
    /// card about the same thing as one the run already has replaces it. Returns whether nobody else is waiting
    /// on the same thing, so this is the first time you are asked. No desktop notification.
    fn raise_card(&self, run_id: &str, card: Card) -> Result<bool> {
        let topic = card.topic();
        let same = |c: &Card| c.waiting() && topic.is_some() && c.topic() == topic;
        let waiting = match &topic {
            Some(t) => self.db().open_topic_runs(t)?.iter().any(|r| r.id != run_id),
            None => false,
        };
        let asking = lock(&self.0.active).iter().any(|(id, a)| id != run_id && a.cards.iter().any(&same));
        let first = !waiting && !asking;
        let put = |cards: &mut Vec<Card>, needs_you: &mut Vec<String>| -> Result<()> {
            if let Some(old) = cards.iter_mut().find(|c| same(c)) {
                needs_you.retain(|m| *m != old.message);
                *old = card.clone();
            } else if cards.iter().filter(|c| c.pending()).count() >= CARDS_PER_RUN {
                return Err(AppError::conflict(format!("This run already has {CARDS_PER_RUN} cards waiting. Put the rest in your report.")));
            } else {
                cards.push(card.clone());
            }
            needs_you.push(card.message.clone());
            Ok(())
        };
        {
            let mut active = lock(&self.0.active);
            if let Some(a) = active.get_mut(run_id) {
                let (mut cards, mut needs_you) = (std::mem::take(&mut a.cards), std::mem::take(&mut a.needs_you));
                let done = put(&mut cards, &mut needs_you);
                (a.cards, a.needs_you) = (cards, needs_you);
                return done.map(|_| first);
            }
        }
        let run = {
            let db = self.db();
            let mut run = db.run(run_id)?.ok_or_else(|| AppError::not_found("run not found"))?.run;
            if !run.state.is_finished() {
                return Err(AppError::conflict("that run has not finished yet"));
            }
            put(&mut run.cards, &mut run.needs_you)?;
            run.archived_at = None;
            db.save_run(&run)?;
            run
        };
        self.sink().run_updated(&run);
        Ok(first)
    }

    /// Asks you on a run to reconnect Claude in Chrome, with `why` under it. Only the first run to ask for it
    /// sends a desktop notification; the rest wait on the same Done.
    pub(super) fn raise_chrome_chore(&self, run_id: &str, why: &str) -> Result<()> {
        const MESSAGE: &str = "Reconnect Claude in Chrome";
        let name = {
            let db = self.db();
            let run = db.run(run_id)?.map(|r| r.run);
            run.and_then(|r| db.employee(&r.employee_id).ok().flatten()).map(|e| e.name).unwrap_or_default()
        };
        let chore = CardAsk::Chore { subject: cards::CHROME.into(), url: None };
        if self.raise_card(run_id, Card::new(MESSAGE, Some(why.to_string()), None, chore))? {
            self.sink().notify(&format!("{name} needs you"), MESSAGE);
        }
        Ok(())
    }

    /// Closes the caller's cards about `key` still waiting on you (`Card::waiting`), ones put off with Later too: it was
    /// settled without you, so they never come back. Rows left with nothing open leave your Inbox, news you have not
    /// seen aside.
    pub fn close_ask_tool(&self, caller: &Employee, key: &str, reason: &str) -> Result<Value> {
        let topic = cards::ask_key(key)?;
        let answer = CardAnswer { at: now(), how: AnswerHow::Withdrawn, text: claude::clip(&claude::one_line(reason), 300), held: false };
        let about = |c: &Card| c.waiting() && c.topic().as_deref() == Some(topic.as_str());
        let close = |c: &mut Card| {
            c.answer = Some(answer.clone());
            (c.back_at, c.parked) = (None, false);
        };
        let mut closed = 0;
        let (runs, said) = {
            let db = self.db();
            let mut runs: Vec<Run> = db.open_topic_runs(&topic)?.into_iter().filter(|r| r.employee_id == caller.id).collect();
            for r in db.parked_runs()? {
                if r.employee_id == caller.id && r.cards.iter().any(about) && !runs.iter().any(|x| x.id == r.id) {
                    runs.push(r);
                }
            }
            let mut said = vec![];
            db.tx(|| {
                for r in &mut runs {
                    for c in r.cards.iter_mut().filter(|c| about(c)) {
                        close(c);
                        closed += 1;
                    }
                    said.push(super::inbox::take_held(r));
                    db.save_run(r)?;
                }
                Ok(())
            })?;
            (runs, said)
        };
        for a in lock(&self.0.active).values_mut().filter(|a| a.employee_id == caller.id) {
            for c in a.cards.iter_mut().filter(|c| about(c)) {
                close(c);
                closed += 1;
            }
        }
        // A run left with nothing open hears the answers you gave its other cards, as a tap would have sent them.
        let (sends, runs): (Vec<_>, Vec<_>) = runs.into_iter().zip(said).partition(|(_, s)| s.is_some());
        self.send_card_answers(&sends);
        for (r, _) in &runs {
            self.sink().run_updated(r);
            if r.state.is_finished() {
                // As a tap files it, so news nobody has seen keeps its row.
                match self.archive_row(&r.id, true, true) {
                    Err(e) if e.code != crate::error::ErrorCode::Conflict => log!("archiving run {} after closing its card failed: {e}", r.id),
                    _ => {}
                }
            }
        }
        if closed == 0 {
            let open: Vec<String> = self.db().open_cards(&caller.id)?.iter().flat_map(|r| r.cards.iter().filter(|c| c.pending()).filter_map(|c| c.key.clone())).collect();
            let open: Vec<&str> = open.iter().map(|k| cards::key_label(k)).collect();
            let list = if open.is_empty() { "none".to_string() } else { open.join(", ") };
            return Err(AppError::invalid(format!("You have no open card about '{key}'. Your open cards: {list}.")));
        }
        Ok(json!({ "closed": closed }))
    }

    /// Records the number the caller's weekly review measured, for your team's scoreboard. Only in a weekly review.
    pub fn report_number_tool(&self, caller_run: &str, args: &Value) -> Result<Value> {
        let text = |k: &str, max: usize| -> Result<String> {
            let v = args.get(k).and_then(Value::as_str).map(str::trim).unwrap_or("");
            if v.chars().count() > max {
                return Err(AppError::invalid(format!("{k} is at most {max} characters.")));
            }
            Ok(claude::one_line(v))
        };
        let score = Score { metric: text("metric", 40)?, value: text("value", 20)?, change: text("change", 30)?, note: text("note", 100)? };
        if score.metric.is_empty() || score.value.is_empty() {
            return Err(AppError::invalid("Give the metric and its value, e.g. metric 'Open support tickets', value '0'."));
        }
        let review = self.db().run(caller_run)?.is_some_and(|r| matches!(r.run.kind, RunKind::Scheduled { review: true, .. }));
        if !review {
            return Err(AppError::invalid("report_number is for your weekly review."));
        }
        let mut active = lock(&self.0.active);
        let a = active.get_mut(caller_run).ok_or_else(|| AppError::invalid("this run has ended"))?;
        a.score = Some(score);
        Ok(json!({ "recorded": true }))
    }

    /// Your Claude Code skill catalog, for choosing the skills a proposed hire needs.
    pub fn catalog_tool(&self) -> Result<Value> {
        let skills: Vec<Value> = self
            .skill_catalog()
            .iter()
            .map(|s| json!({ "name": s.name, "description": claude::clip(&s.description, 300), "source": s.source, "skills": s.skills }))
            .collect();
        Ok(json!({ "skills": skills }))
    }

    /// Records a hire the caller proposes. The hire reports to the caller unless they name one of
    /// their direct reports; skills are matched against your catalog by name.
    pub fn propose_hire_tool(&self, caller_run: &str, caller: &Employee, args: &Value) -> Result<Value> {
        let s = |k: &str| args.get(k).and_then(|v| v.as_str()).map(str::trim).unwrap_or("").to_string();
        let (name, title, reason) = (s("name"), s("title"), s("reason"));
        if title.is_empty() {
            return Err(AppError::invalid("title is required: the role you want hired"));
        }
        if name.is_empty() {
            return Err(AppError::invalid("name is required: suggest a first name for the new hire"));
        }
        if reason.is_empty() {
            return Err(AppError::invalid("reason is required: the gap this hire fills and what they would own"));
        }
        let asked: Vec<String> = args
            .get("skills")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str()).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
            .unwrap_or_default();
        let catalog = self.skill_catalog();
        let (mut keys, mut missing) = (vec![], vec![]);
        for n in &asked {
            match catalog.iter().find(|c| c.name.eq_ignore_ascii_case(n) || c.id.eq_ignore_ascii_case(n)) {
                Some(c) if !keys.contains(&c.id) => keys.push(c.id.clone()),
                Some(_) => {}
                None => missing.push(n.clone()),
            }
        }
        let (proposal, lane, ceo) = {
            let db = self.db();
            let to = s("reportsTo");
            let manager_id = if to.is_empty() || find_employee(std::slice::from_ref(caller), &to).is_some() {
                caller.id.clone()
            } else {
                let reports: Vec<Employee> = db.employees()?.into_iter().filter(|e| e.reports_to(&caller.id)).collect();
                find_employee(&reports, &to).map(|e| e.id.clone()).ok_or_else(|| {
                    AppError::invalid(format!("'{to}' is neither you nor one of your direct reports. Leave reportsTo out and the hire reports to you."))
                })?
            };
            if let Some(dup) = db.proposals(Some(&caller.id), Some(ProposalState::Open))?.into_iter().find(|p| p.draft.title.eq_ignore_ascii_case(&title)) {
                return Err(AppError::conflict(format!(
                    "You already proposed a {} (proposal `{}`) and it is waiting on a decision. To change it, withdraw_proposal it and propose again.",
                    dup.draft.title, dup.id
                )));
            }
            let hue = hue_from_name(&name);
            let p = HireProposal {
                id: uuid::Uuid::new_v4().to_string(),
                employee_id: caller.id.clone(),
                run_id: Some(caller_run.to_string()),
                reason,
                draft: EmployeeDraft {
                    name,
                    title,
                    description: s("description"),
                    objective: s("objective"),
                    manager_id: Some(manager_id),
                    skills: SkillGrant { builtins: false, keys },
                    clearance: Clearance::Supervised,
                    allow_rules: vec![],
                    model: None,
                    effort: None,
                    browser: false,
                    auto_tidy: true,
                    workspaces: vec![],
                    hue,
                    image: None,
                },
                missing_skills: missing,
                state: ProposalState::Open,
                note: None,
                hired_id: None,
                created_at: now(),
                resolved_at: None,
            };
            db.save_proposal(&p)?;
            let lane = db.run(caller_run)?.map(|r| r.run.kind.lane());
            (p, lane, ceo_of(&db)?)
        };
        let d = &proposal.draft;
        self.append_event(caller_run, RunEvent::ProposedHire { proposal_id: proposal.id.clone(), name: d.name.clone(), title: d.title.clone() });
        self.sink().proposal_updated(&proposal);
        if lane != Some(Lane::Chat) {
            self.sink().notify(&format!("{} proposes a hire", caller.name), &format!("{}, {}", d.name, d.title));
        }
        let mut note = format!(
            "Recorded. {ceo} reviews it in their Inbox, may change it, and approves or declines it. Their decision comes back to \
you as a new run. Never create the hire any other way."
        );
        if !proposal.missing_skills.is_empty() {
            note += &format!(
                " Not in the catalog, so not granted: {}. Say what the hire needs them for; {ceo} can install them.",
                proposal.missing_skills.join(", ")
            );
        }
        Ok(json!({ "proposal": proposal, "note": note }))
    }

    pub fn my_proposals(&self, caller: &Employee) -> Result<Value> {
        let v = self.db().proposals(Some(&caller.id), None)?;
        Ok(json!({ "proposals": v.into_iter().take(20).collect::<Vec<_>>() }))
    }

    pub fn withdraw_proposal_tool(&self, caller: &Employee, id: &str) -> Result<Value> {
        let p = {
            let db = self.db();
            let mut p = match db.proposal(id)? {
                Some(p) if p.employee_id == caller.id => p,
                _ => return Err(AppError::not_found(format!("'{id}' is not one of your proposals. Call my_proposals() for the list."))),
            };
            if p.state != ProposalState::Open {
                return Err(AppError::conflict(format!("that proposal is already {}", p.state.as_str())));
            }
            p.state = ProposalState::Withdrawn;
            p.resolved_at = Some(now());
            db.save_proposal(&p)?;
            p
        };
        self.sink().proposal_updated(&p);
        Ok(json!({ "withdrawn": p.id }))
    }

    /// Names the thread the calling run is in (`Run::title`, on its first run): the conversation, or the report and
    /// the replies on it. When that first run is the caller, its end saves the name again, so it keeps it.
    pub fn name_thread_tool(&self, caller_run: &str, title: &str) -> Result<Value> {
        let title = claude::one_line(title);
        if title.is_empty() {
            return Err(AppError::invalid("Give the thread a name of a few words, e.g. 'Spring sale emails'."));
        }
        if title.chars().count() > TITLE_CHARS {
            return Err(AppError::invalid(format!("title is at most {TITLE_CHARS} characters.")));
        }
        let root = {
            let db = self.db();
            let mut root = Self::thread_in(&db, caller_run)?.root;
            root.title = Some(title.clone());
            db.save_run(&root)?;
            root
        };
        if let Some(a) = lock(&self.0.active).get_mut(&root.id) {
            a.title = Some(title.clone());
        }
        self.sink().run_updated(&root);
        Ok(json!({ "named": title }))
    }

    /// Opens a file, a folder or a page on your screen for the caller (`open_target` says what they may open), and
    /// says whether it worked. Only in a conversation you are in (`ceo_present`): from a run nobody watches,
    /// a tab or a window would appear out of nowhere.
    pub fn open_tool(&self, caller_run: &str, caller: &Employee, target: &str) -> Result<Value> {
        let ceo = {
            let db = self.db();
            let ceo = ceo_of(&db)?;
            if !db.run(caller_run)?.is_some_and(|r| ceo_present(&r.run)) {
                return Err(AppError::invalid(format!(
                    "{ceo} is not in this run, so nothing opens on their screen. Put the link on your card instead: a sign-in page goes in the chore's url."
                )));
            }
            ceo
        };
        let target = open_target(&self.employee_dir(&caller.id), &caller.workspaces, target.trim())?;
        if let Err(e) = self.sink().open(&target) {
            return Err(AppError::internal(format!("It did not open: {e}. Tell {ceo} where it is instead.")));
        }
        Ok(json!({ "opened": true, "note": format!("It is open on {ceo}'s screen.") }))
    }
}

/// Where the file a card shows is, as a full path (`in_folders`). It must be plain text of at most `cards::FILE_BYTES`,
/// so the card can show it.
pub(super) fn card_file_path(employee_dir: &Path, workspaces: &[String], raw: &str) -> Result<PathBuf> {
    let path = in_folders(employee_dir, workspaces, raw)?;
    let len = fs::metadata(&path)?.len();
    if len > cards::FILE_BYTES {
        return Err(AppError::invalid(format!(
            "'{raw}' is {} KB, and a card shows files up to {} KB. Point at a shorter file, or leave file out.",
            len.div_ceil(1024),
            cards::FILE_BYTES / 1024
        )));
    }
    if fs::read_to_string(&path).is_err() {
        return Err(AppError::invalid(format!("'{raw}' is not plain text, so the card cannot show it.")));
    }
    Ok(path)
}

/// What `open` opens: an https link as it is, or a path (full, or from the employee's folder as their runs see it),
/// resolved, in one of their workspaces or the folder that holds the company page, which holds every employee's
/// folder too. Links are followed first, so one cannot lead anywhere else.
fn open_target(employee_dir: &Path, workspaces: &[String], raw: &str) -> Result<String> {
    if raw.starts_with("https://") {
        return Ok(raw.to_string());
    }
    if raw.is_empty() {
        return Err(AppError::invalid("Pass the full path of what to open, or an https link."));
    }
    let path = fs::canonicalize(employee_dir.join(raw))
        .map_err(|_| AppError::invalid(format!("Nothing is at '{raw}'. Pass a full path, or an https link.")))?;
    let company = employee_dir.parent().unwrap_or(employee_dir);
    let mut roots = std::iter::once(company.to_path_buf()).chain(workspaces.iter().map(|w| employee_dir.join(w))).filter_map(|r| fs::canonicalize(r).ok());
    if !roots.any(|r| path.starts_with(r)) {
        return Err(AppError::invalid("You can open what is in your folder, one of your workspaces or the company page's folder, or an https link."));
    }
    Ok(path.to_string_lossy().into_owned())
}

/// Where an image a card shows is, as a full path (`in_folders`): a png, jpg, gif or webp file.
pub(super) fn card_image_path(employee_dir: &Path, workspaces: &[String], raw: &str) -> Result<PathBuf> {
    let path = in_folders(employee_dir, workspaces, raw)?;
    if image_mime(&path).is_none() {
        return Err(AppError::invalid(format!("'{raw}' is not a png, jpg, gif or webp file, so the card cannot show it.")));
    }
    Ok(path)
}

/// A file a card shows, as a full path: `raw` is full, or from the employee's folder as their runs see it. Links are
/// followed first, so one cannot lead out of their folder and workspaces.
fn in_folders(employee_dir: &Path, workspaces: &[String], raw: &str) -> Result<PathBuf> {
    let path = fs::canonicalize(employee_dir.join(raw))
        .ok()
        .filter(|p| p.is_file())
        .ok_or_else(|| AppError::invalid(format!("No file at '{raw}'. Pass the path of the file you wrote, e.g. 'workspace/drafts/reply.md'.")))?;
    let mut roots = std::iter::once(employee_dir.to_path_buf()).chain(workspaces.iter().map(|w| employee_dir.join(w))).filter_map(|r| fs::canonicalize(r).ok());
    if !roots.any(|r| path.starts_with(r)) {
        return Err(AppError::invalid("file must be in your folder or one of your workspaces."));
    }
    Ok(path)
}

/// The avatar hue the hire sheet derives from a name (`hueFromName` in lib/format.ts).
fn hue_from_name(name: &str) -> u16 {
    let n = name.trim().to_lowercase();
    if n.is_empty() {
        return 250;
    }
    n.encode_utf16().fold(0u32, |h, c| (h * 31 + c as u32) % 360) as u16
}

pub(super) fn parse_due(s: &str) -> Result<Ts> {
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(t.with_timezone(&Utc));
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map(|d| d.and_hms_opt(12, 0, 0).unwrap().and_utc())
        .map_err(|_| AppError::invalid("dueAt must be an ISO date or date-time"))
}

/// The schedule tool's `effort`: a level, or 'default' (or nothing) to go back to the employee's own.
fn effort_arg(v: &str) -> Result<Option<Effort>> {
    match v.trim() {
        "" | "default" => Ok(None),
        v => serde_json::from_value(json!(v)).map(Some).map_err(|_| AppError::invalid("effort must be low, medium, high, xhigh or max")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{open_office, saved_run, task};
    use crate::office::fixtures::{employee, held_office, office, saved, standup, Office0};

    // Makes a symlink, which needs unix (Windows asks for extra rights).
    #[cfg(unix)]
    #[test]
    fn a_card_shows_only_text_files_in_the_employees_folders() {
        let o = office();
        let home = o.dir.join("employees").join("e1");
        let repo = o.dir.join("repo");
        fs::create_dir_all(home.join("workspace/drafts")).unwrap();
        fs::create_dir_all(&repo).unwrap();
        fs::write(home.join("workspace/drafts/reply.md"), "Hi").unwrap();
        fs::write(repo.join("notes.txt"), "Notes").unwrap();
        fs::write(o.dir.join("outside.md"), "Not theirs").unwrap();
        fs::write(home.join("big.md"), "x".repeat(cards::FILE_BYTES as usize + 1)).unwrap();
        fs::write(home.join("binary.md"), [0xff, 0xfe, 0x00]).unwrap();
        std::os::unix::fs::symlink(o.dir.join("outside.md"), home.join("link.md")).unwrap();
        let workspaces = vec![repo.to_string_lossy().into_owned()];
        let at = |raw: &str| card_file_path(&home, &workspaces, raw);
        let real = |p: PathBuf| fs::canonicalize(p).unwrap();
        assert_eq!(at("workspace/drafts/reply.md").unwrap(), real(home.join("workspace/drafts/reply.md")));
        assert_eq!(at(&repo.join("notes.txt").to_string_lossy()).unwrap(), real(repo.join("notes.txt")));
        for (raw, why) in [
            ("workspace/drafts/gone.md", "No file at"),
            ("workspace/drafts", "No file at"),
            ("../../outside.md", "file must be in your folder"),
            ("link.md", "file must be in your folder"),
            ("big.md", "KB, and a card shows files up to 200 KB"),
            ("binary.md", "is not plain text"),
        ] {
            assert!(at(raw).unwrap_err().message.contains(why), "{raw}");
        }
    }

    // Makes a symlink, which needs unix (Windows asks for extra rights).
    #[cfg(unix)]
    #[tokio::test]
    async fn a_card_shows_its_file_as_it_is_now() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let r = saved_run(&o.db, &ada, RunKind::Manual, "Draft the reply.", RunState::Succeeded, 5, None);
        let office = open_office(&o.dir).await;
        let draft = office.employee_dir(&ada.id).join("workspace/reply.md");
        fs::create_dir_all(draft.parent().unwrap()).unwrap();
        fs::write(&draft, "First go").unwrap();
        let args = json!({ "message": "Send the reply?", "key": "reply", "file": "workspace/reply.md", "choices": [{ "label": "Send", "recommended": true }] });
        office.notify_tool(&r.id, &ada, &args).unwrap();
        let card = o.db.run(&r.id).unwrap().unwrap().run.cards.remove(0);
        assert_eq!(card.file, Some(fs::canonicalize(&draft).unwrap().to_string_lossy().into_owned()), "kept as a full path");

        fs::write(&draft, "Second go").unwrap();
        assert_eq!(office.card_file(&r.id, 0).unwrap(), "Second go", "what would go out now");
        assert_eq!(office.card_file(&r.id, 1).unwrap_err().message, "That card shows no file.");
        fs::remove_file(&draft).unwrap();
        assert!(office.card_file(&r.id, 0).unwrap_err().message.starts_with("Nothing is at"));
        std::os::unix::fs::symlink(o.dir.join("office.db"), &draft).unwrap();
        assert!(office.card_file(&r.id, 0).unwrap_err().message.starts_with("Nothing is at"), "a link put in its place is not followed");
        drop(office);
    }

    /// A sink that keeps the desktop notifications, by body.
    #[derive(Default)]
    struct Told(std::sync::Mutex<Vec<String>>);
    impl EventSink for Told {
        fn org_changed(&self) {}
        fn run_updated(&self, _run: &Run) {}
        fn run_events(&self, _run_id: &str, _events: &[RunEvent]) {}
        fn request_updated(&self, _request: &HumanRequest) {}
        fn notify(&self, _title: &str, body: &str) {
            self.0.lock().unwrap().push(body.to_string());
        }
    }

    /// The Office on `o`'s store, held so nothing it queues starts, keeping its notifications in the sink it returns.
    async fn told_office(o: &Office0) -> (Office, Arc<Told>) {
        o.db.set_held_until(Some(now() + chrono::Duration::hours(1))).unwrap();
        let told = Arc::new(Told::default());
        (Office::open(o.dir.clone(), told.clone(), Build::new("0.0.0".into())).await.unwrap(), told)
    }

    #[tokio::test]
    async fn a_list_grows_and_an_edit_notifies_nobody_again() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let first = saved_run(&o.db, &ada, RunKind::Manual, "Review the pull requests.", RunState::Succeeded, 30, None);
        let later = saved_run(&o.db, &ada, RunKind::Manual, "Review the pull requests.", RunState::Succeeded, 10, None);
        let (office, told) = told_office(&o).await;
        let list = |items: &[(&str, &str)]| {
            let items: Vec<Value> = items.iter().map(|(id, text)| json!({ "id": id, "text": text })).collect();
            json!({ "message": "Merge these?", "key": "merges", "itemVerb": "Merge", "items": items })
        };
        assert_eq!(office.notify_tool(&first.id, &ada, &list(&[("pr:1", "Fix the footer."), ("pr:2", "New prices.")])).unwrap()["notified"], true);
        let grown = office.notify_tool(&later.id, &ada, &list(&[("pr:2", "New prices, with tax."), ("pr:3", "Faster search.")])).unwrap();
        assert_eq!(grown["notified"], false, "{grown}");
        assert!(grown["note"].as_str().unwrap().starts_with("Added to your open list about this: the card now holds 3 items"), "{grown}");
        let CardAsk::Items { items, .. } = saved(&o.db, &later.id).cards.remove(0).ask else { panic!("items") };
        let items: Vec<(&str, &str)> = items.iter().map(|i| (i.id.as_str(), i.text.as_str())).collect();
        assert_eq!(items, vec![("pr:1", "Fix the footer."), ("pr:2", "New prices, with tax."), ("pr:3", "Faster search.")], "the newer text wins");

        // Asking again with the same key edits the card in place, and so does raising it again soon after closing it.
        let venue = |why: &str| json!({ "message": "Book the hall?", "key": "venue", "why": why, "choices": [{ "label": "Book it" }] });
        assert_eq!(office.notify_tool(&later.id, &ada, &venue("Cheapest in May.")).unwrap()["notified"], true);
        assert_eq!(office.notify_tool(&later.id, &ada, &venue("Cheapest in June.")).unwrap()["notified"], false);
        let venues = |r: &Run| saved(&o.db, &r.id).cards.into_iter().filter(|c| c.key.as_deref() == Some("ask:venue")).collect::<Vec<_>>();
        assert_eq!(venues(&later).len(), 1, "changed in place");
        office.close_ask_tool(&ada, "venue", "Asked it wrongly.").unwrap();
        assert_eq!(office.notify_tool(&later.id, &ada, &venue("Cheapest in July.")).unwrap()["notified"], false, "soon after closing it");
        assert_eq!(told.0.lock().unwrap().clone(), vec!["Merge these?".to_string(), "Book the hall?".to_string()]);
        drop(office);
    }

    #[tokio::test]
    async fn a_card_in_a_conversation_you_are_in_shows_there_without_a_notification() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let fresh = saved_run(&o.db, &ada, task("The offsite"), "Plan the offsite.", RunState::Succeeded, 5, None);
        let old = saved_run(&o.db, &ada, task("The rota"), "Plan the rota.", RunState::Succeeded, 60, None);
        let (office, told) = told_office(&o).await;
        let ask = |message: &str, key: &str| json!({ "message": message, "key": key, "choices": [{ "label": "Go ahead" }] });
        assert_eq!(office.notify_tool(&fresh.id, &ada, &ask("Book the hall?", "hall")).unwrap()["notified"], false, "you wrote in it 5 minutes ago");
        assert_eq!(office.notify_tool(&old.id, &ada, &ask("Swap the Friday shift?", "shift")).unwrap()["notified"], true);
        let mut replied = saved(&o.db, &old.id);
        replied.replied_at = Some(now() - chrono::Duration::minutes(1));
        o.db.save_run(&replied).unwrap();
        assert_eq!(office.notify_tool(&old.id, &ada, &ask("Add a Sunday shift?", "sunday")).unwrap()["notified"], false, "your reply a minute ago");
        assert_eq!(told.0.lock().unwrap().len(), 1);
        assert_eq!(saved(&o.db, &fresh.id).cards.len(), 1, "the card is raised all the same");
        drop(office);
    }

    // Makes a symlink, which needs unix (Windows asks for extra rights).
    #[cfg(unix)]
    #[tokio::test]
    async fn a_card_carries_a_link_and_images_and_asks_for_what_it_is_about() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let r = saved_run(&o.db, &ada, RunKind::Manual, "Review it.", RunState::Succeeded, 5, None);
        let office = held_office(&o).await;
        let home = office.employee_dir(&ada.id);
        fs::create_dir_all(home.join("workspace/shots")).unwrap();
        fs::write(home.join("workspace/shots/before.png"), [0x89, b'P', b'N', b'G']).unwrap();
        fs::write(home.join("workspace/shots/notes.md"), "Notes").unwrap();
        std::os::unix::fs::symlink(home.join("workspace/shots/notes.md"), home.join("workspace/shots/fake.png")).unwrap();
        fs::write(o.dir.join("outside.png"), [0u8]).unwrap();
        let merge = |extra: Value| {
            let mut args = json!({ "message": "Merge PR #12?", "key": "pr:12", "choices": [{ "label": "Merge" }] });
            args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            args
        };
        let v = office.notify_tool(&r.id, &ada, &merge(json!({ "images": ["workspace/shots/before.png"] }))).unwrap();
        assert_eq!(v["tip"], "Attach what it is about (file for a draft, link for a pull request) so Pat can read it on the card.");
        let shot = fs::canonicalize(home.join("workspace/shots/before.png")).unwrap().to_string_lossy().into_owned();
        assert_eq!(saved(&o.db, &r.id).cards[0].images, vec![shot], "kept as a full path");
        let v = office.notify_tool(&r.id, &ada, &merge(json!({ "link": "https://example.com/acme/site/pull/12" }))).unwrap();
        assert!(v.get("tip").is_none(), "{v}");
        assert_eq!(saved(&o.db, &r.id).cards[0].link.as_deref(), Some("https://example.com/acme/site/pull/12"));
        for (raw, why) in [("../../outside.png", "file must be in your folder"), ("workspace/shots/gone.png", "No file at"), ("workspace/shots/fake.png", "is not a png, jpg, gif or webp file")] {
            assert!(office.notify_tool(&r.id, &ada, &merge(json!({ "images": [raw] }))).unwrap_err().message.contains(why), "{raw}");
        }
        drop(office);
    }

    #[tokio::test]
    async fn a_company_line_is_proposed_on_a_card_from_any_run() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let r = saved_run(&o.db, &ada, standup("s1"), "Sweep the inbox.", RunState::Succeeded, 5, None);
        let (office, told) = told_office(&o).await;
        let line = json!({ "section": "Who to ask", "line": "Bo signs off refunds.", "why": "Three of us asked this week." });
        let v = office.propose_company_line_tool(&r.id, &ada, &line).unwrap();
        assert!(v["note"].as_str().unwrap().starts_with("Recorded. Pat sees the line on a card"), "{v}");
        assert_eq!(v["notified"], true);
        let card = saved(&o.db, &r.id).cards.remove(0);
        assert_eq!((card.message.as_str(), card.proposal), (cards::ADD_LINE, Some(Proposal::CompanyLine { section: "Who to ask".into(), line: "Bo signs off refunds.".into() })));
        assert_eq!(office.propose_company_line_tool(&r.id, &ada, &line).unwrap()["notified"], false, "the same line again is the same card");
        assert_eq!(saved(&o.db, &r.id).cards.len(), 1);
        assert!(office.propose_company_line_tool(&r.id, &ada, &json!({ "section": "Who to ask", "line": " ", "why": "Asked." })).is_err());
        assert_eq!(told.0.lock().unwrap().len(), 1);
        drop(office);
    }

    #[tokio::test]
    async fn a_card_read_back_after_a_restart_shows_its_file_only_if_notify_would_have() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let r = saved_run(&o.db, &ada, RunKind::Manual, "Draft the replies.", RunState::Running, 10, None);
        let home = o.dir.join("employees").join(&ada.id);
        fs::create_dir_all(home.join("workspace")).unwrap();
        fs::write(home.join("workspace/reply.md"), "Hi").unwrap();
        fs::write(o.dir.join("outside.md"), "Not theirs").unwrap();
        let ask = |key: &str, file: &str| {
            let input = json!({ "message": "Send it?", "key": key, "file": file, "choices": [{ "label": "Send", "recommended": true }] });
            json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "content": [{ "type": "tool_use", "id": key, "name": "mcp__omega__notify", "input": input }] } }).to_string()
        };
        let done = json!({ "type": "result", "subtype": "success", "is_error": false, "num_turns": 2, "duration_ms": 900, "result": "Drafted.", "permission_denials": [] });
        let transcript = [ask("ours", "workspace/reply.md"), ask("theirs", "../../outside.md"), done.to_string()].join("\n");
        fs::create_dir_all(o.dir.join("runs").join(&r.id)).unwrap();
        fs::write(o.dir.join("runs").join(&r.id).join("transcript.jsonl"), transcript).unwrap();

        let office = open_office(&o.dir).await;
        let cards = o.db.run(&r.id).unwrap().unwrap().run.cards;
        let files: Vec<_> = cards.iter().map(|c| c.file.clone()).collect();
        assert_eq!(files, vec![Some(fs::canonicalize(home.join("workspace/reply.md")).unwrap().to_string_lossy().into_owned()), None]);
        drop(office);
    }

    #[test]
    fn schedule_effort_takes_a_level_or_default() {
        assert_eq!(effort_arg("low").unwrap(), Some(Effort::Low));
        assert_eq!(effort_arg(" xhigh ").unwrap(), Some(Effort::Xhigh));
        assert_eq!(effort_arg("default").unwrap(), None);
        assert_eq!(effort_arg("quick").unwrap_err().message, "effort must be low, medium, high, xhigh or max");
    }

    #[tokio::test]
    async fn name_thread_names_the_thread_its_run_is_in() {
        let o = office();
        let ada = employee(&o.db, "Ada", None);
        let first = saved_run(&o.db, &ada, task("Can you look at the sale"), "Can you look at the sale?", RunState::Succeeded, 30, None);
        let reply = Run::new(&ada.id, RunKind::Reply { to_run_id: first.id.clone(), label: "Can you look at the sale".into(), task: true, card: false }, "And the emails?".into(), now());
        assert!(o.db.insert_run(&reply, &first.id).unwrap());
        let office = held_office(&o).await;

        assert_eq!(office.name_thread_tool(&reply.id, "  Spring sale\n emails ").unwrap(), json!({ "named": "Spring sale emails" }));
        assert_eq!(saved(&o.db, &first.id).title.as_deref(), Some("Spring sale emails"), "on the conversation's first run");
        assert_eq!(saved(&o.db, &reply.id).title, None);
        assert!(office.name_thread_tool(&reply.id, " ").unwrap_err().message.starts_with("Give the thread a name of a few words"));
        assert_eq!(office.name_thread_tool(&reply.id, &"x".repeat(61)).unwrap_err().message, "title is at most 60 characters.");
        office.name_thread_tool(&reply.id, &"x".repeat(60)).unwrap();

        // A report still being written names its own thread, and its end keeps the name.
        let report = saved_run(&o.db, &ada, standup("s1"), "Stand-up.", RunState::Running, 5, None);
        let live = Active { token: "t".into(), employee_id: ada.id.clone(), ..Default::default() };
        lock(&office.0.active).insert(report.id.clone(), live);
        office.name_thread_tool(&report.id, "Weekly numbers").unwrap();
        assert_eq!(saved(&o.db, &report.id).title.as_deref(), Some("Weekly numbers"));
        assert_eq!(lock(&office.0.active)[&report.id].title.as_deref(), Some("Weekly numbers"));
        drop(office);
    }

    /// Records what it opens, and fails on anything ending in `.fail`. Only the unix-only test below uses it.
    #[cfg(unix)]
    #[derive(Default)]
    struct Screen(Mutex<Vec<String>>);
    #[cfg(unix)]
    impl EventSink for Screen {
        fn org_changed(&self) {}
        fn run_updated(&self, _run: &Run) {}
        fn run_events(&self, _run_id: &str, _events: &[RunEvent]) {}
        fn request_updated(&self, _request: &HumanRequest) {}
        fn open(&self, target: &str) -> std::result::Result<(), String> {
            if target.ends_with(".fail") {
                return Err("its app is gone".into());
            }
            lock(&self.0).push(target.to_string());
            Ok(())
        }
    }

    // Makes a symlink, which needs unix (Windows asks for extra rights).
    #[cfg(unix)]
    #[tokio::test]
    async fn open_shows_only_what_is_in_the_office_or_a_workspace() {
        let o = office();
        let repo = o.dir.join("repo");
        fs::create_dir_all(&repo).unwrap();
        fs::write(repo.join("notes.md"), "Notes").unwrap();
        fs::write(o.dir.join("outside.md"), "Not theirs").unwrap();
        let ada = Employee { workspaces: vec![repo.to_string_lossy().into_owned()], ..employee(&o.db, "Ada", None) };
        let bo = employee(&o.db, "Bo", None);
        // Finished, so nothing is left to start: whether Pat is in a run depends on its kind alone.
        let talking = saved_run(&o.db, &ada, task("The report"), "Show me the report.", RunState::Succeeded, 1, None);
        let sweep = saved_run(&o.db, &ada, standup("s1"), "Stand-up.", RunState::Succeeded, 1, None);
        let back = RunKind::Reply { to_run_id: talking.id.clone(), label: "The report".into(), task: true, card: false };
        let check = Run { check: Some("c1".into()), state: RunState::Succeeded, ..Run::new(&ada.id, back, "Check.".into(), now()) };
        assert!(o.db.insert_run(&check, &talking.id).unwrap());
        let screen = Arc::new(Screen::default());
        let office = Office::open(o.dir.clone(), screen.clone(), Build::new("0.0.0".into())).await.unwrap();
        let home = office.employee_dir(&ada.id);
        let theirs = office.employee_dir(&bo.id);
        fs::create_dir_all(home.join("workspace")).unwrap();
        fs::create_dir_all(&theirs).unwrap();
        fs::write(home.join("workspace/report.pdf"), "%PDF").unwrap();
        fs::write(home.join("workspace/broken.fail"), "x").unwrap();
        fs::write(theirs.join("draft.md"), "Bo's draft").unwrap();
        std::os::unix::fs::symlink(o.dir.join("outside.md"), home.join("link.md")).unwrap();
        let real = |p: PathBuf| fs::canonicalize(p).unwrap().to_string_lossy().into_owned();
        let open = |target: &str| office.open_tool(&talking.id, &ada, target);

        // Only where Pat is in the run: a routine, or a check back in the conversation, has nobody watching.
        for unseen in [&sweep, &check] {
            let err = office.open_tool(&unseen.id, &ada, "https://acme.example/sign-in").unwrap_err().message;
            assert_eq!(err, "Pat is not in this run, so nothing opens on their screen. Put the link on your card instead: a sign-in page goes in the chore's url.");
        }
        assert!(lock(&screen.0).is_empty());

        assert_eq!(open("https://acme.example/sign-in").unwrap(), json!({ "opened": true, "note": "It is open on Pat's screen." }));
        open(&home.join("workspace/report.pdf").to_string_lossy()).unwrap();
        open("workspace").unwrap();
        open(&repo.join("notes.md").to_string_lossy()).unwrap();
        open(&o.dir.join("employees").join("CLAUDE.md").to_string_lossy()).unwrap();
        open(&theirs.join("draft.md").to_string_lossy()).unwrap();
        let want = vec![
            "https://acme.example/sign-in".to_string(),
            real(home.join("workspace/report.pdf")),
            real(home.join("workspace")),
            real(repo.join("notes.md")),
            real(o.dir.join("employees").join("CLAUDE.md")),
            real(theirs.join("draft.md")),
        ];
        assert_eq!(*lock(&screen.0), want);

        for (target, why) in [
            ("", "Pass the full path of what to open"),
            ("http://acme.example", "Nothing is at 'http://acme.example'"),
            ("workspace/gone.pdf", "Nothing is at"),
            ("../../outside.md", "You can open what is in your folder"),
            ("link.md", "You can open what is in your folder"),
        ] {
            assert!(open(target).unwrap_err().message.starts_with(why), "{target}");
        }
        assert_eq!(open("workspace/broken.fail").unwrap_err().message, "It did not open: its app is gone. Tell Pat where it is instead.");
        assert_eq!(lock(&screen.0).len(), want.len(), "nothing it refused reached the screen");
        drop(office);

        let office = held_office(&o).await;
        assert!(office.open_tool(&talking.id, &ada, "workspace").unwrap_err().message.starts_with("It did not open: there is no screen to open it on."));
        drop(office);
    }
}
