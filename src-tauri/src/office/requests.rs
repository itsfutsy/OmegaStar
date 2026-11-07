//! Requests to real people, and watching their channels for the reply.

use super::*;

/// The failed check in a row that sends you one desktop notification.
const FAILURES_TO_NOTIFY: u32 = 3;

impl Office {
    pub fn requests(&self, person_id: Option<String>, employee_id: Option<String>, state: Option<String>) -> Result<Vec<HumanRequest>> {
        let state = state.as_deref().map(RequestState::parse).transpose()?;
        let watch = self.settings()?.watch;
        let mut v = self.db().requests(&RequestFilter { person_id, employee_id, state })?;
        for r in &mut v {
            Self::present_with(&watch, r);
        }
        Ok(v)
    }

    /// Records an ask of a person, made by an employee (from a run, via MCP). An FYI needs no reply,
    /// so it is done as soon as it is recorded. The arguments are the request tool's, one to one.
    #[allow(clippy::too_many_arguments)]
    pub fn create_request(
        &self,
        employee_id: &str,
        run_id: Option<&str>,
        person: &str,
        kind: RequestKind,
        message: &str,
        channel_id: Option<&str>,
        due_at: Option<Ts>,
    ) -> Result<(HumanRequest, Person)> {
        if message.trim().is_empty() {
            return Err(AppError::invalid("message is empty"));
        }
        let (req, p) = {
            let db = self.db();
            let people = db.people()?;
            let p = find_member(&people, person, |p| (&p.id, None, &p.name))
                .ok_or_else(|| AppError::not_found(format!("no person '{person}'. Call people() for the list.")))?
                .clone();
            let channel_id = channel_id.filter(|c| !c.trim().is_empty());
            if let Some(c) = channel_id {
                if !p.channels.iter().any(|x| x.id == c) {
                    return Err(AppError::invalid(format!("{} has no channel '{c}'", p.name)));
                }
            }
            let info = kind == RequestKind::Info;
            let at = now();
            let req = HumanRequest {
                id: uuid::Uuid::new_v4().to_string(),
                person_id: p.id.clone(),
                employee_id: employee_id.to_string(),
                run_id: run_id.map(String::from),
                kind,
                message: message.trim().to_string(),
                due_at,
                channel_id: channel_id.map(String::from),
                state: if info { RequestState::Done } else { RequestState::Open },
                response: None,
                created_at: at,
                resolved_at: info.then_some(at),
                delivery: None,
            };
            db.save_request(&req)?;
            (req, p)
        };
        if let Some(run_id) = run_id {
            self.append_event(
                run_id,
                RunEvent::Requested { request_id: req.id.clone(), person_id: p.id.clone(), kind, message: req.message.clone() },
            );
        }
        self.announce(&req);
        self.sink().org_changed();
        Ok((req, p))
    }

    /// Moves a request to answered/done/cancelled. The first reply (answered/done with a response)
    /// enqueues exactly one follow-up run for the requester, however many times this is called, unless
    /// the requester recorded it (`recorded_by`): that run already holds the reply and carries on itself.
    /// The follow-up resumes the run that made the request when it can (`origin_session`).
    pub fn resolve_request(&self, request_id: &str, state: &str, response: Option<String>, recorded_by: Option<&str>) -> Result<HumanRequest> {
        let target = RequestState::parse(state)?;
        if target == RequestState::Open {
            return Err(AppError::invalid("a request cannot be reopened"));
        }
        let (req, follow_up) = {
            let db = self.db();
            let mut r = db.request(request_id)?.ok_or_else(|| AppError::not_found("request not found"))?;
            if r.kind == RequestKind::Info {
                return Err(AppError::invalid("an FYI needs no reply, so it cannot be answered or closed"));
            }
            let had_reply = matches!(r.state, RequestState::Answered | RequestState::Done) && r.response.is_some();
            let by_requester = recorded_by == Some(r.employee_id.as_str());
            let terminal = matches!(r.state, RequestState::Done | RequestState::Cancelled);
            if terminal && r.state != target {
                return Err(AppError::conflict(format!("request is already {}", r.state.as_str())));
            }
            if let Some(resp) = response.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
                r.response = Some(resp);
            }
            if r.state != target {
                r.state = target;
                r.resolved_at = Some(now());
            }
            db.save_request(&r)?;
            let replied = matches!(r.state, RequestState::Answered | RequestState::Done) && r.response.is_some();
            let resume = replied && !had_reply && !by_requester;
            let follow_up = match (resume, db.employee(&r.employee_id)?, db.person(&r.person_id)?) {
                (true, Some(_), person) => {
                    let who = person.map(|p| p.name).unwrap_or_else(|| "A person".into());
                    let ceo = ceo_of(&db)?;
                    let kind = r.kind.as_str();
                    let mut run = Run::new(&r.employee_id, RunKind::FollowUp { request_id: r.id.clone(), person_id: r.person_id.clone() }, String::new(), now());
                    let (session, quote) = Self::origin_session(&db, r.run_id.as_deref(), &run.id)?;
                    let heard = r.delivery.as_ref().and_then(|d| d.watch.as_ref()).and_then(|w| w.heard.as_ref());
                    let judged = heard.filter(|h| matches!(h.verdict, Verdict::Answered | Verdict::Done | Verdict::AskedBack));
                    let mut read = String::new();
                    if let Some(h) = judged.filter(|h| !h.note.trim().is_empty()) {
                        read += &format!("\n\nOmega Star read their reply as: {}", h.note.trim());
                    }
                    if judged.is_some_and(|h| h.verdict == Verdict::AskedBack) {
                        read += "\n\nThey asked something back: answer it on the same thread, then record a new request and `sent` for what you still need.";
                    }
                    let origin = quote.map(|q| format!("You were talking with {ceo}, whose message was:\n\n{q}\n\n")).unwrap_or_default();
                    run.prompt = format!(
                        "{who} replied to your {kind} (request `{}`, asked {}).\n\nYour original ask:\n{}\n\nTheir response:\n{}{read}\n\n{origin}\
Continue the work that was waiting on this within your standing rules, then report. If a rule says {ceo} \
has to start that work, hand it back with what they need to start it instead of doing it.",
                        r.id,
                        r.created_at.format("%Y-%m-%d"),
                        r.message,
                        r.response.as_deref().unwrap_or("")
                    );
                    db.insert_run(&run, &session)?.then_some(run)
                }
                _ => None,
            };
            (r, follow_up)
        };
        self.announce(&req);
        if let Some(run) = follow_up {
            self.sink().run_updated(&run);
            self.pump();
        }
        self.sink().org_changed();
        Ok(req)
    }

    // ------------------------------------------------------------------ watching requests for replies

    /// Fills a request's watch plan, which is derived and never stored.
    pub(super) fn present_with(settings: &WatchSettings, r: &mut HumanRequest) {
        let plan = watch::Facts::of(r).map(|f| watch::plan(&f, settings, &Local));
        if let Some(w) = watch_of(r) {
            w.plan = plan;
        }
    }

    pub(super) fn present(&self, mut r: HumanRequest) -> HumanRequest {
        Self::present_with(&self.settings().map(|s| s.watch).unwrap_or_default(), &mut r);
        r
    }

    /// The one way a changed request reaches the UI. Callers hold no lock.
    pub(super) fn announce(&self, r: &HumanRequest) {
        self.sink().request_updated(&self.present(r.clone()));
    }

    /// The session a run about a request continues: the asking run's own when it was a work run whose
    /// session started; otherwise `fresh`. An ask from a turn of an older build's chat starts fresh, with your
    /// message there to quote; a turn Omega Star wrote, such as a colleague's report coming back, is not yours.
    fn origin_session(db: &Store, origin_run_id: Option<&str>, fresh: &str) -> Result<(String, Option<String>)> {
        let Some(rec) = origin_run_id.map(|id| db.run(id)).transpose()?.flatten() else {
            return Ok((fresh.to_string(), None));
        };
        Ok(match rec.run.kind.lane() {
            Lane::Work if db.session_started(&rec.session)? => (rec.session, None),
            Lane::Work => (fresh.to_string(), None),
            Lane::Chat => {
                let yours = matches!(rec.run.kind, RunKind::Chat) && !rec.run.omega_wrote;
                (fresh.to_string(), yours.then(|| claude::clip(&rec.run.prompt, 2000)))
            }
        })
    }

    pub(super) fn person_name(db: &Store, person_id: &str) -> Result<String> {
        Ok(db.person(person_id)?.map(|p| p.name).unwrap_or_else(|| "the person".into()))
    }

    fn employee_name(db: &Store, employee_id: &str) -> Result<String> {
        Ok(db.employee(employee_id)?.map(|e| e.name).unwrap_or_else(|| "An employee".into()))
    }

    /// What a run about a request starts from: who it went to, when, and the ask.
    fn ask_brief(r: &HumanRequest, person: &str) -> String {
        let sent = r.delivery.as_ref().map_or(r.created_at, |d| d.sent_at);
        let due = r.due_at.map(|d| format!(", needed by {}", local_time(d))).unwrap_or_default();
        format!("Your {} to {person} (request `{}`), sent {}{due}:\n\n{}", r.kind.as_str(), r.id, local_time(sent), r.message)
    }

    /// Queues a reply check or a nudge about a request, in a fresh session.
    fn queue_watch_run(db: &Store, r: &HumanRequest, nudge: bool, at: Ts) -> Result<Run> {
        let (request_id, person_id) = (r.id.clone(), r.person_id.clone());
        let kind = if nudge { RunKind::Nudge { request_id, person_id } } else { RunKind::ReplyCheck { request_id, person_id } };
        let run = Run::new(&r.employee_id, kind, Self::ask_brief(r, &Self::person_name(db, &r.person_id)?), at);
        db.insert_run(&run, &run.id)?;
        Ok(run)
    }

    /// A Nudge run now; the request remembers it was nudged. The automatic one never follows a Nudge
    /// run, even one whose request did not get to remember it.
    fn nudge(db: &Store, r: &mut HumanRequest, at: Ts) -> Result<Run> {
        let run = Self::queue_watch_run(db, r, true, at)?;
        if let Some(w) = watch_of(r) {
            w.nudged_at = Some(at);
        }
        db.save_request(r)?;
        Ok(run)
    }

    fn nudge_notice(db: &Store, r: &HumanRequest) -> Result<(String, String)> {
        let (employee, person) = (Self::employee_name(db, &r.employee_id)?, Self::person_name(db, &r.person_id)?);
        Ok((format!("{employee} is nudging {person}"), format!("About: {}", claude::clip(&r.message, 160))))
    }

    /// Stops watching a request nobody answered and, once per request, starts the run that tells its
    /// employee to go on without it.
    fn give_up(db: &Store, r: &mut HumanRequest, at: Ts) -> Result<(Option<Run>, (String, String))> {
        let (employee, person) = (Self::employee_name(db, &r.employee_id)?, Self::person_name(db, &r.person_id)?);
        let run = if db.has_run_for(&r.id, "unanswered")? || db.employee(&r.employee_id)?.is_none() {
            None
        } else {
            let kind = RunKind::Unanswered { request_id: r.id.clone(), person_id: r.person_id.clone() };
            let mut run = Run::new(&r.employee_id, kind, Self::ask_brief(r, &person), at);
            let (session, quote) = Self::origin_session(db, r.run_id.as_deref(), &run.id)?;
            if let Some(q) = quote {
                run.prompt += &format!("\n\nYou were talking with {}, whose message was:\n\n{q}", ceo_of(db)?);
            }
            db.insert_run(&run, &session)?.then_some(run)
        };
        if let Some(w) = watch_of(r) {
            w.phase = WatchPhase::GaveUp { at };
        }
        db.save_request(r)?;
        let ask = claude::clip(&r.message, 120);
        let body = if run.is_some() { format!("{employee} will go on without it: {ask}") } else { format!("Omega Star stopped watching again: {ask}") };
        Ok((run, (format!("No reply from {person}"), body)))
    }

    /// Announces what a pass changed, tells you what it should, and starts the runs it queued.
    fn publish(&self, changed: &[HumanRequest], runs: &[Run], notices: &[(String, String)]) {
        for r in changed {
            self.announce(r);
        }
        for run in runs {
            self.sink().run_updated(run);
        }
        for (title, body) in notices {
            self.sink().notify(title, body);
        }
        if !runs.is_empty() {
            self.pump();
            self.sink().org_changed();
        }
    }

    /// One pass of the reply monitor at `at`. What each watched request needs is decided under the
    /// lock; Teams reads and judging happen without it. Employees with a running run are left alone,
    /// so the Office never races their own run on a reply.
    pub async fn watch_tick(&self, at: Ts) -> Result<()> {
        let Ok(_watching) = self.0.watching.try_lock() else { return Ok(()) };
        let settings = self.settings()?;
        if settings.paused {
            return Ok(());
        }
        let s = settings.watch;
        let model_may_run = watch::in_hours(at, &s.hours, &Local) && tidy::room_for_a_tidy(&self.usage_limits());
        let (mut changed, mut runs, mut notices, mut reads) = (vec![], vec![], vec![], vec![]);
        {
            let db = self.db();
            let busy: HashSet<String> = db.runs_in_state(RunState::Running)?.into_iter().map(|r| r.employee_id).collect();
            for mut r in db.watched_requests()? {
                let Some(f) = watch::Facts::of(&r) else { continue };
                if busy.contains(&r.employee_id) {
                    continue;
                }
                let plan = watch::plan(&f, &s, &Local);
                let due = plan.due(at);
                let may_nudge = due == watch::Due::Nudge && model_may_run && !db.has_run_for(&r.id, "nudge")?;
                let backing_off = f.failures > 0 && at < plan.next_check_at;
                match due {
                    watch::Due::Nothing => {}
                    watch::Due::GiveUp | watch::Due::Nudge if f.native && backing_off => {}
                    watch::Due::GiveUp if f.native => reads.push((r.id.clone(), After::GiveUp)),
                    watch::Due::GiveUp => {
                        let (run, notice) = Self::give_up(&db, &mut r, at)?;
                        runs.extend(run);
                        notices.push(notice);
                        changed.push(r);
                    }
                    watch::Due::Nudge if may_nudge && f.native => reads.push((r.id.clone(), After::Nudge)),
                    watch::Due::Nudge if may_nudge => {
                        notices.push(Self::nudge_notice(&db, &r)?);
                        runs.push(Self::nudge(&db, &mut r, at)?);
                        changed.push(r);
                    }
                    _ if at < plan.next_check_at => {}
                    _ if f.native => reads.push((r.id.clone(), After::Nothing)),
                    _ => {
                        if model_may_run && db.active_run_for(&r.id, "replyCheck")?.is_none() {
                            runs.push(Self::queue_watch_run(&db, &r, false, at)?);
                            if let Some(w) = watch_of(&mut r) {
                                w.last_checked_at = Some(at);
                            }
                            db.save_request(&r)?;
                            changed.push(r);
                        }
                    }
                }
            }
        }
        self.publish(&changed, &runs, &notices);
        self.read_and_judge(reads, at, true).await
    }

    async fn read_group(&self, employee_id: &str, server: &str, chats: &[String]) -> Vec<std::result::Result<Vec<ChatMessage>, ReadError>> {
        let every = |e: ReadError| chats.iter().map(|_| Err(e.clone())).collect();
        match reader::load_server(&self.employee_dir(employee_id), employee_id, server) {
            Ok(Some(spec)) => reader::read_teams(&spec, chats).await,
            Ok(None) => every(ReadError::Unsupported(format!("{server} is not one of this employee's local MCP servers"))),
            Err(e) => every(ReadError::Failed(e)),
        }
    }

    fn running(db: &Store, employee_id: &str) -> Result<bool> {
        Ok(db.active_runs_of(employee_id)?.iter().any(|r| r.state == RunState::Running))
    }

    /// Reads these requests' Teams chats, judges what the person wrote, applies each verdict, and then
    /// does what each still waiting request is owed: its nudge or its give-up, if still due and only
    /// once its chat was read and every reply in it judged. With
    /// `wait_for_runs` (the monitor), nothing is applied for an employee whose own run started
    /// meanwhile; Check now applies regardless.
    async fn read_and_judge(&self, reads: Vec<(String, After)>, at: Ts, wait_for_runs: bool) -> Result<()> {
        struct Item {
            id: String,
            employee: String,
            server: String,
            chat: String,
            anchor: String,
            after: After,
        }
        if reads.is_empty() {
            return Ok(());
        }
        let settings = self.settings()?;
        let items: Vec<Item> = {
            let db = self.db();
            let mut v = vec![];
            for (id, after) in reads {
                let Some(r) = db.request(&id)? else { continue };
                if let Some(Delivery { locator: Locator::TeamsChat { chat_id, message_id, server: Some(server) }, .. }) = &r.delivery {
                    v.push(Item { employee: r.employee_id.clone(), server: server.clone(), chat: chat_id.clone(), anchor: message_id.clone(), id, after });
                }
            }
            v
        };

        // A server takes seconds to start, so one start serves every chat of an employee on it.
        let mut groups: Vec<((String, String), Vec<String>)> = vec![];
        for it in &items {
            let key = (it.employee.clone(), it.server.clone());
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, chats)) if chats.contains(&it.chat) => {}
                Some((_, chats)) => chats.push(it.chat.clone()),
                None => groups.push((key, vec![it.chat.clone()])),
            }
        }
        let mut read: HashMap<(String, String, String), std::result::Result<Vec<ChatMessage>, ReadError>> = HashMap::new();
        for pair in groups.chunks(2) {
            let results = match pair {
                [a, b] => {
                    let (x, y) = tokio::join!(self.read_group(&a.0 .0, &a.0 .1, &a.1), self.read_group(&b.0 .0, &b.0 .1, &b.1));
                    vec![x, y]
                }
                [a] => vec![self.read_group(&a.0 .0, &a.0 .1, &a.1).await],
                _ => vec![],
            };
            for (((employee, server), chats), got) in pair.iter().zip(results) {
                for (chat, result) in chats.iter().zip(got) {
                    read.insert((employee.clone(), server.clone(), chat.clone()), result);
                }
            }
        }

        struct Judging {
            id: String,
            anchor: String,
            request: HumanRequest,
            messages: Vec<ChatMessage>,
            replies: Vec<ChatMessage>,
            employee: String,
            person: String,
        }
        let (mut changed, mut notices, mut judging) = (vec![], vec![], vec![]);
        let mut settled: HashSet<String> = HashSet::new();
        {
            let db = self.db();
            for it in &items {
                let Some(result) = read.get(&(it.employee.clone(), it.server.clone(), it.chat.clone())).cloned() else { continue };
                let Some(mut r) = db.request(&it.id)? else { continue };
                if !still_watching(&r, &it.anchor) {
                    continue;
                }
                let person = Self::person_name(&db, &r.person_id)?;
                match result {
                    Ok(messages) => {
                        let replies = match &r.delivery {
                            Some(d) => watch::replies(&messages, d, d.watch.as_ref().and_then(|w| w.seen.as_ref())),
                            None => vec![],
                        };
                        let Some(w) = watch_of(&mut r) else { continue };
                        w.last_checked_at = Some(at);
                        // With replies to judge, the check is not done until the judge answers.
                        if replies.is_empty() {
                            w.failures = 0;
                            w.last_error = None;
                            settled.insert(it.id.clone());
                        } else {
                            let employee = Self::employee_name(&db, &r.employee_id)?;
                            judging.push(Judging { id: it.id.clone(), anchor: it.anchor.clone(), request: r.clone(), messages, replies, employee, person });
                        }
                    }
                    Err(ReadError::Unsupported(m)) => {
                        if let Some(Delivery { locator: Locator::TeamsChat { server, .. }, .. }) = r.delivery.as_mut() {
                            *server = None;
                        }
                        let Some(w) = watch_of(&mut r) else { continue };
                        w.last_checked_at = Some(at);
                        w.last_error = Some(format!("{m}, so a model checks for replies instead."));
                    }
                    Err(ReadError::Failed(m)) => {
                        let Some(w) = watch_of(&mut r) else { continue };
                        w.last_checked_at = Some(at);
                        if fail_check(w, m.clone()) {
                            notices.push((format!("Omega Star cannot check {person}'s replies"), m));
                        }
                    }
                }
                db.save_request(&r)?;
                changed.push(r);
            }
        }
        self.publish(&changed, &[], &notices);

        let program = claude::locate(settings.claude_path.as_deref());
        let proxy_env = self.proxy_env(&settings.proxy);
        let cwd = self.0.data_dir.join("judge");
        if !judging.is_empty() {
            fs::create_dir_all(&cwd)?;
        }
        let mut resolves = vec![];
        for j in judging {
            let Some(d) = j.request.delivery.as_ref() else { continue };
            let sender = watch::sender_of(&j.messages, d).map(String::from);
            let seen = d.watch.as_ref().and_then(|w| w.seen.as_ref());
            let said: Vec<claude::Said> = watch::after_ask(&j.messages, d, seen)
                .into_iter()
                .map(|m| claude::Said { sender: sender.is_some() && m.from_id == sender, at: m.at, from: m.from_name, text: m.text })
                .collect();
            let prompt = claude::judge_prompt(&j.request.message, j.request.kind, &j.employee, &j.person, d.sent_at, &said);
            let verdict = match &program {
                Some(p) => claude::judge(p, &proxy_env, &cwd, &prompt).await,
                None => Err(AppError::invalid("Claude Code was not found")),
            };
            let Some(newest) = j.replies.last() else { continue };
            let through = Seen { id: newest.id.clone(), at: newest.at };
            let words = j.replies.iter().map(|m| m.text.as_str()).collect::<Vec<_>>().join("\n");
            let response = watch::replies(&j.messages, d, None).iter().map(|m| format!("{}: {}", local_time(m.at), m.text)).collect::<Vec<_>>().join("\n");
            let (applied, judged, r, notice) = {
                let db = self.db();
                let Some(mut r) = db.request(&j.id)? else { continue };
                if !still_watching(&r, &j.anchor) || (wait_for_runs && Self::running(&db, &r.employee_id)?) {
                    continue;
                }
                let mut notice = None;
                let (applied, judged) = match verdict {
                    Ok((verdict, note)) => (watch::apply_verdict(&mut r, &through, Heard { at: through.at, from: j.person.clone(), text: words, verdict, note }, &response), true),
                    Err(e) => {
                        let Some(w) = watch_of(&mut r) else { continue };
                        let e = format!("The reply could not be judged: {}", e.message);
                        if fail_check(w, e.clone()) {
                            notice = Some((format!("Omega Star cannot check {}'s replies", j.person), e));
                        }
                        (Applied::Kept, false)
                    }
                };
                if applied != Applied::Stale {
                    db.save_request(&r)?;
                }
                (applied, judged, r, notice)
            };
            if applied != Applied::Stale {
                self.announce(&r);
            }
            if judged && applied != Applied::Stale {
                settled.insert(j.id.clone());
            }
            if let Some((title, body)) = notice {
                self.sink().notify(&title, &body);
            }
            if let Applied::Resolve { state, response } = applied {
                resolves.push((j.id, state, response));
            }
        }
        for (id, state, response) in resolves {
            if let Err(e) = self.resolve_request(&id, state.as_str(), Some(response), None) {
                log!("recording the reply to request {id} failed: {e}");
            }
        }

        let (mut changed, mut runs, mut notices) = (vec![], vec![], vec![]);
        {
            let db = self.db();
            for it in items.iter().filter(|i| i.after != After::Nothing && settled.contains(&i.id)) {
                let Some(mut r) = db.request(&it.id)? else { continue };
                let Some(f) = watch::Facts::of(&r) else { continue };
                if wait_for_runs && Self::running(&db, &r.employee_id)? {
                    continue;
                }
                // What was just read (an acknowledgement, say) can move the nudge and the give-up.
                match (it.after, watch::plan(&f, &settings.watch, &Local).due(at)) {
                    (After::Nudge, watch::Due::Nudge) if !db.has_run_for(&r.id, "nudge")? => {
                        notices.push(Self::nudge_notice(&db, &r)?);
                        runs.push(Self::nudge(&db, &mut r, at)?);
                    }
                    (After::GiveUp, watch::Due::GiveUp) => {
                        let (run, notice) = Self::give_up(&db, &mut r, at)?;
                        runs.extend(run);
                        notices.push(notice);
                    }
                    _ => continue,
                }
                changed.push(r);
            }
        }
        self.publish(&changed, &runs, &notices);
        Ok(())
    }

    /// Your hand on a watched request. Check now reads (or queues a reply check) whatever the cadence,
    /// hours, pause, usage or the employee's own runs; Nudge now queues a nudge; Stop and Resume flip
    /// the watch, and Resume restarts its clocks.
    pub async fn watch_request(&self, request_id: &str, action: WatchAction) -> Result<HumanRequest> {
        let at = now();
        let (r, run, read) = {
            let db = self.db();
            let mut r = db.request(request_id)?.ok_or_else(|| AppError::not_found("request not found"))?;
            if r.kind == RequestKind::Info {
                return Err(AppError::invalid("an FYI needs no reply, so it is not watched"));
            }
            if !r.state.is_open() {
                return Err(AppError::invalid(format!("this request is already {}", r.state.as_str())));
            }
            let (native, watching) = match &r.delivery {
                None => return Err(AppError::invalid("this request was not recorded as sent, so there is nothing to watch")),
                Some(d) => (
                    matches!(d.locator, Locator::TeamsChat { server: Some(_), .. }),
                    d.watch.as_ref().is_some_and(|w| w.phase == WatchPhase::Watching),
                ),
            };
            let mut run = None;
            match action {
                WatchAction::Check | WatchAction::Nudge if !watching => {
                    return Err(AppError::invalid("Omega Star is not watching this request: resume watching first"));
                }
                WatchAction::Check if native => {}
                WatchAction::Check => {
                    if db.active_run_for(&r.id, "replyCheck")?.is_some() {
                        return Err(AppError::conflict("a check for this request is already waiting or running"));
                    }
                    run = Some(Self::queue_watch_run(&db, &r, false, at)?);
                    if let Some(w) = watch_of(&mut r) {
                        w.last_checked_at = Some(at);
                    }
                }
                WatchAction::Nudge => {
                    if db.active_run_for(&r.id, "nudge")?.is_some() {
                        return Err(AppError::conflict("a nudge for this request is already waiting or running"));
                    }
                    run = Some(Self::nudge(&db, &mut r, at)?);
                }
                WatchAction::Stop => {
                    if let Some(w) = watch_of(&mut r) {
                        w.phase = WatchPhase::Stopped { at };
                    }
                }
                WatchAction::Resume => {
                    if let Some(Delivery { locator: Locator::TeamsChat { server, .. }, via, .. }) = r.delivery.as_mut() {
                        if server.is_none() {
                            *server = watch::server_of(via);
                        }
                    }
                    if let Some(w) = watch_of(&mut r) {
                        w.phase = WatchPhase::Watching;
                        w.resumed_at = Some(at);
                        w.failures = 0;
                        w.last_error = None;
                    }
                }
            }
            db.save_request(&r)?;
            (r, run, action == WatchAction::Check && native)
        };
        self.publish(std::slice::from_ref(&r), run.as_slice(), &[]);
        if read {
            self.read_and_judge(vec![(r.id.clone(), After::Nothing)], at, false).await?;
        }
        let r = self.db().request(request_id)?.ok_or_else(|| AppError::not_found("request not found"))?;
        Ok(self.present(r))
    }

    /// Records the deliveries a run's transcript shows for requests it made and did not report
    /// sending. `open_only`: when recovering a lost run, requests already closed are left as they are.
    pub(super) fn capture_deliveries(&self, run_id: &str, open_only: bool) -> Result<()> {
        let (mut undelivered, mut claimed) = (vec![], vec![]);
        for r in self.db().requests_of_run(run_id)? {
            match r.delivery {
                None => undelivered.push(r.id),
                Some(Delivery { locator: Locator::TeamsChat { message_id, .. }, .. }) => claimed.push(message_id),
                Some(_) => {}
            }
        }
        if undelivered.is_empty() {
            return Ok(());
        }
        let transcript = fs::read_to_string(self.run_dir(run_id).join("transcript.jsonl")).unwrap_or_default();
        let found = watch::capture(&transcript, &undelivered, &claimed);
        let mut changed = vec![];
        {
            let db = self.db();
            for (id, tool, result) in found {
                let Some(mut r) = db.request(&id)? else { continue };
                if r.delivery.is_some() || (open_only && !r.state.is_open()) {
                    continue;
                }
                r.delivery = Some(Delivery::new(watch::parse_sent(&tool, &result, now()), DeliverySource::Transcript, r.kind));
                db.save_request(&r)?;
                changed.push(r);
            }
        }
        for r in &changed {
            self.announce(r);
        }
        Ok(())
    }

    /// A reply check that ended without recording what it found counts as a failed check.
    pub(super) fn check_missed(&self, request_id: &str, error: String) -> Result<()> {
        let (r, notice) = {
            let db = self.db();
            let Some(mut r) = db.request(request_id)? else { return Ok(()) };
            if watch::Facts::of(&r).is_none() {
                return Ok(());
            }
            let person = Self::person_name(&db, &r.person_id)?;
            let Some(w) = watch_of(&mut r) else { return Ok(()) };
            let notice = fail_check(w, error.clone()).then(|| (format!("Omega Star cannot check {person}'s replies"), error));
            db.save_request(&r)?;
            (r, notice)
        };
        self.publish(&[r], &[], notice.as_slice());
        Ok(())
    }
}

/// What a pass owes a Teams-read request once its chat is read and judged, if it is still waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum After {
    Nothing,
    Nudge,
    GiveUp,
}

/// Still open, still watched, and still about the message that was read.
fn still_watching(r: &HumanRequest, anchor: &str) -> bool {
    watch::Facts::of(r).is_some() && matches!(&r.delivery, Some(Delivery { locator: Locator::TeamsChat { message_id, .. }, .. }) if message_id == anchor)
}

/// Counts a failed check. True for the failure that should tell you, once.
fn fail_check(w: &mut Watch, error: String) -> bool {
    w.failures += 1;
    w.last_error = Some(error);
    w.failures == FAILURES_TO_NOTIFY
}

/// A moment on your local clock: "Tue 6 Oct 1:15 pm".
fn local_time(t: Ts) -> String {
    t.with_timezone(&Local).format("%a %-d %b %-I:%M %P").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_your_own_chat_turn_is_quoted_for_a_request() {
        let dir = std::env::temp_dir().join(format!("omegastar-requests-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Store::open(&dir.join("office.db")).unwrap();
        let turn = Run::new("e1", RunKind::Chat, "Ask the supplier for a date.".into(), now());
        db.insert_run(&turn, "chat-e1").unwrap();
        let came = RunKind::Answered { asked_run_id: "r1".into(), from: "e2".into(), task: false, chat: true, thread: None };
        let mut answered = Run::new("e1", came, "Bo answered your question. Their answer: ask the supplier.".into(), now());
        answered.omega_wrote = true;
        db.insert_run(&answered, "chat-e1").unwrap();

        let quoted = |id: &str| Office::origin_session(&db, Some(id), "fresh").unwrap();
        assert_eq!(quoted(&turn.id), ("fresh".to_string(), Some("Ask the supplier for a date.".to_string())));
        assert_eq!(quoted(&answered.id), ("fresh".to_string(), None));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
