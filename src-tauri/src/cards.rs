//! Cards: what an employee asks you with the answer built in (`notify` with choices, items or chore), and
//! the fixed text your answer sends back. Pure.

use crate::claude::{clip, one_line};
use crate::model::*;
use serde_json::Value;
use std::collections::HashSet;

/// The chore of reconnecting Claude in Chrome.
pub const CHROME: &str = "chrome";
const SIGN_IN: &str = "sign-in:";
const MAX_CHOICES: usize = 4;
const CHOICE_CHARS: usize = 40;
const MAX_ITEMS: usize = 10;
const ITEM_ID_CHARS: usize = 20;
const ITEM_CHARS: usize = 300;
const VERB_CHARS: usize = 20;
const NOTE_CHARS: usize = 300;
const DEFAULT_VERB: &str = "Approve";

/// The chore of signing in to `site`.
pub fn sign_in(site: &str) -> String {
    format!("{SIGN_IN}{site}")
}

/// A chore's subject as stored: lower case without spaces, exactly `chrome` or `sign-in:<site>`.
pub fn chore_subject(raw: &str) -> Result<String> {
    let s: String = raw.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase();
    let site_ok = |site: &str| (1..=40).contains(&site.len()) && site.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-');
    match s.strip_prefix(SIGN_IN) {
        _ if s == CHROME => Ok(s),
        Some(site) if site_ok(site) => Ok(sign_in(site)),
        _ => Err(AppError::invalid("chore must be 'chrome' or 'sign-in:<site>', for example 'sign-in:wise'.")),
    }
}

/// The card `notify`'s arguments ask for, if any: at most one of choices, items and chore. An empty list or
/// text counts as left out.
pub fn ask(args: &Value) -> Result<Option<CardAsk>> {
    let list = |k: &str| args.get(k).and_then(Value::as_array).filter(|a| !a.is_empty());
    let text = |k: &str| args.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let (choices, items, chore) = (list("choices"), list("items"), text("chore"));
    if [choices.is_some(), items.is_some(), chore.is_some()].iter().filter(|x| **x).count() > 1 {
        return Err(AppError::invalid("Use one of choices, items or chore in a card, not several."));
    }
    if let Some(c) = choices {
        return choices_ask(c).map(Some);
    }
    if let Some(i) = items {
        return items_ask(i, text("itemVerb")).map(Some);
    }
    chore.map(|c| chore_subject(c).map(|subject| CardAsk::Chore { subject })).transpose()
}

fn choices_ask(raw: &[Value]) -> Result<CardAsk> {
    if !(2..=MAX_CHOICES).contains(&raw.len()) {
        return Err(AppError::invalid(format!("Give 2 to {MAX_CHOICES} choices.")));
    }
    let mut choices: Vec<String> = vec![];
    let mut recommended = vec![];
    for (i, c) in raw.iter().enumerate() {
        let label = c.get("label").and_then(Value::as_str).map(str::trim).unwrap_or("");
        if label.is_empty() || label.chars().count() > CHOICE_CHARS {
            return Err(AppError::invalid(format!("Each choice needs a label of 1 to {CHOICE_CHARS} characters.")));
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
        let recommended = i.get("recommended").and_then(Value::as_bool).unwrap_or(false);
        items.push(CardItem { id: id.to_string(), text: clip(text, ITEM_CHARS), recommended });
    }
    Ok(CardAsk::Items { verb: verb.to_string(), items })
}

/// The fixed message your answer sends to the employee, or None when it sends nothing (a dismissed chore).
pub fn answer_text(ask: &CardAsk, reply: &CardReply, ceo: &str) -> Result<Option<String>> {
    let unfit = || AppError::invalid("That answer does not fit this card.");
    let text = match (ask, reply) {
        (CardAsk::Choices { choices, .. }, CardReply::Choice { index }) => format!("Chose: {}.", choices.get(*index).ok_or_else(unfit)?),
        (CardAsk::Items { verb, items }, CardReply::Items { picks }) => items_text(verb, items, picks, ceo)?,
        (CardAsk::Choices { .. } | CardAsk::Items { .. }, CardReply::Dismiss) => "No answer. Go with the safe default you stated.".into(),
        (CardAsk::Chore { subject }, CardReply::Done) => match subject.strip_prefix(SIGN_IN) {
            Some(site) => format!("{ceo} signed in to {site}. Carry on with what was waiting on it."),
            None => format!("{ceo} reconnected Chrome. Carry on with what was waiting on it."),
        },
        (CardAsk::Chore { .. }, CardReply::Dismiss) => return Ok(None),
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

    fn err(args: Value) -> String {
        ask(&args).unwrap_err().message
    }

    #[test]
    fn choices_need_two_to_four_distinct_labels_and_one_recommendation() {
        let ok = ask(&json!({ "message": "Boost?", "choices": [{ "label": " Not now ", "recommended": true }, { "label": "Try A$5 a day" }] })).unwrap();
        assert_eq!(ok, Some(CardAsk::Choices { choices: vec!["Not now".into(), "Try A$5 a day".into()], recommended: 0 }));
        assert_eq!(err(json!({ "choices": [{ "label": "Yes", "recommended": true }] })), "Give 2 to 4 choices.");
        let five: Vec<Value> = (0..5).map(|i| json!({ "label": format!("Option {i}"), "recommended": i == 0 })).collect();
        assert_eq!(err(json!({ "choices": five })), "Give 2 to 4 choices.");
        assert_eq!(err(json!({ "choices": [{ "label": "Yes" }, { "label": "No" }] })), "Mark exactly one choice with recommended: true: the one you recommend.");
        assert_eq!(err(json!({ "choices": [{ "label": "Yes", "recommended": true }, { "label": "No", "recommended": true }] })), "Mark exactly one choice with recommended: true: the one you recommend.");
        assert!(err(json!({ "choices": [{ "label": "Yes", "recommended": true }, { "label": "yes" }] })).contains("'yes' is there twice"));
        assert!(err(json!({ "choices": [{ "label": "Yes", "recommended": true }, { "label": " " }] })).starts_with("Each choice needs a label"));
        assert!(err(json!({ "choices": [{ "label": "Yes", "recommended": true }, { "label": "x".repeat(41) }] })).starts_with("Each choice needs a label"));
    }

    #[test]
    fn items_need_short_unique_ids_and_text() {
        let long = "y".repeat(400);
        let ok = ask(&json!({ "items": [{ "id": " d1 ", "text": "Reply to the launch post", "recommended": true }, { "id": "d2", "text": long }], "itemVerb": " Post " })).unwrap();
        let Some(CardAsk::Items { verb, items }) = ok else { panic!("items") };
        assert_eq!(verb, "Post");
        assert_eq!(items[0], CardItem { id: "d1".into(), text: "Reply to the launch post".into(), recommended: true });
        assert!(!items[1].recommended && items[1].text.chars().count() == 303, "clipped to 300 and an ellipsis");
        let Some(CardAsk::Items { verb, .. }) = ask(&json!({ "items": [{ "id": "a", "text": "One" }] })).unwrap() else { panic!("items") };
        assert_eq!(verb, "Approve");
        let eleven: Vec<Value> = (0..11).map(|i| json!({ "id": format!("d{i}"), "text": "x" })).collect();
        assert_eq!(err(json!({ "items": eleven })), "Give at most 10 items.");
        assert!(err(json!({ "items": [{ "id": "d1", "text": "a" }, { "id": "d1", "text": "b" }] })).contains("'d1' is there twice"));
        assert!(err(json!({ "items": [{ "id": "x".repeat(21), "text": "a" }] })).starts_with("Each item needs an id"));
        assert_eq!(err(json!({ "items": [{ "id": "d1", "text": " " }] })), "Item 'd1' needs its text.");
        assert!(err(json!({ "items": [{ "id": "d1", "text": "a" }], "itemVerb": "x".repeat(21) })).starts_with("itemVerb"));
    }

    #[test]
    fn a_card_asks_for_one_thing() {
        assert_eq!(ask(&json!({ "message": "FYI" })).unwrap(), None);
        assert_eq!(ask(&json!({ "message": "FYI", "choices": [], "chore": "" })).unwrap(), None);
        assert_eq!(
            err(json!({ "choices": [{ "label": "A", "recommended": true }, { "label": "B" }], "chore": "chrome" })),
            "Use one of choices, items or chore in a card, not several."
        );
    }

    #[test]
    fn a_chore_is_chrome_or_a_sign_in() {
        assert_eq!(ask(&json!({ "chore": "Sign-In: Wise" })).unwrap(), Some(CardAsk::Chore { subject: "sign-in:wise".into() }));
        assert_eq!(chore_subject(" Chrome ").unwrap(), CHROME);
        assert_eq!(chore_subject("sign-in:app.example-bank.com").unwrap(), sign_in("app.example-bank.com"));
        for bad in ["gmail", "account", "sign-in:", "sign-in:wise/login", &format!("sign-in:{}", "a".repeat(41))] {
            assert_eq!(chore_subject(bad).unwrap_err().message, "chore must be 'chrome' or 'sign-in:<site>', for example 'sign-in:wise'.", "{bad}");
        }
    }

    fn items() -> CardAsk {
        let item = |id: &str| CardItem { id: id.into(), text: format!("Draft {id}"), recommended: false };
        CardAsk::Items { verb: "Post".into(), items: vec![item("d1"), item("d2"), item("d3"), item("d4")] }
    }

    fn pick(id: &str, tick: bool, note: Option<&str>) -> ItemPick {
        ItemPick { id: id.into(), tick, note: note.map(String::from) }
    }

    #[test]
    fn answers_are_fixed_text() {
        let choices = CardAsk::Choices { choices: vec!["Not now".into(), "Try it".into()], recommended: 0 };
        assert_eq!(answer_text(&choices, &CardReply::Choice { index: 0 }, "Pat").unwrap().as_deref(), Some("Chose: Not now."));
        assert_eq!(answer_text(&choices, &CardReply::Choice { index: 2 }, "Pat").unwrap_err().message, "That answer does not fit this card.");
        let dismissed = "No answer. Go with the safe default you stated.";
        assert_eq!(answer_text(&choices, &CardReply::Dismiss, "Pat").unwrap().as_deref(), Some(dismissed));
        assert_eq!(answer_text(&items(), &CardReply::Dismiss, "Pat").unwrap().as_deref(), Some(dismissed));
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

        let chrome = CardAsk::Chore { subject: CHROME.into() };
        assert_eq!(answer_text(&chrome, &CardReply::Done, "Pat").unwrap().as_deref(), Some("Pat reconnected Chrome. Carry on with what was waiting on it."));
        let wise = CardAsk::Chore { subject: sign_in("wise") };
        assert_eq!(answer_text(&wise, &CardReply::Done, "Pat").unwrap().as_deref(), Some("Pat signed in to wise. Carry on with what was waiting on it."));
        assert_eq!(answer_text(&wise, &CardReply::Dismiss, "Pat").unwrap(), None);
        assert!(answer_text(&wise, &CardReply::Choice { index: 0 }, "Pat").is_err());
    }
}
