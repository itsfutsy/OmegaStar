//! Cards: what an employee asks you with the answer built in (`notify` with choices, items, a chore, a task, a
//! question or news), and the fixed text your answer sends back. A card is one thing, short enough to answer from the
//! Inbox without opening anything. Pure.

use crate::claude::{clip, one_line};
use crate::model::*;
use chrono::{Local, Utc};
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

/// The chore of reconnecting Claude in Chrome.
pub const CHROME: &str = "chrome";
const SIGN_IN: &str = "sign-in:";
/// A card's question, as the Inbox item's title.
const QUESTION_CHARS: usize = 80;
/// The one line under the question.
pub const WHY_CHARS: usize = 140;
const KEY_CHARS: usize = 60;
const MAX_CHOICES: usize = 4;
/// The buttons every decision card has already, so never one of its choices.
const BUILT_IN: [&str; 3] = ["later", "not now", "no"];
const CHOICE_CHARS: usize = 40;
const MAX_ITEMS: usize = 10;
const ITEM_ID_CHARS: usize = 20;
/// An item's text in full, never clipped: what you tick is what goes out.
const ITEM_CHARS: usize = 2000;
const ABOUT_CHARS: usize = 200;
const VERB_CHARS: usize = 20;
const NOTE_CHARS: usize = 300;
const DEFAULT_VERB: &str = "Approve";
/// A task's steps, in full.
const STEPS_CHARS: usize = 1000;
const LINK_CHARS: usize = 2000;
/// Kinds of file a card shows: markdown and plain text.
const FILE_KINDS: &[&str] = &["md", "markdown", "txt"];
/// Largest file a card shows.
pub const FILE_BYTES: u64 = 200 * 1024;
/// Most images a card shows, and the kinds it shows.
const MAX_IMAGES: usize = 3;
const IMAGE_KINDS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];
/// A company page heading a line is proposed for, and the line.
const SECTION_CHARS: usize = 60;
const LINE_CHARS: usize = 300;
/// A brief's rows, a row's label and its text.
const BRIEF_ROWS: std::ops::RangeInclusive<usize> = 2..=5;
const BRIEF_LABEL_CHARS: usize = 24;
const BRIEF_TEXT_CHARS: usize = 400;
/// The question a line for the company page is put to you with.
pub const ADD_LINE: &str = "Add to the company page?";

/// A notes file the app lays out to read beside a conversation: a .md or .txt file, in any letter case.
pub fn is_text_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("txt"))
}

/// The chore of signing in to `site`.
fn sign_in(site: &str) -> String {
    format!("{SIGN_IN}{site}")
}

/// A chore's subject as stored: lower case without spaces, exactly `chrome` or `sign-in:<site>`.
pub fn chore_subject(raw: &str) -> Result<String> {
    let s: String = raw.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase();
    let site_ok = |site: &str| (1..=40).contains(&site.len()) && site.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-');
    match s.strip_prefix(SIGN_IN) {
        _ if s == CHROME => Ok(s),
        Some(site) if site_ok(site) => Ok(sign_in(site)),
        _ => Err(AppError::invalid("chore must be 'chrome' or 'sign-in:<site>', for example 'sign-in:acme'.")),
    }
}

/// A key an employee names, as stored: lower case, spaces as dashes, under `ask:`, so `PR #12` is `ask:pr-#12`.
pub fn ask_key(raw: &str) -> Result<String> {
    let k: String = raw.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join("-");
    let k = k.strip_prefix(ASK).unwrap_or(&k).to_string();
    let ok = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || ":#._/-".contains(c);
    if !(2..=KEY_CHARS).contains(&k.chars().count()) || !k.chars().all(ok) {
        return Err(AppError::invalid(format!(
            "key names what the card is about in 2 to {KEY_CHARS} letters, digits and : # . _ / -, e.g. 'pr:12' or 'spring-sale'."
        )));
    }
    Ok(format!("{ASK}{k}"))
}

/// A key as the employee named it, without the prefix it is stored under.
pub fn key_label(key: &str) -> &str {
    key.strip_prefix(ASK).unwrap_or(key)
}

/// The card `notify`'s arguments ask for: exactly one of choices, items, chore, task, question and news, a short
/// question, an optional line on why, and for anything but a chore a key naming what it is about. Any card may carry a
/// link and up to three images, and any but a chore the markdown or text file it is about; files and images are as
/// given (the caller checks where they are). An empty list or text, or false, counts as left out.
pub fn card(args: &Value) -> Result<Card> {
    let list = |k: &str| args.get(k).and_then(Value::as_array).filter(|a| !a.is_empty());
    let text = |k: &str| args.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let flag = |k: &str| args.get(k).and_then(Value::as_bool) == Some(true);
    let message = text("message").ok_or_else(|| AppError::invalid("message is empty"))?;
    let n = message.chars().count();
    if n > QUESTION_CHARS {
        return Err(AppError::invalid(format!(
            "message is the question itself, at most {QUESTION_CHARS} characters (yours has {n}), e.g. 'Merge PR #12, the pricing page?'. \
Put the reason in why, at most {WHY_CHARS}."
        )));
    }
    if message.matches('?').count() > 1 {
        return Err(AppError::invalid("A card asks one thing. Raise one card for each decision."));
    }
    let why = text("why").map(one_line);
    if let Some(n) = why.as_ref().map(|w| w.chars().count()).filter(|n| *n > WHY_CHARS) {
        return Err(AppError::invalid(format!(
            "why is one line of at most {WHY_CHARS} characters (yours has {n}): what it changes or costs. The rest stays in your report."
        )));
    }
    let task = args.get("task").filter(|t| !t.is_null());
    let given = [list("choices").is_some(), list("items").is_some(), text("chore").is_some(), task.is_some(), flag("question"), flag("news")];
    let ask = match given.iter().filter(|g| **g).count() {
        0 => {
            return Err(AppError::invalid(
                "Give one of choices (e.g. Merge), items, chore, task, question or news. Routine updates and results go in your report.",
            ))
        }
        1 => match (list("choices"), list("items"), text("chore"), task) {
            (Some(c), ..) => choices_ask(c)?,
            (_, Some(i), ..) => items_ask(i, text("itemVerb"))?,
            (_, _, Some(c), _) => CardAsk::Chore { subject: chore_subject(c)?, url: None },
            (_, _, _, Some(t)) => task_ask(t)?,
            _ if flag("question") => CardAsk::Question,
            _ => CardAsk::News,
        },
        _ => return Err(AppError::invalid("Use one of choices, items, chore, task, question or news in a card, not several.")),
    };
    let ask = match (ask, text("url")) {
        (CardAsk::Chore { subject, .. }, Some(u)) => CardAsk::Chore { subject, url: Some(web_link(u)?) },
        (_, Some(_)) => return Err(AppError::invalid("url goes with chore: the page it is done on. A task takes its own in task; anything else takes link.")),
        (ask, None) => ask,
    };
    let chore = matches!(ask, CardAsk::Chore { .. });
    let key = match (chore, text("key")) {
        (true, _) => None,
        (false, Some(k)) => Some(ask_key(k)?),
        (false, None) => {
            return Err(AppError::invalid(
                "Add a key naming what this card is about, e.g. 'pr:12' or 'spring-sale', so asking about it again replaces this card.",
            ))
        }
    };
    let file = text("file");
    if file.is_some() && chore {
        return Err(AppError::invalid("file goes with any card but a chore: the draft or page it is about."));
    }
    if file.is_some_and(|f| !of_kind(f, FILE_KINDS)) {
        return Err(AppError::invalid("file must be a markdown or text file (.md or .txt), so it shows on the card."));
    }
    let images: Vec<String> = list("images").map(|a| a.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()).unwrap_or_default();
    if images.len() > MAX_IMAGES {
        return Err(AppError::invalid(format!("Give at most {MAX_IMAGES} images.")));
    }
    if images.iter().any(|i| !of_kind(i, IMAGE_KINDS)) {
        return Err(AppError::invalid("images must be png, jpg, gif or webp files, so they show on the card."));
    }
    let mut card = Card::new(message, why, key, ask);
    card.file = file.map(String::from);
    card.link = text("link").map(web_link).transpose()?;
    card.images = images;
    card.brief = match args.get("brief") {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(rows)) if rows.is_empty() => vec![],
        Some(Value::Array(rows)) => brief(rows, &card)?,
        Some(_) => return Err(AppError::invalid("brief is a list of rows, e.g. [{\"label\": \"The fix\", \"text\": \"...\"}], not text.")),
    };
    Ok(card)
}

/// A card's brief: 2 to 5 rows, each a one-line label and its text, and optionally which of the card's attachments it
/// shows, each attachment shown by one row at most.
fn brief(raw: &[Value], card: &Card) -> Result<Vec<BriefRow>> {
    if !BRIEF_ROWS.contains(&raw.len()) {
        return Err(AppError::invalid(format!(
            "brief is {} to {} labelled rows, e.g. [{{\"label\": \"The problem\", \"text\": \"...\"}}, {{\"label\": \"The fix\", \"text\": \"...\", \"shows\": \"link\"}}]. Leave it out on a card that needs none.",
            BRIEF_ROWS.start(),
            BRIEF_ROWS.end()
        )));
    }
    let mut rows: Vec<BriefRow> = vec![];
    for r in raw {
        if !r.is_object() || r.get("shows").is_some_and(|v| !v.is_string() && !v.is_null()) {
            return Err(AppError::invalid("Each brief row is {\"label\": \"...\", \"text\": \"...\"}, with shows, if given, one of images, file or link."));
        }
        let field = |k: &str| r.get(k).and_then(Value::as_str).map(str::trim).unwrap_or("");
        let label = one_line(field("label"));
        if label.is_empty() || label.chars().count() > BRIEF_LABEL_CHARS {
            return Err(AppError::invalid(format!("Each brief row needs a label of 1 to {BRIEF_LABEL_CHARS} characters on one line, e.g. 'The fix'.")));
        }
        let text = field("text");
        let n = text.chars().count();
        if n == 0 || n > BRIEF_TEXT_CHARS {
            return Err(AppError::invalid(format!(
                "Brief row '{label}': text is 1 to {BRIEF_TEXT_CHARS} characters (it has {n}). Say each fact once; a longer draft goes in file."
            )));
        }
        let shows = match field("shows") {
            "" => None,
            "images" if !card.images.is_empty() => Some(Shows::Images),
            "file" if card.file.is_some() => Some(Shows::File),
            "link" if card.link.is_some() => Some(Shows::Link),
            s @ ("images" | "file" | "link") => return Err(AppError::invalid(format!("Brief row '{label}' shows the card's {s}, but the card has none. Add it, or leave shows out."))),
            _ => return Err(AppError::invalid(format!("Brief row '{label}': shows is one of images, file or link."))),
        };
        if let Some(other) = rows.iter().find(|x| shows.is_some() && x.shows == shows) {
            return Err(AppError::invalid(format!("One brief row at most shows each attachment: '{}' and '{label}' both show it.", other.label)));
        }
        rows.push(BriefRow { label, text: text.to_string(), shows });
    }
    Ok(rows)
}

/// Whether `path`'s extension is one of `kinds`, whatever its case.
fn of_kind(path: &str, kinds: &[&str]) -> bool {
    Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|e| kinds.contains(&e.to_ascii_lowercase().as_str()))
}

/// A web address as a card keeps it: http or https, with a host and no spaces.
fn web_link(raw: &str) -> Result<String> {
    let lower = raw.to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://")).unwrap_or("");
    if rest.is_empty() || rest.starts_with('/') || raw.chars().any(|c| c.is_whitespace() || c.is_control()) || raw.chars().count() > LINK_CHARS {
        return Err(AppError::invalid(format!(
            "A link is a web address starting with https:// or http://, at most {LINK_CHARS} characters, e.g. 'https://example.com/pull/12'."
        )));
    }
    Ok(raw.to_string())
}

fn task_ask(raw: &Value) -> Result<CardAsk> {
    let field = |k: &str| raw.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let steps = field("steps").ok_or_else(|| AppError::invalid("task is {steps, url}: the steps, in order, and optionally the page to do them on."))?;
    let n = steps.chars().count();
    if n > STEPS_CHARS {
        return Err(AppError::invalid(format!("A task's steps are at most {STEPS_CHARS} characters (yours have {n}). Put the rest in a file.")));
    }
    Ok(CardAsk::Task { steps: steps.to_string(), url: field("url").map(web_link).transpose()? })
}

fn choices_ask(raw: &[Value]) -> Result<CardAsk> {
    if !(1..=MAX_CHOICES).contains(&raw.len()) {
        return Err(AppError::invalid(format!("Give 1 to {MAX_CHOICES} choices.")));
    }
    let mut choices: Vec<String> = vec![];
    let mut recommended = vec![];
    for (i, c) in raw.iter().enumerate() {
        let label = c.get("label").and_then(Value::as_str).map(str::trim).unwrap_or("");
        if label.is_empty() || label.chars().count() > CHOICE_CHARS {
            return Err(AppError::invalid(format!("Each choice needs a label of 1 to {CHOICE_CHARS} characters.")));
        }
        if BUILT_IN.contains(&label.trim_end_matches(['.', '!']).to_lowercase().as_str()) {
            return Err(AppError::invalid("Every card has Later and No built in: give only the ways to go ahead."));
        }
        if choices.iter().any(|x| x.to_lowercase() == label.to_lowercase()) {
            return Err(AppError::invalid(format!("Each choice must differ: '{label}' is there twice.")));
        }
        if c.get("recommended").and_then(Value::as_bool) == Some(true) {
            recommended.push(i);
        }
        choices.push(label.to_string());
    }
    match recommended[..] {
        [i] => Ok(CardAsk::Choices { choices, recommended: i }),
        [] if choices.len() == 1 => Ok(CardAsk::Choices { choices, recommended: 0 }),
        _ => Err(AppError::invalid("Mark exactly one choice with recommended: true: the one you recommend.")),
    }
}

fn items_ask(raw: &[Value], verb: Option<&str>) -> Result<CardAsk> {
    if raw.len() > MAX_ITEMS {
        return Err(AppError::invalid(format!("Give at most {MAX_ITEMS} items.")));
    }
    let verb = verb.unwrap_or(DEFAULT_VERB);
    if verb.chars().count() > VERB_CHARS {
        return Err(AppError::invalid(format!("itemVerb is one word of at most {VERB_CHARS} characters, e.g. 'Post'.")));
    }
    let mut ids = HashSet::new();
    let mut items = vec![];
    for i in raw {
        let field = |k: &str| i.get(k).and_then(Value::as_str).map(str::trim).unwrap_or("");
        let (id, text) = (field("id"), field("text"));
        if id.is_empty() || id.chars().count() > ITEM_ID_CHARS {
            return Err(AppError::invalid(format!("Each item needs an id of 1 to {ITEM_ID_CHARS} characters.")));
        }
        if !ids.insert(id.to_string()) {
            return Err(AppError::invalid(format!("Each item needs its own id: '{id}' is there twice.")));
        }
        if text.is_empty() {
            return Err(AppError::invalid(format!("Item '{id}' needs its text.")));
        }
        let n = text.chars().count();
        if n > ITEM_CHARS {
            return Err(AppError::invalid(format!(
                "Item '{id}': text is at most {ITEM_CHARS} characters (it has {n}). Put a longer draft in a file and ask about it on its own card."
            )));
        }
        let about = Some(one_line(field("about"))).filter(|a| !a.is_empty());
        if let Some(n) = about.as_ref().map(|a| a.chars().count()).filter(|n| *n > ABOUT_CHARS) {
            return Err(AppError::invalid(format!("Item '{id}': about is one line of at most {ABOUT_CHARS} characters (it has {n}).")));
        }
        let recommended = i.get("recommended").and_then(Value::as_bool).unwrap_or(false);
        items.push(CardItem { id: id.to_string(), text: text.to_string(), about, recommended });
    }
    Ok(CardAsk::Items { verb: verb.to_string(), items })
}

/// The items of a list card raised while the same employee's list about the same thing is open: `lists`, oldest first
/// and the new card's last, as one list in the order each item first came, an item with an id already there taking the
/// newer text. Err past `MAX_ITEMS`.
pub fn grow(lists: impl IntoIterator<Item = Vec<CardItem>>) -> Result<Vec<CardItem>> {
    let mut items: Vec<CardItem> = vec![];
    for i in lists.into_iter().flatten() {
        match items.iter_mut().find(|x| x.id == i.id) {
            Some(x) => *x = i,
            None => items.push(i),
        }
    }
    if items.len() > MAX_ITEMS {
        return Err(AppError::invalid(format!(
            "Your open list about this already has items, and with these it would hold {}, past the {MAX_ITEMS} a card holds. Ask about the rest once it is answered.",
            items.len()
        )));
    }
    Ok(items)
}

/// A choices, items or task card with neither file nor link whose message or why names something written or online (a
/// draft, a pull request, a post, an email, a page or a reply): it should carry what it is about.
pub fn wants_attachment(card: &Card) -> bool {
    const NAMES: [&str; 12] = ["draft", "drafts", "pr", "prs", "post", "posts", "email", "emails", "page", "pages", "reply", "replies"];
    if card.file.is_some() || card.link.is_some() || !matches!(card.ask, CardAsk::Choices { .. } | CardAsk::Items { .. } | CardAsk::Task { .. }) {
        return false;
    }
    let text = format!("{} {}", card.message, card.why.as_deref().unwrap_or_default()).to_lowercase();
    text.contains("pull request") || text.split(|c: char| !c.is_alphanumeric()).any(|w| NAMES.contains(&w))
}

/// The card `propose_company_line` raises: `line` as a bullet under the company page's `section`, with `why` under the
/// question. Keyed by the section and the line, so the same line proposed again, by anyone, is the same card.
pub fn company_line_card(section: &str, line: &str, why: &str) -> Result<Card> {
    let section = one_line(section.trim().trim_start_matches('#'));
    let line = line.trim();
    let line = one_line(line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")).unwrap_or(line));
    let why = one_line(why);
    if section.is_empty() || section.chars().count() > SECTION_CHARS {
        return Err(AppError::invalid(format!("section is the page's heading the line goes under, e.g. 'Who to ask', at most {SECTION_CHARS} characters.")));
    }
    if line.is_empty() || line.chars().count() > LINE_CHARS {
        return Err(AppError::invalid(format!("line is one fact for everyone, at most {LINE_CHARS} characters.")));
    }
    if why.is_empty() || why.chars().count() > WHY_CHARS {
        return Err(AppError::invalid(format!("why is one line of at most {WHY_CHARS} characters: why everyone should know it.")));
    }
    let slug = section.to_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join("-");
    let key = format!("company:{slug}:{}", short_hash(&line));
    let mut card = Card::new(ADD_LINE, Some(why), Some(key), CardAsk::Choices { choices: vec!["Add it".into()], recommended: 0 });
    card.proposal = Some(Proposal::CompanyLine { section, line });
    Ok(card)
}

/// Eight hex digits of FNV-1a over `s`: short, and the same in every build.
fn short_hash(s: &str) -> String {
    format!("{:08x}", s.bytes().fold(0x811c_9dc5_u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193)))
}

/// What No on a decision tells the employee.
pub const REJECTED: &str = "No. Do not do it, and do not ask again.";

/// What Later tells the employee: busy, not a no, and the day the card comes back to you by itself.
pub fn later_text(ceo: &str, until: Ts) -> String {
    let day = until.with_timezone(&Local).format("%A, %-d %B");
    format!("Later: {ceo} is busy, not a no. The card comes back to them on {day} by itself. Leave it as it is.")
}

/// What Later tells the employee about a card that came back once already: it is parked.
pub fn parked_text(ceo: &str) -> String {
    format!("Later again: {ceo} parked it in their Parked list. It no longer comes back by itself.")
}

/// 9 am on the day after `at`, on your local calendar: when Not now on a decision brings it back.
pub fn tomorrow_morning(at: Ts) -> Ts {
    let day = at.with_timezone(&Local).date_naive() + chrono::Days::new(1);
    day.and_hms_opt(9, 0, 0).and_then(|t| t.and_local_timezone(Local).earliest()).map_or(at + chrono::Duration::days(1), |t| t.with_timezone(&Utc))
}

/// Puts your `reply` on `card` at `at`, as the employee is to hear it (`answer_text`). Later, and Not now on anything but
/// a chore (Later until 9 am tomorrow), sets when the card comes back, or parks it once it came back already. Any answer
/// on a card put off with Later (`Card::put_off`), waiting to come back or parked, takes the place of its Later. A
/// proposal's own answers say what OmegaStar did with it. When the answer does not fit, the card is left as it was.
pub fn answer(card: &mut Card, reply: &CardReply, ceo: &str, at: Ts) -> Result<()> {
    if !card.waiting() {
        return Err(AppError::conflict("That card was already answered."));
    }
    let later = match reply {
        CardReply::Later { until } => Some(*until),
        CardReply::Dismiss if !matches!(card.ask, CardAsk::Chore { .. }) => Some(tomorrow_morning(at)),
        _ => None,
    };
    let waits = matches!(card.ask, CardAsk::Choices { .. } | CardAsk::Items { .. } | CardAsk::Task { .. } | CardAsk::Question);
    let (text, back_at, parked) = match later {
        Some(_) if !waits => return Err(AppError::invalid("That answer does not fit this card.")),
        Some(_) if card.parked => return Err(AppError::conflict("That card is parked already: answer it, or leave it in Parked.")),
        Some(until) if until <= at => return Err(AppError::invalid("Pick a time after now.")),
        Some(_) if card.times_back > 0 => (parked_text(ceo), None, true),
        Some(until) => (later_text(ceo, until), Some(until), false),
        None => {
            let fixed = answer_text(&card.ask, reply, ceo)?;
            let text = match (&card.proposal, reply) {
                (Some(p), CardReply::Choice { .. }) => Some(proposal_text(p, true)),
                (Some(p), CardReply::Reject) => Some(proposal_text(p, false)),
                _ => fixed,
            };
            (text.unwrap_or_default(), None, false)
        }
    };
    let how = match reply {
        CardReply::Dismiss | CardReply::Later { .. } => AnswerHow::Dismissed,
        CardReply::Reject => AnswerHow::Rejected,
        CardReply::Text { .. } => AnswerHow::Typed,
        _ => AnswerHow::Tapped,
    };
    card.answer = Some(CardAnswer { at, how, text, held: false });
    card.back_at = back_at;
    card.parked = parked;
    Ok(())
}

/// What going ahead with a proposal, or No, tells the employee.
fn proposal_text(p: &Proposal, yes: bool) -> String {
    match (p, yes) {
        (Proposal::CompanyLine { section, .. }, true) => format!("Added to the company page under {section}."),
        (Proposal::CompanyLine { .. }, false) => "Not added.".into(),
        (Proposal::Owns, true) => "Used: your update to what you own is in what-you-own.md now.".into(),
        (Proposal::CompanyPage, true) => "Used: your change to the company page is on it now.".into(),
        (_, false) => "Not used.".into(),
    }
}

/// Your reply on a card that asks `ask`, as it reaches another card about the same thing that asks `other`, or None
/// when it does not fit there: a choice reaches only a card offering the same label, whatever its case.
pub fn same_reply(ask: &CardAsk, reply: &CardReply, other: &CardAsk) -> Option<CardReply> {
    match (ask, reply, other) {
        (CardAsk::Choices { choices, .. }, CardReply::Choice { index }, CardAsk::Choices { choices: theirs, .. }) => {
            let label = choices.get(*index)?.to_lowercase();
            theirs.iter().position(|c| c.to_lowercase() == label).map(|index| CardReply::Choice { index })
        }
        (_, CardReply::Choice { .. }, _) => None,
        _ => Some(reply.clone()),
    }
}

/// The fixed message your answer sends to the employee, or None when it sends nothing (a dismissed chore, news you
/// saw). An answer in your own words is your words as you typed them, on any card: on news, a follow-up that also
/// counts as seeing it. Later is `answer`'s.
fn answer_text(ask: &CardAsk, reply: &CardReply, ceo: &str) -> Result<Option<String>> {
    let unfit = || AppError::invalid("That answer does not fit this card.");
    let text = match (ask, reply) {
        (CardAsk::News, CardReply::Seen) => return Ok(None),
        (_, CardReply::Text { text }) => Some(text.trim()).filter(|t| !t.is_empty()).ok_or_else(|| AppError::invalid("Type your answer first."))?.to_string(),
        (CardAsk::News, _) | (_, CardReply::Seen) => return Err(unfit()),
        (CardAsk::Choices { choices, .. }, CardReply::Choice { index }) => format!("Chose: {}.", choices.get(*index).ok_or_else(unfit)?),
        (CardAsk::Items { verb, items }, CardReply::Items { picks }) => items_text(verb, items, picks, ceo)?,
        (CardAsk::Choices { .. } | CardAsk::Items { .. }, CardReply::Reject) => REJECTED.into(),
        (CardAsk::Chore { subject, .. }, CardReply::Done) => match subject.strip_prefix(SIGN_IN) {
            Some(site) => format!("{ceo} signed in to {site}. Carry on with what was waiting on it."),
            // Done is what they say, not a check: Chrome can still be down.
            None => format!(
                "{ceo} pressed Done on reconnecting Chrome. Check with list_connected_browsers first. If Chrome is connected, carry on \
with what was waiting on it. If it is still not connected, say so in one line and stop. Do not raise another Chrome card in this run."
            ),
        },
        (CardAsk::Chore { .. }, CardReply::Dismiss) => return Ok(None),
        (CardAsk::Task { .. }, CardReply::Done) => format!("{ceo} did it."),
        (CardAsk::Task { .. }, CardReply::Cant) => format!("{ceo} can't do this. Find another way, or say exactly what you need from them."),
        _ => return Err(unfit()),
    };
    Ok(Some(text))
}

/// `Post d1, d3. Skip d2 (Pat: 'too late'). No answer on d4.`, in the card's order.
fn items_text(verb: &str, items: &[CardItem], picks: &[ItemPick], ceo: &str) -> Result<String> {
    if let Some(p) = picks.iter().find(|p| !items.iter().any(|i| i.id == p.id)) {
        return Err(AppError::invalid(format!("'{}' is not on this card.", p.id)));
    }
    let (mut plain, mut noted, mut untouched) = (vec![], vec![], vec![]);
    for i in items {
        let pick = picks.iter().find(|p| p.id == i.id);
        let tick = pick.is_some_and(|p| p.tick);
        let note = pick.and_then(|p| p.note.as_deref()).map(one_line).filter(|n| !n.is_empty()).map(|n| clip(&n, NOTE_CHARS));
        match (tick, note) {
            (true, None) => plain.push(i.id.as_str()),
            (true, Some(n)) => noted.push(format!("{verb} {} ({ceo}: '{n}').", i.id)),
            (false, Some(n)) => noted.push(format!("Skip {} ({ceo}: '{n}').", i.id)),
            (false, None) => untouched.push(i.id.as_str()),
        }
    }
    if plain.is_empty() && noted.is_empty() {
        return Err(AppError::invalid("Tick or note at least one item."));
    }
    let mut parts = vec![];
    if !plain.is_empty() {
        parts.push(format!("{verb} {}.", plain.join(", ")));
    }
    parts.extend(noted);
    if !untouched.is_empty() {
        parts.push(format!("No answer on {}.", untouched.join(", ")));
    }
    Ok(parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `args` with a question and a key, as most cards need.
    fn asked(mut args: Value) -> Value {
        let o = args.as_object_mut().unwrap();
        o.entry("message").or_insert(json!("Boost it?"));
        o.entry("key").or_insert(json!("boost"));
        args
    }

    fn ask_of(args: Value) -> CardAsk {
        card(&asked(args)).unwrap().ask
    }

    fn err(args: Value) -> String {
        card(&asked(args)).unwrap_err().message
    }

    #[test]
    fn choices_need_one_to_four_distinct_labels_and_one_recommendation() {
        let ok = ask_of(json!({ "choices": [{ "label": " Try it for a day " }, { "label": "Try it for a week", "recommended": true }] }));
        assert_eq!(ok, CardAsk::Choices { choices: vec!["Try it for a day".into(), "Try it for a week".into()], recommended: 1 });
        assert_eq!(ask_of(json!({ "choices": [{ "label": "Merge" }] })), CardAsk::Choices { choices: vec!["Merge".into()], recommended: 0 }, "one choice is the one recommended");
        let five: Vec<Value> = (0..5).map(|i| json!({ "label": format!("Option {i}"), "recommended": i == 0 })).collect();
        assert_eq!(err(json!({ "choices": five })), "Give 1 to 4 choices.");
        assert_eq!(err(json!({ "choices": [{ "label": "Yes" }, { "label": "Soon" }] })), "Mark exactly one choice with recommended: true: the one you recommend.");
        assert_eq!(err(json!({ "choices": [{ "label": "Yes", "recommended": true }, { "label": "Soon", "recommended": true }] })), "Mark exactly one choice with recommended: true: the one you recommend.");
        for built_in in ["Not now", " not now. ", "Later", "No", "NO!"] {
            let e = err(json!({ "choices": [{ "label": "Merge", "recommended": true }, { "label": built_in }] }));
            assert_eq!(e, "Every card has Later and No built in: give only the ways to go ahead.", "{built_in}");
        }
        assert!(err(json!({ "choices": [{ "label": "Yes", "recommended": true }, { "label": "yes" }] })).contains("'yes' is there twice"));
        assert!(err(json!({ "choices": [{ "label": "Yes", "recommended": true }, { "label": " " }] })).starts_with("Each choice needs a label"));
        assert!(err(json!({ "choices": [{ "label": "Yes", "recommended": true }, { "label": "x".repeat(41) }] })).starts_with("Each choice needs a label"));
    }

    #[test]
    fn items_need_short_unique_ids_and_text() {
        let long = "y".repeat(2000);
        let about = " 1 star, 3 May:\n\"Too slow.\" ";
        let CardAsk::Items { verb, items } = ask_of(json!({ "items": [{ "id": " d1 ", "text": "Reply to the launch post", "about": about, "recommended": true }, { "id": "d2", "text": long, "about": " " }], "itemVerb": " Post " })) else { panic!("items") };
        assert_eq!(verb, "Post");
        assert_eq!(items[0], CardItem { id: "d1".into(), text: "Reply to the launch post".into(), about: Some("1 star, 3 May: \"Too slow.\"".into()), recommended: true });
        assert!(!items[1].recommended && items[1].text.chars().count() == 2000, "never clipped");
        assert_eq!(items[1].about, None, "a blank about counts as left out");
        assert!(err(json!({ "items": [{ "id": "d1", "text": "y".repeat(2001) }] })).starts_with("Item 'd1': text is at most 2000 characters (it has 2001)"));
        assert!(err(json!({ "items": [{ "id": "d1", "text": "a", "about": "y".repeat(201) }] })).starts_with("Item 'd1': about is one line of at most 200 characters (it has 201)"));
        let CardAsk::Items { verb, .. } = ask_of(json!({ "items": [{ "id": "a", "text": "One" }] })) else { panic!("items") };
        assert_eq!(verb, "Approve");
        let eleven: Vec<Value> = (0..11).map(|i| json!({ "id": format!("d{i}"), "text": "x" })).collect();
        assert_eq!(err(json!({ "items": eleven })), "Give at most 10 items.");
        assert!(err(json!({ "items": [{ "id": "d1", "text": "a" }, { "id": "d1", "text": "b" }] })).contains("'d1' is there twice"));
        assert!(err(json!({ "items": [{ "id": "x".repeat(21), "text": "a" }] })).starts_with("Each item needs an id"));
        assert_eq!(err(json!({ "items": [{ "id": "d1", "text": " " }] })), "Item 'd1' needs its text.");
        assert!(err(json!({ "items": [{ "id": "d1", "text": "a" }], "itemVerb": "x".repeat(21) })).starts_with("itemVerb"));
    }

    #[test]
    fn a_card_asks_one_short_thing() {
        let yes_no = json!([{ "label": "Merge", "recommended": true }]);
        assert!(err(json!({ "message": "FYI: signups are up." })).starts_with("Give one of choices (e.g. Merge), items, chore, task, question or news."), "an update is not a card");
        assert!(err(json!({ "choices": [], "chore": "", "question": false })).starts_with("Give one of choices"));
        let several = "Use one of choices, items, chore, task, question or news in a card, not several.";
        assert_eq!(err(json!({ "choices": [{ "label": "A", "recommended": true }, { "label": "B" }], "chore": "chrome" })), several);
        assert_eq!(err(json!({ "question": true, "news": true })), several);
        assert!(err(json!({ "message": "x".repeat(81), "choices": yes_no })).contains("at most 80 characters (yours has 81)"));
        assert_eq!(err(json!({ "message": "Merge #12? And #13?", "choices": yes_no })), "A card asks one thing. Raise one card for each decision.");
        assert!(err(json!({ "why": "y".repeat(141), "choices": yes_no })).starts_with("why is one line of at most 140 characters (yours has 141)"));
        let c = card(&json!({ "message": " Merge PR #12? ", "why": "Busiest page.\nReviewed.", "key": "PR #12", "choices": yes_no })).unwrap();
        assert_eq!((c.message.as_str(), c.why.as_deref(), c.key.as_deref()), ("Merge PR #12?", Some("Busiest page. Reviewed."), Some("ask:pr-#12")));
        assert!(card(&json!({ "message": "Merge PR #12?", "choices": yes_no })).unwrap_err().message.starts_with("Add a key"));
        assert!(card(&json!({ "message": "The sale is live.", "news": true })).unwrap_err().message.starts_with("Add a key"), "news names what it is about too");
        let chore = card(&json!({ "message": "Reconnect Chrome.", "chore": "chrome" })).unwrap();
        assert_eq!(chore.key.as_deref(), Some("chore:chrome"), "a chore needs no key of its own");
    }

    #[test]
    fn a_card_is_a_task_a_question_or_news_too() {
        assert_eq!(
            ask_of(json!({ "task": { "steps": " Open the bank's site.\nApprove the new payee. ", "url": "https://bank.example.com/payees" } })),
            CardAsk::Task { steps: "Open the bank's site.\nApprove the new payee.".into(), url: Some("https://bank.example.com/payees".into()) }
        );
        assert_eq!(ask_of(json!({ "task": { "steps": "Sign the form." } })), CardAsk::Task { steps: "Sign the form.".into(), url: None });
        assert!(err(json!({ "task": { "url": "https://example.com" } })).starts_with("task is {steps, url}"));
        assert!(err(json!({ "task": "Sign the form." })).starts_with("task is {steps, url}"));
        assert!(err(json!({ "task": { "steps": "y".repeat(1001) } })).starts_with("A task's steps are at most 1000 characters (yours have 1001)"));
        assert!(err(json!({ "task": { "steps": "Sign it.", "url": "ftp://example.com" } })).starts_with("A link is a web address"));
        assert_eq!(ask_of(json!({ "question": true })), CardAsk::Question);
        assert_eq!(ask_of(json!({ "news": true, "choices": [] })), CardAsk::News);
    }

    #[test]
    fn a_chore_is_chrome_or_a_sign_in_on_its_page() {
        assert_eq!(ask_of(json!({ "chore": "Sign-In: Acme" })), CardAsk::Chore { subject: "sign-in:acme".into(), url: None });
        assert_eq!(
            ask_of(json!({ "chore": "sign-in:acme", "url": " https://acme.example.com/login " })),
            CardAsk::Chore { subject: "sign-in:acme".into(), url: Some("https://acme.example.com/login".into()) }
        );
        assert!(err(json!({ "choices": [{ "label": "Go" }], "url": "https://example.com" })).starts_with("url goes with chore"));
        assert_eq!(chore_subject(" Chrome ").unwrap(), CHROME);
        assert_eq!(chore_subject("sign-in:app.example-bank.com").unwrap(), sign_in("app.example-bank.com"));
        for bad in ["gmail", "account", "sign-in:", "sign-in:acme/login", &format!("sign-in:{}", "a".repeat(41))] {
            assert_eq!(chore_subject(bad).unwrap_err().message, "chore must be 'chrome' or 'sign-in:<site>', for example 'sign-in:acme'.", "{bad}");
        }
    }

    #[test]
    fn a_card_can_show_the_text_file_it_is_about() {
        let yes_no = json!([{ "label": "Send", "recommended": true }]);
        let c = card(&asked(json!({ "choices": yes_no, "file": " workspace/drafts/Reply.MD " }))).unwrap();
        assert_eq!(c.file.as_deref(), Some("workspace/drafts/Reply.MD"));
        assert_eq!(card(&asked(json!({ "choices": yes_no, "file": "" }))).unwrap().file, None);
        assert!(card(&asked(json!({ "items": [{ "id": "d1", "text": "One" }], "file": "/tmp/notes.txt" }))).is_ok());
        assert!(card(&asked(json!({ "task": { "steps": "Sign it." }, "file": "workspace/form.md" }))).is_ok());
        for bad in ["workspace/drafts/reply.pdf", "workspace/drafts", "notes.md.sh"] {
            assert!(err(json!({ "choices": yes_no, "file": bad })).starts_with("file must be a markdown or text file"), "{bad}");
        }
        assert!(card(&json!({ "message": "Reconnect Chrome.", "chore": "chrome", "file": "a.md" })).unwrap_err().message.starts_with("file goes with any card but a chore"));
    }

    #[test]
    fn a_card_can_carry_a_link_and_up_to_three_images() {
        let go = json!([{ "label": "Merge", "recommended": true }]);
        let c = card(&asked(json!({ "choices": go, "link": " https://example.com/acme/site/pull/12 ", "images": [" shots/before.png", "shots/after.JPG", ""] }))).unwrap();
        assert_eq!(c.link.as_deref(), Some("https://example.com/acme/site/pull/12"));
        assert_eq!(c.images, vec!["shots/before.png".to_string(), "shots/after.JPG".to_string()]);
        assert!(card(&asked(json!({ "news": true, "link": "http://example.com" }))).is_ok());
        for bad in ["example.com/pull/12", "javascript:alert(1)", "file:///etc/hosts", "https://", "https:///path", "https://example.com/a b"] {
            assert!(err(json!({ "choices": go, "link": bad })).starts_with("A link is a web address"), "{bad}");
        }
        assert_eq!(err(json!({ "choices": go, "images": ["a.png", "b.png", "c.png", "d.png"] })), "Give at most 3 images.");
        assert!(err(json!({ "choices": go, "images": ["chart.svg"] })).starts_with("images must be png, jpg, gif or webp files"));
    }

    #[test]
    fn a_brief_is_a_few_labelled_rows_placing_the_attachments() {
        let go = json!([{ "label": "Merge", "recommended": true }]);
        let rows = json!([
            { "label": " The problem ", "text": " Grouped cards leave out the line.\n\n- One\n- Two " },
            { "label": "The fix", "text": "Show it.", "shows": "images" },
            { "label": "If you\nmerge", "text": "Only the Inbox changes.", "shows": "link" },
        ]);
        let c = card(&asked(json!({ "choices": go, "brief": rows, "images": ["a.png"], "link": "https://example.com/pull/12" }))).unwrap();
        assert_eq!(c.brief[0], BriefRow { label: "The problem".into(), text: "Grouped cards leave out the line.\n\n- One\n- Two".into(), shows: None });
        assert_eq!((c.brief[1].shows, c.brief[2].label.as_str(), c.brief[2].shows), (Some(Shows::Images), "If you merge", Some(Shows::Link)));
        assert!(card(&asked(json!({ "choices": go, "brief": [] }))).unwrap().brief.is_empty(), "an empty brief counts as left out");
        let row = |label: &str, text: &str| json!({ "label": label, "text": text });
        let brief_err = |brief: Value| err(json!({ "choices": go, "brief": brief, "file": "draft.md" }));
        assert!(brief_err(json!([row("Who", "Tim")])).starts_with("brief is 2 to 5 labelled rows"));
        assert!(brief_err(json!((0..6).map(|i| row(&format!("Row {i}"), "x")).collect::<Vec<_>>())).starts_with("brief is 2 to 5 labelled rows"));
        assert!(brief_err(json!([row(" ", "x"), row("B", "y")])).starts_with("Each brief row needs a label of 1 to 24 characters"));
        assert!(brief_err(json!([row(&"l".repeat(25), "x"), row("B", "y")])).starts_with("Each brief row needs a label"));
        assert_eq!(brief_err(json!([row("Who", " "), row("B", "y")])), "Brief row 'Who': text is 1 to 400 characters (it has 0). Say each fact once; a longer draft goes in file.");
        assert!(brief_err(json!([row("Who", &"y".repeat(401)), row("B", "y")])).starts_with("Brief row 'Who': text is 1 to 400 characters (it has 401)"));
        assert!(card(&asked(json!({ "choices": go, "brief": [row("Who", &"y".repeat(400)), row("B", "y")] }))).is_ok());
        let shows = |s: &str| json!([row("Who", "Tim"), { "label": "Draft", "text": "Here.", "shows": s }]);
        assert_eq!(brief_err(shows("images")), "Brief row 'Draft' shows the card's images, but the card has none. Add it, or leave shows out.");
        assert!(brief_err(shows("link")).starts_with("Brief row 'Draft' shows the card's link, but the card has none"));
        assert_eq!(brief_err(shows("picture")), "Brief row 'Draft': shows is one of images, file or link.");
        assert_eq!(card(&asked(json!({ "choices": go, "brief": shows("file"), "file": "draft.md" }))).unwrap().brief[1].shows, Some(Shows::File));
        let twice = json!([{ "label": "Who", "text": "Tim", "shows": "file" }, { "label": "Draft", "text": "Here.", "shows": "file" }]);
        assert_eq!(brief_err(twice), "One brief row at most shows each attachment: 'Who' and 'Draft' both show it.");
        assert!(brief_err(json!("The fix: show it.")).starts_with("brief is a list of rows"));
        assert!(brief_err(json!([row("Who", "Tim"), "Draft"])).starts_with("Each brief row is"));
        assert!(brief_err(json!([row("Who", "Tim"), { "label": "Draft", "text": "Here.", "shows": true }])).starts_with("Each brief row is"));
    }

    #[test]
    fn keys_are_short_plain_names() {
        assert_eq!(ask_key(" ask:Spring  Sale ").unwrap(), "ask:spring-sale");
        assert_eq!(key_label("ask:pr:12"), "pr:12");
        for bad in ["x", "pr 12!", &"k".repeat(61)] {
            assert!(ask_key(bad).unwrap_err().message.starts_with("key names what the card is about"), "{bad}");
        }
    }

    fn items() -> CardAsk {
        let item = |id: &str| CardItem { id: id.into(), text: format!("Draft {id}"), about: None, recommended: false };
        CardAsk::Items { verb: "Post".into(), items: vec![item("d1"), item("d2"), item("d3"), item("d4")] }
    }

    fn pick(id: &str, tick: bool, note: Option<&str>) -> ItemPick {
        ItemPick { id: id.into(), tick, note: note.map(String::from) }
    }

    #[test]
    fn answers_are_fixed_text() {
        let choices = CardAsk::Choices { choices: vec!["Try it".into(), "Try it for a week".into()], recommended: 0 };
        assert_eq!(answer_text(&choices, &CardReply::Choice { index: 0 }, "Pat").unwrap().as_deref(), Some("Chose: Try it."));
        assert_eq!(answer_text(&choices, &CardReply::Choice { index: 2 }, "Pat").unwrap_err().message, "That answer does not fit this card.");
        for ask in [&choices, &items()] {
            assert_eq!(answer_text(ask, &CardReply::Reject, "Pat").unwrap().as_deref(), Some(REJECTED));
        }
        assert!(answer_text(&choices, &CardReply::Done, "Pat").is_err());

        let picks = vec![pick("d1", true, None), pick("d2", false, Some("too late")), pick("d3", true, Some(" ")), pick("d4", false, None)];
        assert_eq!(
            answer_text(&items(), &CardReply::Items { picks }, "Pat").unwrap().as_deref(),
            Some("Post d1, d3. Skip d2 (Pat: 'too late'). No answer on d4.")
        );
        let noted = vec![pick("d2", true, Some("shorter"))];
        assert_eq!(answer_text(&items(), &CardReply::Items { picks: noted }, "Pat").unwrap().as_deref(), Some("Post d2 (Pat: 'shorter'). No answer on d1, d3, d4."));
        assert_eq!(answer_text(&items(), &CardReply::Items { picks: vec![pick("d1", false, None)] }, "Pat").unwrap_err().message, "Tick or note at least one item.");
        assert_eq!(answer_text(&items(), &CardReply::Items { picks: vec![pick("d9", true, None)] }, "Pat").unwrap_err().message, "'d9' is not on this card.");

        let chrome = CardAsk::Chore { subject: CHROME.into(), url: None };
        let back = answer_text(&chrome, &CardReply::Done, "Pat").unwrap().unwrap();
        assert!(back.starts_with("Pat pressed Done on reconnecting Chrome. Check with list_connected_browsers first."), "{back}");
        assert!(back.contains("If it is still not connected, say so in one line and stop. Do not raise another Chrome card"), "{back}");
        let acme = CardAsk::Chore { subject: sign_in("acme"), url: Some("https://acme.example.com".into()) };
        assert_eq!(answer_text(&acme, &CardReply::Done, "Pat").unwrap().as_deref(), Some("Pat signed in to acme. Carry on with what was waiting on it."));
        assert_eq!(answer_text(&acme, &CardReply::Dismiss, "Pat").unwrap(), None);
        assert!(answer_text(&acme, &CardReply::Reject, "Pat").is_err(), "a chore is done or not now");
        assert!(answer_text(&acme, &CardReply::Choice { index: 0 }, "Pat").is_err());

        let task = CardAsk::Task { steps: "Sign the form.".into(), url: None };
        assert_eq!(answer_text(&task, &CardReply::Done, "Pat").unwrap().as_deref(), Some("Pat did it."));
        assert_eq!(answer_text(&task, &CardReply::Cant, "Pat").unwrap().as_deref(), Some("Pat can't do this. Find another way, or say exactly what you need from them."));
        assert!(answer_text(&task, &CardReply::Reject, "Pat").is_err(), "Can't is a task's no");
        assert!(answer_text(&CardAsk::Question, &CardReply::Done, "Pat").is_err(), "a question is answered in words");
        assert_eq!(answer_text(&CardAsk::News, &CardReply::Seen, "Pat").unwrap(), None, "seen sends nothing");
        assert!(answer_text(&CardAsk::News, &CardReply::Done, "Pat").is_err(), "news is seen, or followed up in words");
        assert!(answer_text(&choices, &CardReply::Seen, "Pat").is_err());

        let typed = CardReply::Text { text: " Only after the copy fix lands. ".into() };
        for ask in [&choices, &items(), &acme, &task, &CardAsk::Question, &CardAsk::News] {
            assert_eq!(answer_text(ask, &typed, "Pat").unwrap().as_deref(), Some("Only after the copy fix lands."), "your words on any card");
        }
        assert_eq!(answer_text(&choices, &CardReply::Text { text: " ".into() }, "Pat").unwrap_err().message, "Type your answer first.");
        assert!(answer_text(&choices, &CardReply::Ask { text: "Why?".into() }, "Pat").is_err(), "a question answers nothing");
    }

    #[test]
    fn later_brings_a_card_back_once_then_parks_it() {
        let at: Ts = "2026-10-30T02:00:00Z".parse().unwrap();
        let until = at + chrono::Duration::days(3);
        let mut c = Card::new("Book the venue?", None, Some("ask:venue".into()), CardAsk::Choices { choices: vec!["Book it".into()], recommended: 0 });
        answer(&mut c, &CardReply::Later { until }, "Pat", at).unwrap();
        let a = c.answer.clone().unwrap();
        assert_eq!((a.how, a.text.as_str(), c.back_at, c.parked), (AnswerHow::Dismissed, later_text("Pat", until).as_str(), Some(until), false));
        let day = until.with_timezone(&Local).format("%A, %-d %B").to_string();
        assert_eq!(a.text, format!("Later: Pat is busy, not a no. The card comes back to them on {day} by itself. Leave it as it is."));
        // Put off, it still waits: an answer before its day takes the place of Later, so it never comes back.
        let mut early = c.clone();
        answer(&mut early, &CardReply::Choice { index: 0 }, "Pat", at).unwrap();
        assert_eq!((early.answer.clone().map(|a| (a.how, a.text)), early.back_at), (Some((AnswerHow::Tapped, "Chose: Book it.".into())), None));
        assert_eq!(answer(&mut early, &CardReply::Choice { index: 0 }, "Pat", at).unwrap_err().message, "That card was already answered.");

        // It came back once, so Later parks it.
        (c.answer, c.back_at, c.times_back) = (None, None, 1);
        answer(&mut c, &CardReply::Later { until }, "Pat", at).unwrap();
        assert_eq!((c.answer.as_ref().map(|a| a.text.clone()), c.back_at, c.parked), (Some(parked_text("Pat")), None, true));
        assert_eq!(parked_text("Pat"), "Later again: Pat parked it in their Parked list. It no longer comes back by itself.");
        assert_eq!(answer(&mut c, &CardReply::Later { until }, "Pat", at).unwrap_err().message, "That card is parked already: answer it, or leave it in Parked.");
        answer(&mut c, &CardReply::Choice { index: 0 }, "Pat", at).unwrap();
        assert_eq!((c.answer.as_ref().map(|a| (a.how, a.text.clone())), c.parked), (Some((AnswerHow::Tapped, "Chose: Book it.".into())), false), "any answer takes its place");

        let mut q = Card::new("Which bank holds the account?", None, Some("ask:bank".into()), CardAsk::Question);
        assert_eq!(answer(&mut q, &CardReply::Later { until: at }, "Pat", at).unwrap_err().message, "Pick a time after now.");
        answer(&mut q, &CardReply::Dismiss, "Pat", at).unwrap();
        assert_eq!(q.back_at, Some(tomorrow_morning(at)), "Not now from an older Inbox is Later until tomorrow morning");
        let mut news = Card::new("The sale is live.", None, Some("ask:sale".into()), CardAsk::News);
        assert!(answer(&mut news, &CardReply::Later { until }, "Pat", at).is_err(), "news is only seen");
        assert!(news.answer.is_none(), "left as it was");
        let mut chore = Card::new("Reconnect Chrome.", None, None, CardAsk::Chore { subject: CHROME.into(), url: None });
        answer(&mut chore, &CardReply::Dismiss, "Pat", at).unwrap();
        assert_eq!((chore.answer.as_ref().map(|a| a.text.as_str()), chore.back_at), (Some(""), None), "a chore's Not now sends nothing and never comes back");
    }

    #[test]
    fn no_card_text_has_an_em_dash() {
        // Every text here reaches an employee or you: the answers, Later's words, and what a card that does not fit says.
        assert!(!include_str!("cards.rs").contains('\u{2014}'));
    }

    #[test]
    fn tomorrow_morning_is_nine_on_the_next_local_day() {
        let at: Ts = "2026-10-30T23:30:00Z".parse().unwrap();
        let t = tomorrow_morning(at).with_timezone(&Local);
        assert_eq!(t.date_naive(), at.with_timezone(&Local).date_naive() + chrono::Days::new(1));
        assert_eq!(t.format("%H:%M").to_string(), "09:00");
    }

    #[test]
    fn a_choice_reaches_only_cards_offering_it() {
        let ours = CardAsk::Choices { choices: vec!["Merge".into(), "Merge after the fix".into()], recommended: 0 };
        let theirs = CardAsk::Choices { choices: vec!["Hold it".into(), "merge".into()], recommended: 0 };
        let other = CardAsk::Choices { choices: vec!["Hold it".into()], recommended: 0 };
        assert_eq!(same_reply(&ours, &CardReply::Choice { index: 0 }, &theirs), Some(CardReply::Choice { index: 1 }), "the same label, whatever its case");
        assert_eq!(same_reply(&ours, &CardReply::Choice { index: 0 }, &other), None);
        assert_eq!(same_reply(&ours, &CardReply::Choice { index: 1 }, &CardAsk::Question), None);
        assert_eq!(same_reply(&ours, &CardReply::Reject, &other), Some(CardReply::Reject));
    }

    #[test]
    fn a_list_grows_by_id_and_the_newer_text_wins() {
        let item = |id: &str, text: &str| CardItem { id: id.into(), text: text.into(), about: None, recommended: false };
        let grown = grow([vec![item("pr:1", "Old one"), item("pr:2", "Two")], vec![item("pr:1", "New one"), item("pr:3", "Three")]]).unwrap();
        assert_eq!(grown, vec![item("pr:1", "New one"), item("pr:2", "Two"), item("pr:3", "Three")]);
        let many: Vec<CardItem> = (0..8).map(|i| item(&format!("a{i}"), "x")).collect();
        let more: Vec<CardItem> = (0..3).map(|i| item(&format!("b{i}"), "x")).collect();
        assert!(grow([many, more]).unwrap_err().message.contains("it would hold 11, past the 10 a card holds"));
    }

    #[test]
    fn a_card_about_something_written_without_it_wants_it_attached() {
        let card = |message: &str, ask: CardAsk| Card::new(message, None, Some("ask:x".into()), ask);
        let go = || CardAsk::Choices { choices: vec!["Go".into()], recommended: 0 };
        assert!(wants_attachment(&card("Merge PR #12?", go())));
        assert!(wants_attachment(&card("Send the supplier email?", go())));
        let mut why = card("Go ahead?", go());
        why.why = Some("The pull request fixes the pricing page.".into());
        assert!(wants_attachment(&why));
        assert!(!wants_attachment(&card("Book the bigger room?", go())));
        assert!(!wants_attachment(&card("Approve the draft?", CardAsk::Question)), "only decisions and tasks");
        assert!(!wants_attachment(&card("The new page is live.", CardAsk::News)));
        let mut linked = card("Merge PR #12?", go());
        linked.link = Some("https://example.com/pull/12".into());
        assert!(!wants_attachment(&linked));
        assert!(!wants_attachment(&card("Postpone the offsite?", go())), "a whole word");
    }

    #[test]
    fn a_company_line_is_one_fact_keyed_by_its_section_and_text() {
        let c = company_line_card(" ## Who to ask ", "- Bo signs off refunds over 100.", "Three of us asked this week.").unwrap();
        assert_eq!(c.message, ADD_LINE);
        assert_eq!(c.why.as_deref(), Some("Three of us asked this week."));
        assert_eq!(c.ask, CardAsk::Choices { choices: vec!["Add it".into()], recommended: 0 });
        assert_eq!(c.proposal, Some(Proposal::CompanyLine { section: "Who to ask".into(), line: "Bo signs off refunds over 100.".into() }));
        let key = c.key.clone().unwrap();
        assert!(key.starts_with("company:who-to-ask:") && key.len() == "company:who-to-ask:".len() + 8, "{key}");
        assert_eq!(company_line_card("Who to ask", "Bo signs off refunds over 100.", "Again.").unwrap().key, Some(key.clone()), "the same line is the same card");
        assert_ne!(company_line_card("Who to ask", "Cy signs off refunds.", "Again.").unwrap().key, Some(key));
        assert!(company_line_card(" ", "A fact.", "Why.").unwrap_err().message.starts_with("section is the page's heading"));
        assert!(company_line_card("Tools", &"y".repeat(301), "Why.").unwrap_err().message.starts_with("line is one fact for everyone"));
        assert!(company_line_card("Tools", "We use Acme Mail.", "").unwrap_err().message.starts_with("why is one line"));

        let mut c = c;
        answer(&mut c, &CardReply::Choice { index: 0 }, "Pat", Utc::now()).unwrap();
        assert_eq!(c.answer.map(|a| a.text), Some("Added to the company page under Who to ask.".into()));
        let mut no = company_line_card("Tools", "We use Acme Mail.", "Asked twice.").unwrap();
        answer(&mut no, &CardReply::Reject, "Pat", Utc::now()).unwrap();
        assert_eq!(no.answer.map(|a| a.text), Some("Not added.".into()));
    }
}
