//! The reads only the phone uses (`remote::api`): the Inbox rows, the team without its pictures, one picture at a
//! time, and a card's image. Plus hiring a proposal exactly as proposed, since the phone has no hire sheet.

use super::*;
use base64::Engine;
use serde::Serialize;

/// You, as a member of the team: the id the interface uses for you (`YOU_ID` in `lib/org.ts`).
pub const YOU_ID: &str = "you";
/// The largest card image the phone is sent, so a sealed frame stays under the relay's 4 MiB.
const CARD_IMAGE_BYTES: u64 = 2_500_000;

/// One member of the team, as the phone lists them: everything but the picture, which it fetches once
/// (`member_image`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMember {
    pub id: String,
    pub name: String,
    pub title: String,
    pub hue: u16,
    pub has_image: bool,
    pub activity: Option<Activity>,
}

/// The longest `what` the phone is sent, so `team` stays small whatever anyone typed.
const WHAT_CHARS: usize = 120;

/// What an employee is doing, as the org chart's status line says it (`employeeState` in `lib/memberState.ts`), with
/// the run it is about, which the phone opens with `thread`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Activity {
    Working { run_id: String, what: String },
    Queued { run_id: String, what: String },
    Attention { run_id: String, reason: Attention },
    Idle { next_run: Option<Ts> },
}

/// Why an employee's last run needs you.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Attention {
    NeedsYourOk,
    Interrupted,
    Failed,
}

impl From<&EmployeeView> for Activity {
    fn from(v: &EmployeeView) -> Activity {
        let live = |state| v.live.iter().find(|r| r.state == state);
        if let Some(run) = live(RunState::Running) {
            Activity::Working { run_id: run.id.clone(), what: what(run) }
        } else if let Some(run) = live(RunState::Queued) {
            Activity::Queued { run_id: run.id.clone(), what: what(run) }
        } else if let (Presence::Attention, Some(run)) = (v.presence, &v.last_run) {
            let reason = if !run.denials.is_empty() {
                Attention::NeedsYourOk
            } else if run.state == RunState::Lost {
                Attention::Interrupted
            } else {
                Attention::Failed
            };
            Activity::Attention { run_id: run.id.clone(), reason }
        } else {
            Activity::Idle { next_run: v.next_fire }
        }
    }
}

/// A short name for what a run is about, on one line of at most `WHAT_CHARS` characters, a long one ending in `…`: its
/// name, else a conversation's or a reply's label, else what was asked in its first sentence, else what kind of run
/// it is. Close to the org chart's `workTitle` (`lib/workTitle.ts`) but not word for word: it names no person or
/// colleague, drops more openers from what was asked ("ok", "hey, can you"), says "New task" for a blank one, names
/// other kinds in the words written for the employee ("A colleague's answer"), keeps up to 120, and capitalises.
fn what(run: &Run) -> String {
    let what = match (&run.title, &run.kind) {
        (Some(title), _) => title.clone(),
        (None, RunKind::Reply { label, .. } | RunKind::Task { label, .. }) => label.clone(),
        (None, RunKind::Manual) if run.prompt == DEFAULT_PROMPT => threads::report_label(&run.kind),
        (None, RunKind::Manual | RunKind::Delegated { .. } | RunKind::Asked { .. }) => threads::task_label(&run.prompt, &run.attachments),
        // The org chart's words: `claude::kind_label` is written to the employee, so its "your skills" would read as yours.
        (None, RunKind::Tidy { reason: TidyReason::Approved, .. }) => "Tidy: approved changes".into(),
        (None, RunKind::Tidy { .. }) => "Tidy".into(),
        (None, kind) => threads::report_label(kind),
    };
    let what = claude::one_line(&what);
    let mut chars = what.chars();
    let what: String = chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default();
    if what.chars().count() <= WHAT_CHARS {
        return what;
    }
    let cut: String = what.chars().take(WHAT_CHARS - 1).collect();
    format!("{}…", cut.trim_end_matches(|c: char| c == '…' || c.is_whitespace()))
}

/// The Inbox, as the phone shows it.
#[derive(Debug, Clone, Serialize)]
pub struct InboxView {
    pub threads: Vec<Thread>,
}

impl Office {
    /// The threads that lead an Inbox row, in the Inbox's order.
    pub fn inbox(&self) -> Result<InboxView> {
        let Conversations { threads, inbox } = self.conversations()?;
        let mut by_id: HashMap<String, Thread> = threads.into_iter().map(|t| (t.root.id.clone(), t)).collect();
        Ok(InboxView { threads: inbox.iter().filter_map(|id| by_id.remove(id)).collect() })
    }

    /// The company's name, as pairing tells the phone.
    pub fn company_name(&self) -> Result<String> {
        Ok(self.db().company()?.unwrap_or_else(Company::unset).name)
    }

    /// You, your employees with what each is doing, and the people you added, without their pictures.
    pub fn members(&self) -> Result<Vec<TeamMember>> {
        let OrgSnapshot { company, employees, people, .. } = self.org()?;
        let you = TeamMember { id: YOU_ID.into(), name: company.ceo_name, title: company.title, hue: 0, has_image: company.image.is_some(), activity: None };
        let employees = employees.into_iter().map(|v| {
            let activity = Some(Activity::from(&v));
            let e = v.employee;
            TeamMember { id: e.id, name: e.name, title: e.title, hue: e.hue, has_image: e.image.is_some(), activity }
        });
        let people = people.into_iter().map(|p| p.person).map(|p| TeamMember { id: p.id, name: p.name, title: p.title, hue: p.hue, has_image: p.image.is_some(), activity: None });
        Ok(std::iter::once(you).chain(employees).chain(people).collect())
    }

    /// The picture of member `id` (`YOU_ID` for you), as the data URL it is kept as.
    pub fn member_image(&self, id: &str) -> Result<String> {
        let db = self.db();
        let image = if id == YOU_ID {
            db.company()?.and_then(|c| c.image)
        } else if let Some(e) = db.employee(id)? {
            e.image
        } else {
            db.person(id)?.ok_or_else(|| AppError::not_found("No one on the team has that id."))?.image
        };
        image.ok_or_else(|| AppError::not_found("They have no picture."))
    }

    /// Image `index` of card `card` on a run, as a data URL. The path comes from the card itself, never from the asker.
    pub fn card_image(&self, run_id: &str, card: usize, index: usize) -> Result<String> {
        let run = self.db().run(run_id)?.ok_or_else(|| AppError::not_found("run not found"))?.run;
        let path = run.cards.get(card).and_then(|c| c.images.get(index).cloned()).ok_or_else(|| AppError::invalid("That card has no such image."))?;
        // Still the file the card was given, not a link put in its place since.
        if fs::canonicalize(&path).ok().as_deref() != Some(Path::new(&path)) {
            return Err(AppError::not_found(format!("Nothing is at {path} any more.")));
        }
        let mime = image_mime(Path::new(&path)).ok_or_else(|| AppError::invalid(format!("{path} is not a local image")))?;
        let bytes = read_file(Path::new(&path), CARD_IMAGE_BYTES)?
            .ok_or_else(|| AppError::invalid("That image is too large to send to the phone. Open it on the computer."))?;
        Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
    }

    /// Hires from proposal `proposal_id` exactly as proposed, as the hire sheet does when you change nothing.
    pub fn approve_proposal_as_proposed(&self, proposal_id: &str, note: Option<String>) -> Result<Employee> {
        let p = self.db().proposal(proposal_id)?.ok_or_else(|| AppError::not_found("proposal not found"))?;
        self.approve_proposal(proposal_id, p.draft, vec![], note)
    }
}
