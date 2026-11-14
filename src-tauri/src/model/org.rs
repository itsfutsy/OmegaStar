//! The company: you, employees and their drafts, people, their faces, and hire proposals.

use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Company {
    pub name: String,
    pub ceo_name: String,
    /// Your title, "CEO" unless you set another.
    pub title: String,
    pub email: Option<String>,
    /// The person you report to; `None` = you are the top of the company.
    pub reports_to: Option<String>,
    /// Your display image (see `check_image`).
    #[serde(default)]
    pub image: Option<String>,
}

impl Company {
    const DEFAULT_TITLE: &'static str = "CEO";

    /// Before onboarding: no name, the default title, nobody above you.
    pub fn unset() -> Company {
        Company { name: String::new(), ceo_name: String::new(), title: Self::DEFAULT_TITLE.into(), email: None, reports_to: None, image: None }
    }

    pub fn title_or_default(title: Option<&str>) -> String {
        match title.map(str::trim) {
            Some(t) if !t.is_empty() => t.to_string(),
            _ => Self::DEFAULT_TITLE.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Clearance {
    Supervised,
    Autonomous,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::Xhigh => "xhigh",
            Effort::Max => "max",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillGrant {
    pub builtins: bool,
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmployeeDraft {
    pub name: String,
    pub title: String,
    pub description: String,
    pub objective: String,
    pub manager_id: Option<String>,
    pub skills: SkillGrant,
    pub clearance: Clearance,
    pub allow_rules: Vec<String>,
    pub model: Option<String>,
    pub effort: Option<Effort>,
    /// Lets them drive your Chrome (Claude in Chrome). Off unless you turn it on for a new hire.
    #[serde(default)]
    pub browser: bool,
    /// Tidies their skills and notes on their own when the health check finds something. On unless you turn it off.
    #[serde(default = "tidy_on")]
    pub auto_tidy: bool,
    pub workspaces: Vec<String>,
    pub hue: u16,
    /// Their display image (see `check_image`).
    #[serde(default)]
    pub image: Option<String>,
}

/// Employees hired before the browser switch existed could already drive Chrome, so they keep it.
fn browser_on() -> bool {
    true
}

/// Tidying on their own is on unless you turn it off, including for employees saved before it was.
fn tidy_on() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Employee {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub title: String,
    pub description: String,
    pub objective: String,
    pub manager_id: Option<String>,
    pub skills: SkillGrant,
    pub clearance: Clearance,
    pub allow_rules: Vec<String>,
    pub model: Option<String>,
    pub effort: Option<Effort>,
    #[serde(default = "browser_on")]
    pub browser: bool,
    /// On unless you turn it off.
    #[serde(default = "tidy_on")]
    pub auto_tidy: bool,
    pub workspaces: Vec<String>,
    pub hue: u16,
    /// Their display image, shown in place of their initials (see `check_image`). None: their initials on their colour.
    #[serde(default)]
    pub image: Option<String>,
    /// The session of the one long chat older builds kept. Nothing reads it now, but a hire still gets one of
    /// its own, so an older build reading this employee never shares a session with another.
    pub chat_session: String,
    pub hired_at: Ts,
    /// The front desk: Omega Star's own agent you chat to about the team, which passes messages on and does no work
    /// itself. It is not on the org chart, and colleagues never see it (`Store::employees` leaves it out).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub desk: bool,
}

/// Largest display image, as its data URL. The app makes them a few dozen KB at most.
const MAX_IMAGE_CHARS: usize = 256 * 1024;

/// A display image, which anyone on the chart (you, an employee, a person) may have in place of their initials: a
/// small picture as a `data:image/` URL, so the app never fetches it from anywhere.
pub fn check_image(image: Option<&str>) -> Result<()> {
    match image {
        Some(i) if !i.starts_with("data:image/") || i.len() > MAX_IMAGE_CHARS => {
            Err(AppError::invalid("the display image must be a picture under 256 KB"))
        }
        _ => Ok(()),
    }
}

fn check_face(name: &str, hue: u16, image: Option<&str>) -> Result<()> {
    if name.trim().is_empty() {
        return Err(AppError::invalid("name is required"));
    }
    if hue > 359 {
        return Err(AppError::invalid("hue must be 0-359"));
    }
    check_image(image)
}

impl EmployeeDraft {
    /// Boundary validation shared by hire and update.
    pub fn validate(&self) -> Result<()> {
        check_face(&self.name, self.hue, self.image.as_deref())
    }

    pub fn into_employee(self, id: String, slug: String, chat_session: String, hired_at: Ts) -> Employee {
        Employee {
            id,
            slug,
            name: self.name.trim().to_string(),
            title: self.title,
            description: self.description,
            objective: self.objective,
            manager_id: self.manager_id,
            skills: self.skills,
            clearance: self.clearance,
            allow_rules: self.allow_rules,
            model: self.model.filter(|m| !m.trim().is_empty()),
            effort: self.effort,
            browser: self.browser,
            auto_tidy: self.auto_tidy,
            workspaces: self.workspaces,
            hue: self.hue,
            image: self.image,
            chat_session,
            hired_at,
            desk: false,
        }
    }
}

impl Employee {
    pub fn reports_to(&self, manager_id: &str) -> bool {
        self.manager_id.as_deref() == Some(manager_id)
    }

    pub fn draft(&self) -> EmployeeDraft {
        EmployeeDraft {
            name: self.name.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            objective: self.objective.clone(),
            manager_id: self.manager_id.clone(),
            skills: self.skills.clone(),
            clearance: self.clearance,
            allow_rules: self.allow_rules.clone(),
            model: self.model.clone(),
            effort: self.effort,
            browser: self.browser,
            auto_tidy: self.auto_tidy,
            workspaces: self.workspaces.clone(),
            hue: self.hue,
            image: self.image.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Presence {
    Idle,
    Working,
    Queued,
    Attention,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmployeeView {
    #[serde(flatten)]
    pub employee: Employee,
    pub presence: Presence,
    /// What they are working on: every queued or running run of theirs, running first, oldest first within each.
    pub live: Vec<Run>,
    /// How many runs they have going at once.
    pub working: u32,
    pub next_fire: Option<Ts>,
    /// Their latest work-lane run. `Presence::Attention` is decided from it, so it stays work-only.
    pub last_run: Option<Run>,
    /// When they last finished anything, in either lane: what "last worked" means.
    pub last_worked_at: Option<Ts>,
    pub status: EmployeeStatus,
}

/// How an employee is doing at a glance, for the org chart and their panel.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmployeeStatus {
    /// Their latest weekly review's number, as "metric value (change)".
    pub number: Option<String>,
    /// The first line of their last finished report, and when it finished.
    pub last_report: Option<Dated>,
    /// What they do next: their next schedule slot's label or their soonest open check, and when.
    pub next: Option<Dated>,
}

/// A line of text and the time it is about.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Dated {
    pub text: String,
    pub at: Ts,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ChannelKind {
    Teams,
    Slack,
    Email,
    Jira,
    Asana,
    AzureDevOps,
    Github,
    Linear,
    Phone,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    /// Generated server-side when the client leaves it empty.
    #[serde(default)]
    pub id: String,
    pub kind: ChannelKind,
    pub address: String,
    pub when_to_use: String,
}

/// Who a person reports to. Employees need no `Top`: only people can sit above you.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PersonManager {
    /// You, the user.
    You,
    /// An employee or another person.
    Member { id: String },
    /// Nobody: the top of the company. Only the person at the top of the line above you.
    Top,
}

impl PersonManager {
    pub fn member_id(&self) -> Option<&str> {
        match self {
            PersonManager::Member { id } => Some(id),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersonDraft {
    pub name: String,
    pub title: String,
    pub description: String,
    pub strengths: String,
    pub responsibilities: String,
    pub channels: Vec<Channel>,
    pub notes: String,
    pub manager: PersonManager,
    pub hue: u16,
    /// Their display image (see `check_image`).
    #[serde(default)]
    pub image: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub id: String,
    pub name: String,
    pub title: String,
    pub description: String,
    pub strengths: String,
    pub responsibilities: String,
    pub channels: Vec<Channel>,
    pub notes: String,
    pub manager: PersonManager,
    pub hue: u16,
    /// Their display image, shown in place of their initials (see `check_image`). None: their initials.
    #[serde(default)]
    pub image: Option<String>,
    pub added_at: Ts,
}

impl PersonDraft {
    pub fn validate(&self) -> Result<()> {
        check_face(&self.name, self.hue, self.image.as_deref())
    }

    pub fn into_person(self, id: String, added_at: Ts) -> Person {
        Person {
            id,
            name: self.name.trim().to_string(),
            title: self.title,
            description: self.description,
            strengths: self.strengths,
            responsibilities: self.responsibilities,
            channels: self.channels.into_iter().map(|mut c| {
                if c.id.trim().is_empty() {
                    c.id = uuid::Uuid::new_v4().to_string();
                }
                c
            }).collect(),
            notes: self.notes,
            manager: self.manager,
            hue: self.hue,
            image: self.image,
            added_at,
        }
    }
}

impl Person {
    pub fn draft(&self) -> PersonDraft {
        PersonDraft {
            name: self.name.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            strengths: self.strengths.clone(),
            responsibilities: self.responsibilities.clone(),
            channels: self.channels.clone(),
            notes: self.notes.clone(),
            manager: self.manager.clone(),
            hue: self.hue,
            image: self.image.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonView {
    #[serde(flatten)]
    pub person: Person,
    pub open_requests: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProposalState {
    Open,
    Approved,
    Declined,
    /// The proposer took it back, or left the company.
    Withdrawn,
}

impl ProposalState {
    pub fn as_str(self) -> &'static str {
        match self {
            ProposalState::Open => "open",
            ProposalState::Approved => "approved",
            ProposalState::Declined => "declined",
            ProposalState::Withdrawn => "withdrawn",
        }
    }
}

/// A hire an employee proposed (`propose_hire`). You decide: approve it, as proposed or changed,
/// which hires the employee, or decline it. Either way the proposer hears back in a new run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HireProposal {
    pub id: String,
    /// The employee who proposed it.
    pub employee_id: String,
    pub run_id: Option<String>,
    /// The gap the hire fills, in the proposer's words.
    pub reason: String,
    /// The hire as proposed: supervised, reporting to the proposer or one of their direct reports.
    pub draft: EmployeeDraft,
    /// Skills they asked for that are not in your catalog.
    pub missing_skills: Vec<String>,
    pub state: ProposalState,
    /// Your note to the proposer when you decided.
    pub note: Option<String>,
    /// The employee hired when you approved it.
    pub hired_id: Option<String>,
    pub created_at: Ts,
    pub resolved_at: Option<Ts>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgSnapshot {
    pub company: Company,
    pub employees: Vec<EmployeeView>,
    pub people: Vec<PersonView>,
    /// The front desk, once you have opened it.
    pub desk: Option<EmployeeView>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    fn ada_draft() -> EmployeeDraft {
        EmployeeDraft {
            name: "Ada".into(),
            title: "Analyst".into(),
            description: String::new(),
            objective: String::new(),
            manager_id: None,
            skills: SkillGrant { builtins: false, keys: vec![] },
            clearance: Clearance::Supervised,
            allow_rules: vec![],
            model: None,
            effort: None,
            browser: false,
            auto_tidy: false,
            workspaces: vec![],
            hue: 0,
            image: None,
        }
    }

    #[test]
    fn a_display_image_is_only_ever_a_small_embedded_picture() {
        let with = |image: String| EmployeeDraft { image: Some(image), ..ada_draft() }.validate();
        assert!(with("data:image/webp;base64,UklGR".into()).is_ok());
        assert!(with("https://example.com/face.png".into()).is_err(), "never fetched from the network");
        assert!(with(format!("data:image/png;base64,{}", "A".repeat(300 * 1024))).is_err(), "too large");
    }

    #[test]
    fn a_face_needs_a_name_and_a_hue_under_360() {
        let bo = PersonDraft { name: "Bo".into(), title: String::new(), description: String::new(), strengths: String::new(), responsibilities: String::new(), channels: vec![], notes: String::new(), manager: PersonManager::You, hue: 0, image: None };
        let cases = [
            ("Ada", 359, None, None),
            (" ", 0, None, Some("name is required")),
            (" ", 360, None, Some("name is required")),
            ("Ada", 360, None, Some("hue must be 0-359")),
            ("Ada", 360, Some("https://example.com/face.png"), Some("hue must be 0-359")),
            ("Ada", 0, Some("https://example.com/face.png"), Some("the display image must be a picture under 256 KB")),
        ];
        for (name, hue, image, want) in cases {
            let image = image.map(String::from);
            let employee = EmployeeDraft { name: name.into(), hue, image: image.clone(), ..ada_draft() }.validate();
            let person = PersonDraft { name: name.into(), hue, image, ..bo.clone() }.validate();
            for got in [employee, person] {
                assert_eq!(got.err(), want.map(AppError::invalid), "{name:?} {hue}");
            }
        }
    }

    #[test]
    fn tidy_on_its_own_is_on_unless_turned_off() {
        let draft = ada_draft();
        let employee = draft.clone().into_employee("e1".into(), "ada".into(), "chat-ada".into(), Utc::now());
        let mut draft = serde_json::to_value(draft).unwrap();
        let mut employee = serde_json::to_value(employee).unwrap();
        assert!(!serde_json::from_value::<EmployeeDraft>(draft.clone()).unwrap().auto_tidy, "an explicit off stays off");
        assert!(!serde_json::from_value::<Employee>(employee.clone()).unwrap().auto_tidy, "an explicit off stays off");
        draft.as_object_mut().unwrap().remove("autoTidy");
        employee.as_object_mut().unwrap().remove("autoTidy");
        assert!(serde_json::from_value::<EmployeeDraft>(draft).unwrap().auto_tidy);
        assert!(serde_json::from_value::<Employee>(employee).unwrap().auto_tidy);
    }

    #[test]
    fn an_employee_saved_with_their_own_lanes_still_loads() {
        let mut employee = serde_json::to_value(Employee { id: "e1".into(), title: "Analyst".into(), auto_tidy: true, ..fixtures::employee() }).unwrap();
        employee.as_object_mut().unwrap().insert("lanes".into(), 2.into());
        assert_eq!(serde_json::from_value::<Employee>(employee).unwrap().name, "Ada", "older builds kept a lane count per employee");
    }
}
