//! Cards: what a run asks you, your answers, and news between colleagues.

use super::*;
use serde::{Deserialize, Serialize};

/// What a key an employee names is stored under, so it never meets a chore's.
pub(crate) const ASK: &str = "ask:";
/// What a chore's key is stored under, so every run waiting on the same chore shares it.
const CHORE: &str = "chore:";

fn chore_key(subject: &str) -> String {
    format!("{CHORE}{subject}")
}

/// One thing a run asks you with the answer built in: a choice, a list to approve, a chore or a task only you can
/// do, a question only you can answer, or news to read (`notify`). A run can ask several.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    /// The question, as the Inbox item's title.
    pub message: String,
    /// One line on why it matters, under the question.
    #[serde(default)]
    pub why: Option<String>,
    /// What it is about, so the same question from anyone is one Inbox item and asking again replaces it:
    /// `ask:<what the employee named it>`, `chore:<subject>`, or for a proposal OmegaStar raised `company:`,
    /// `owns:` or `company-page:` and what it is about. None on cards asked before keys.
    #[serde(default)]
    pub key: Option<String>,
    /// The file it is about, e.g. a draft to approve, which the card shows as it is now: a full path in the
    /// employee's folder or one of their workspaces.
    #[serde(default)]
    pub file: Option<String>,
    /// A web page it is about, e.g. the pull request: http or https only.
    #[serde(default)]
    pub link: Option<String>,
    /// Up to three images it shows, as full paths in the employee's folder or one of their workspaces.
    #[serde(default)]
    pub images: Vec<String>,
    pub ask: CardAsk,
    #[serde(default)]
    pub answer: Option<CardAnswer>,
    /// You said Later: the card opens again then, by itself.
    #[serde(default)]
    pub back_at: Option<Ts>,
    /// Times it came back after a Later.
    #[serde(default)]
    pub times_back: u32,
    /// You said Later again once it came back: parked in your Later list, answerable, it never comes back by itself.
    #[serde(default)]
    pub parked: bool,
    /// What OmegaStar does when you go ahead: the card is its proposal, not an employee's ask.
    #[serde(default)]
    pub proposal: Option<Proposal>,
    /// The facts needed to decide, each said once in a labelled row; the Inbox shows it in place of why and the
    /// run's report. Empty on cards without one.
    #[serde(default)]
    pub brief: Vec<BriefRow>,
}

/// A row of a card's brief: a short label, its text (simple markdown), and the attachment it shows under the text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BriefRow {
    pub label: String,
    pub text: String,
    #[serde(default)]
    pub shows: Option<Shows>,
}

/// Which of the card's attachments a brief row shows.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Shows {
    Images,
    File,
    Link,
}

impl Card {
    pub fn new(message: &str, why: Option<String>, key: Option<String>, ask: CardAsk) -> Card {
        let key = match &ask {
            CardAsk::Chore { subject, .. } => Some(chore_key(subject)),
            _ => key,
        };
        Card {
            message: message.to_string(),
            why,
            key,
            file: None,
            link: None,
            images: vec![],
            ask,
            answer: None,
            back_at: None,
            times_back: 0,
            parked: false,
            proposal: None,
            brief: vec![],
        }
    }

    /// Waiting on your answer. News waits on nothing: it is only unseen until you see it.
    pub fn is_open(&self) -> bool {
        self.answer.is_none() && self.ask != CardAsk::News
    }

    /// No answer yet: open, or news you have not seen.
    pub fn pending(&self) -> bool {
        self.answer.is_none()
    }

    /// You put it off with Later: it comes back by itself (`back_at`), or waits in your Later list, parked.
    pub fn put_off(&self) -> bool {
        self.back_at.is_some() || self.parked
    }

    /// Still waiting on you: no answer yet, or put off with Later. A newer card about the same thing replaces it,
    /// `close_ask` withdraws it, and any answer settles it.
    pub fn waiting(&self) -> bool {
        self.pending() || self.put_off()
    }

    /// What the card is about: its key, or for a chore asked before keys `chore:<subject>`.
    pub fn topic(&self) -> Option<String> {
        match (&self.key, &self.ask) {
            (Some(k), _) => Some(k.clone()),
            (None, CardAsk::Chore { subject, .. }) => Some(chore_key(subject)),
            _ => None,
        }
    }
}

/// What a card OmegaStar raised for an employee does when you go ahead.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Proposal {
    /// Adds `line` as a bullet under the company page's `## <section>` (`propose_company_line`).
    CompanyLine { section: String, line: String },
    /// Writes the employee's version of what they own, saved as the card's file, into their what-you-own.md.
    Owns,
    /// Writes the employee's version of the company page, saved as the card's file, into the page.
    CompanyPage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum CardAsk {
    /// Up to four buttons; `recommended` is the index of the one the employee recommends.
    Choices { choices: Vec<String>, recommended: usize },
    /// A list you tick one by one; `verb` is what a tick means, e.g. "Post".
    Items { verb: String, items: Vec<CardItem> },
    /// `chrome` or `sign-in:<site>`: every run waiting on the same subject carries on when you press Done. `url`: the
    /// page to do it on.
    Chore {
        subject: String,
        #[serde(default)]
        url: Option<String>,
    },
    /// A to-do only you can do: the steps, and the page to do it on. Done or Can't.
    Task {
        steps: String,
        #[serde(default)]
        url: Option<String>,
    },
    /// A fact only you know, answered in your own words.
    Question,
    /// Something you should know that needs no decision. Seen sends nothing.
    News,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardItem {
    pub id: String,
    /// What would go out, in full, e.g. the post.
    pub text: String,
    /// One line on what it is about, e.g. the review it answers, shown above the text.
    #[serde(default)]
    pub about: Option<String>,
    #[serde(default)]
    pub recommended: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CardAnswer {
    pub at: Ts,
    pub how: AnswerHow,
    /// The fixed answer sent to the employee, e.g. "Chose: Not now."; empty when nothing was sent.
    pub text: String,
    /// Not sent yet: its run still asks something else, and hears all your answers in one message once nothing is
    /// left open.
    #[serde(default)]
    pub held: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AnswerHow {
    /// You pressed one of its buttons.
    Tapped,
    /// You answered in your own words on the card. An older build counted any reply in the thread as one.
    Typed,
    /// You pressed Later (Not now): busy, not a no. The card comes back once by itself (`Card::back_at`), then is parked.
    /// On a chore it sends nothing and never comes back.
    Dismissed,
    /// You pressed No: drop it.
    Rejected,
    /// A newer card from the same routine, or about the same thing, took its place.
    Replaced,
    /// The employee closed it: it was settled without you.
    Withdrawn,
}

/// Your answer to a card, from the Inbox.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum CardReply {
    Choice { index: usize },
    Items { picks: Vec<ItemPick> },
    /// You did the chore or the task.
    Done,
    /// Not now: on a chore it sends nothing; on anything else it is Later until 9 am tomorrow.
    Dismiss,
    /// No, on a decision.
    Reject,
    /// An answer in your own words, to this card only.
    Text { text: String },
    /// A question about the card: it goes to the employee now, and the card stays open.
    Ask { text: String },
    /// Busy, not a no: the card comes back by itself at `until`, or is parked if it came back once already.
    Later { until: Ts },
    /// You cannot do the task.
    Cant,
    /// You read the news.
    Seen,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ItemPick {
    pub id: String,
    pub tick: bool,
    #[serde(default)]
    pub note: Option<String>,
}

/// News a colleague sent with `ask_colleague` kind info. It starts no run: the receiver reads it at the top of
/// their next run, and then it is read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct News {
    pub id: String,
    /// Who it is for.
    pub employee_id: String,
    /// The employee who sent it.
    pub from: String,
    pub text: String,
    pub created_at: Ts,
    #[serde(default)]
    pub read_at: Option<Ts>,
    /// The run whose opening note showed it, so a run that never started gives it back for the next one.
    #[serde(default)]
    pub read_in: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn a_card_reads_its_own_shapes() {
        let card = Card::new("Boost it?", None, None, CardAsk::Choices { choices: vec!["Not now".into(), "Try it".into()], recommended: 0 });
        let v = serde_json::to_value(&card).unwrap();
        assert_eq!(
            v,
            json!({
                "message": "Boost it?", "why": null, "key": null, "file": null, "link": null, "images": [],
                "ask": { "type": "choices", "choices": ["Not now", "Try it"], "recommended": 0 }, "answer": null,
                "backAt": null, "timesBack": 0, "parked": false, "proposal": null, "brief": []
            })
        );
        assert_eq!(serde_json::from_value::<Card>(v).unwrap(), card);
        let chrome = CardAsk::Chore { subject: "chrome".into(), url: None };
        assert_eq!(Card::new("Open Chrome.", None, Some("ignored".into()), chrome).key.as_deref(), Some("chore:chrome"));
        let reply: CardReply = serde_json::from_value(json!({ "type": "items", "picks": [{ "id": "d1", "tick": true }] })).unwrap();
        assert_eq!(reply, CardReply::Items { picks: vec![ItemPick { id: "d1".into(), tick: true, note: None }] });
        assert_eq!(serde_json::from_value::<CardReply>(json!({ "type": "done" })).unwrap(), CardReply::Done);
    }

    #[test]
    fn the_new_card_shapes_read_as_the_inbox_sends_them() {
        let ask = |v: Value| serde_json::from_value::<CardAsk>(v).unwrap();
        assert_eq!(ask(json!({ "type": "task", "steps": "Renew the domain.", "url": "https://example.com" })), CardAsk::Task { steps: "Renew the domain.".into(), url: Some("https://example.com".into()) });
        assert_eq!(ask(json!({ "type": "question" })), CardAsk::Question);
        assert_eq!(ask(json!({ "type": "news" })), CardAsk::News);
        assert_eq!(ask(json!({ "type": "chore", "subject": "chrome" })), CardAsk::Chore { subject: "chrome".into(), url: None }, "a chore saved before urls");
        assert_eq!(serde_json::to_value(CardAsk::Chore { subject: "sign-in:acme".into(), url: None }).unwrap(), json!({ "type": "chore", "subject": "sign-in:acme", "url": null }));
        assert_eq!(serde_json::to_value(CardAsk::Question).unwrap(), json!({ "type": "question" }));
        let line = Proposal::CompanyLine { section: "Who to ask".into(), line: "Bo signs off refunds.".into() };
        assert_eq!(serde_json::to_value(&line).unwrap(), json!({ "type": "companyLine", "section": "Who to ask", "line": "Bo signs off refunds." }));
        assert_eq!(serde_json::to_value(Proposal::Owns).unwrap(), json!({ "type": "owns" }));
        assert_eq!(serde_json::to_value(Proposal::CompanyPage).unwrap(), json!({ "type": "companyPage" }));
        let reply = |v: Value| serde_json::from_value::<CardReply>(v).unwrap();
        let until: Ts = "2026-11-02T09:00:00Z".parse().unwrap();
        assert_eq!(reply(json!({ "type": "later", "until": "2026-11-02T09:00:00Z" })), CardReply::Later { until });
        assert_eq!(reply(json!({ "type": "cant" })), CardReply::Cant);
        assert_eq!(reply(json!({ "type": "seen" })), CardReply::Seen);
        // A card an older build saved loads with none of what came later.
        let old: Card = serde_json::from_value(json!({ "message": "Boost it?", "ask": { "type": "choices", "choices": ["Go"], "recommended": 0 }, "answer": null })).unwrap();
        assert_eq!((old.link, old.images.len(), old.back_at, old.times_back, old.parked, old.proposal, old.brief.len()), (None, 0, None, 0, false, None, 0));
        let row: BriefRow = serde_json::from_value(json!({ "label": "The fix", "text": "Upgrade it.", "shows": "link" })).unwrap();
        assert_eq!(row, BriefRow { label: "The fix".into(), text: "Upgrade it.".into(), shows: Some(Shows::Link) });
    }

    #[test]
    fn news_is_unseen_until_answered_and_waits_on_nothing() {
        let mut run = Run::new("e1", RunKind::Manual, "Go".into(), Utc::now());
        run.cards = vec![Card::new("The spring sale is live.", None, Some("ask:sale".into()), CardAsk::News)];
        assert!(run.has_news() && !run.has_open_card(), "it holds back no answer");
        assert!(run.cards[0].pending() && !run.cards[0].is_open());
        run.cards[0].answer = Some(CardAnswer { at: Utc::now(), how: AnswerHow::Tapped, text: String::new(), held: false });
        assert!(!run.has_news());
    }
}
