//! The reads only the phone uses (`remote::api`): the Inbox rows, the team without its pictures, one picture at a
//! time, and a card's image. Plus hiring a proposal exactly as proposed, since the phone has no hire sheet.

use super::*;
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

    /// You, your employees and the people you added, without their pictures.
    pub fn members(&self) -> Result<Vec<TeamMember>> {
        let db = self.db();
        let company = db.company()?.unwrap_or_else(Company::unset);
        let you = TeamMember { id: YOU_ID.into(), name: company.ceo_name, title: company.title, hue: 0, has_image: company.image.is_some() };
        let employees = db.employees()?.into_iter().map(|e| TeamMember { id: e.id, name: e.name, title: e.title, hue: e.hue, has_image: e.image.is_some() });
        let people = db.people()?.into_iter().map(|p| TeamMember { id: p.id, name: p.name, title: p.title, hue: p.hue, has_image: p.image.is_some() });
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
        if fs::metadata(&path)?.len() > CARD_IMAGE_BYTES {
            return Err(AppError::invalid("That image is too large to send to the phone. Open it on the computer."));
        }
        image_data_url(Path::new(&path))
    }

    /// Hires from proposal `proposal_id` exactly as proposed, as the hire sheet does when you change nothing.
    pub fn approve_proposal_as_proposed(&self, proposal_id: &str, note: Option<String>) -> Result<Employee> {
        let p = self.db().proposal(proposal_id)?.ok_or_else(|| AppError::not_found("proposal not found"))?;
        self.approve_proposal(proposal_id, p.draft, vec![], note)
    }
}
