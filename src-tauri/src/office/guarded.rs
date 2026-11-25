//! The company page, the guard on it and on what each employee owns, and carrying out the cards that propose a change
//! to either.

use super::*;
use super::skills::record;
use crate::cards;

impl Office {
    /// The page every employee reads at the start of every run; empty until it exists.
    pub fn company_page(&self) -> Result<CompanyPage> {
        let text = fs::read_to_string(own_skills::company_page_path(&self.0.data_dir.join("employees"))).unwrap_or_default();
        Ok(CompanyPage { findings: health::page(&text), text })
    }

    /// You edit the company page yourself. A plain write with no revision: you are its author.
    pub fn save_company_page(&self, text: &str) -> Result<CompanyPage> {
        fs::write(own_skills::company_page_path(&self.0.data_dir.join("employees")), text)?;
        *lock(&self.0.company_saved) = Some(now());
        self.company_page()
    }

    /// You saved the company page in You at or after `since`, in this session.
    pub(super) fn company_saved_since(&self, since: Ts) -> bool {
        lock(&self.0.company_saved).is_some_and(|at| at >= since)
    }

    /// Raises a flag on a live run without a desktop notification.
    fn flag(&self, run_id: &str, message: String) {
        if let Some(a) = lock(&self.0.active).get_mut(run_id) {
            a.needs_you.push(message);
        }
    }

    /// Handles a run that changed the company page or what its employee owns while you were not in it:
    /// `PutBack` writes `before` back (`None` removes the file) and puts the employee's version to you on a card you can
    /// use with one tap (`propose_guarded`), and `FlagOnly`, or a put-back that removed the file, shows you what it tried
    /// in one headline. `Leave` does nothing.
    pub(super) fn put_back_guarded(
        &self,
        run: &Run,
        e: &Employee,
        key: &(String, String),
        before: Option<&String>,
        now: Option<&String>,
        action: GuardedAction,
    ) -> Result<()> {
        let (skill, file) = (key.0.as_str(), key.1.as_str());
        let put_back = match action {
            GuardedAction::Leave => return Ok(()),
            GuardedAction::PutBack => true,
            GuardedAction::FlagOnly => false,
        };
        if put_back {
            let path = own_skills::recorded_path(&self.employee_dir(&e.id), skill, file)?;
            own_skills::write_recorded(&path, skill, before.map(String::as_str))?;
            if let Some(text) = now {
                match self.propose_guarded(run, e, skill, before.map(String::as_str), text) {
                    Ok(()) => return Ok(()),
                    Err(err) => log!("run {}: proposing {skill}/{file} to you failed: {err}", run.id),
                }
            }
        }
        let name = &e.name;
        let parts: Vec<String> = if skill == own_skills::COMPANY {
            let (added, removed) = line_changes(before.map(String::as_str), now.map(String::as_str));
            let head = if put_back {
                format!("{name} tried to change the company page in a run nobody was watching, so it was put back.")
            } else {
                format!("{name} changed the company page in a run nobody was watching while you were also changing it, so it was not put back.")
            };
            let tail = if put_back { format!("If you want this, tell {name} in a conversation or edit the page in You.") } else { "Check the page in You.".to_string() };
            let added = (!added.is_empty()).then(|| format!("It added: \"{added}\""));
            let removed = (!removed.is_empty()).then(|| format!("It removed: \"{removed}\""));
            [Some(head), added, removed, Some(tail)].into_iter().flatten().collect()
        } else {
            let text = claude::clip(&claude::one_line(now.map(String::as_str).unwrap_or_default()), 500);
            if put_back {
                vec![format!("{name} proposes what they own: \"{text}\" It was put back until you agree. To use it, edit What {name} owns on their profile, or tell them in a conversation.")]
            } else {
                vec![format!("{name} changed what they own in a run nobody was watching while you were also changing it, so it was not put back: \"{text}\" Check it on their profile.")]
            }
        };
        self.flag(&run.id, parts.join(" "));
        Ok(())
    }

    /// Saves the version of what they own (`skill` OWN) or of the company page that `e`'s run wrote while nobody was
    /// watching to `workspace/proposals/` in their folder, and puts it to you on a card on that run: Use it writes it in
    /// (`carry_out`). A newer proposal about the same file takes the older one's place, with no new notification.
    fn propose_guarded(&self, run: &Run, e: &Employee, skill: &str, before: Option<&str>, text: &str) -> Result<()> {
        let (name, key, message, proposal) = if skill == own_skills::COMPANY {
            ("company-page.md", format!("company-page:{}", e.slug), format!("Use {}'s change to the company page?", e.name), Proposal::CompanyPage)
        } else {
            ("what-you-own.md", format!("owns:{}", e.slug), format!("Use {}'s update to what they own?", e.name), Proposal::Owns)
        };
        let dir = self.employee_dir(&e.id).join("workspace").join("proposals");
        fs::create_dir_all(&dir)?;
        let path = dir.join(name);
        own_skills::write_recorded(&path, own_skills::NOTES, Some(text))?;
        let why = match line_changes(before, Some(text)) {
            (a, r) if a.is_empty() && r.is_empty() => None,
            (a, r) if r.is_empty() => Some(format!("Adds \"{a}\"")),
            (a, r) if a.is_empty() => Some(format!("Removes \"{r}\"")),
            (a, r) => Some(format!("Adds \"{a}\", removes \"{r}\"")),
        };
        let why = why.map(|w| claude::clip(&w, cards::WHY_CHARS));
        let mut card = Card::new(&message, why, Some(key), CardAsk::Choices { choices: vec!["Use it".into()], recommended: 0 });
        card.file = Some(fs::canonicalize(&path)?.to_string_lossy().into_owned());
        card.proposal = Some(proposal);
        self.ask_card(&run.id, e, card, false)?;
        Ok(())
    }

    /// Carries out a proposal card `run` raised, now that you went ahead: adds a line to the company page, or writes the
    /// employee's saved version (the card's file) into what they own or the company page. You decided, so it stands like
    /// your own edit: a run that changed the same file meanwhile has its change shown to you, not put back over it.
    pub(super) fn carry_out(&self, db: &Store, run: &Run, card: &Card) -> Result<()> {
        let page = own_skills::company_page_path(&self.0.data_dir.join("employees"));
        let saved = || -> Result<String> {
            let path = card.file.as_deref().ok_or_else(|| AppError::invalid("That proposal has no saved version."))?;
            fs::read_to_string(path).map_err(|_| AppError::not_found(format!("Nothing is at {path} any more.")))
        };
        match &card.proposal {
            Some(Proposal::CompanyLine { section, line }) => {
                let text = fs::read_to_string(&page).unwrap_or_default();
                self.save_company_page(&with_line(&text, section, line))?;
            }
            Some(Proposal::CompanyPage) => {
                self.save_company_page(&saved()?)?;
            }
            Some(Proposal::Owns) => {
                record(db, &self.employee_dir(&run.employee_id), &run.employee_id, own_skills::OWN, own_skills::OWN_FILE, Some(&saved()?))?;
            }
            None => {}
        }
        Ok(())
    }
}

/// What happens to a guarded file a run changed while you were not in it.
#[derive(Debug, PartialEq)]
pub(super) enum GuardedAction {
    /// The run did not write it: someone else did, and their run or editor records it.
    Leave,
    PutBack,
    /// Shown to you but kept: putting it back could lose your own edit.
    FlagOnly,
}

/// Decides by what this run did, not by what else is running. Pure.
pub(super) fn guarded_action(wrote: bool, ceo_talking: bool, revised_since_start: bool) -> GuardedAction {
    match (wrote, ceo_talking || revised_since_start) {
        (false, _) => GuardedAction::Leave,
        (true, true) => GuardedAction::FlagOnly,
        (true, false) => GuardedAction::PutBack,
    }
}

/// (added, removed): the lines only `now` has and the lines only `before` has, trimmed, without blank lines and
/// headings, each joined with "; " and clipped to 500 characters. Pure.
fn line_changes(before: Option<&str>, now: Option<&str>) -> (String, String) {
    fn lines(t: Option<&str>) -> Vec<&str> {
        t.unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect()
    }
    fn only(xs: &[&str], ys: &[&str]) -> String {
        claude::clip(&xs.iter().filter(|x| !ys.contains(x)).copied().collect::<Vec<_>>().join("; "), 500)
    }
    let (b, n) = (lines(before), lines(now));
    (only(&n, &b), only(&b, &n))
}

/// `page` with `line` as a bullet under its `## <section>` heading (any case): first in a section that says it is
/// newest first, else after the section's last line. A missing section is added at the end, and a line already
/// there is not added again. Pure.
fn with_line(page: &str, section: &str, line: &str) -> String {
    let bullet = format!("- {line}");
    let mut lines: Vec<String> = page.lines().map(String::from).collect();
    let heading = |l: &str| l.strip_prefix("## ").is_some_and(|h| h.trim().eq_ignore_ascii_case(section));
    let Some(start) = lines.iter().position(|l| heading(l)) else {
        let mut out = page.trim_end().to_string();
        if !out.is_empty() {
            out += "\n\n";
        }
        return format!("{out}## {section}\n\n{bullet}\n");
    };
    let end = lines[start + 1..].iter().position(|l| l.starts_with("# ") || l.starts_with("## ")).map_or(lines.len(), |i| start + 1 + i);
    let body = &lines[start + 1..end];
    if body.iter().any(|l| l.trim() == bullet) {
        return page.to_string();
    }
    let bulleted = |l: &str| l.trim_start().starts_with("- ");
    let first = body.iter().position(|l| bulleted(l)).filter(|_| body.iter().any(|l| l.to_lowercase().contains("newest first")));
    let (at, after) = match first {
        Some(i) => (start + 1 + i, None),
        None => match body.iter().rposition(|l| !l.trim().is_empty()) {
            Some(i) => (start + 2 + i, Some(body[i].clone())),
            None => (start + 1, None),
        },
    };
    // A bullet goes straight under another; after a heading or a line of text, a blank line comes first.
    let mut add = vec![bullet];
    if first.is_none() && !after.is_some_and(|l| bulleted(&l)) {
        add.insert(0, String::new());
    }
    for (k, l) in add.into_iter().enumerate() {
        lines.insert(at + k, l);
    }
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{self, open_office, temp_path};

    #[tokio::test]
    async fn a_change_to_what_they_own_put_back_comes_to_you_as_a_card_to_use() {
        let dir = temp_path("own-proposal");
        let e = fixtures::employee();
        let mut run = Run::new(&e.id, RunKind::Manual, "Review your week.".into(), now());
        run.state = RunState::Succeeded;
        run.ended_at = Some(now());
        {
            fs::create_dir_all(&dir).unwrap();
            let db = Store::open(&dir.join("office.db")).unwrap();
            db.save_employee(&e).unwrap();
            db.set_held_until(Some(now() + chrono::Duration::hours(1))).unwrap();
            assert!(db.insert_run(&run, &run.id).unwrap());
        }
        let office = open_office(&dir).await;
        let own = office.employee_dir(&e.id).join(own_skills::OWN_FILE);
        let (before, theirs) = ("The signups.".to_string(), "The signups and the churn.".to_string());
        fs::create_dir_all(own.parent().unwrap()).unwrap();
        fs::write(&own, &theirs).unwrap();
        let key = (own_skills::OWN.to_string(), own_skills::OWN_FILE.to_string());
        office.put_back_guarded(&run, &e, &key, Some(&before), Some(&theirs), GuardedAction::PutBack).unwrap();
        assert_eq!(fs::read_to_string(&own).unwrap(), before, "put back until you decide");
        let card = office.run(&run.id).unwrap().cards.remove(0);
        assert_eq!((card.message.as_str(), card.key.as_deref(), card.proposal.clone()), ("Use Ada's update to what they own?", Some("owns:ada"), Some(Proposal::Owns)));
        assert_eq!(card.why.as_deref(), Some("Adds \"The signups and the churn.\", removes \"The signups.\""));
        let saved = card.file.clone().unwrap();
        assert!(Path::new(&saved).ends_with("workspace/proposals/what-you-own.md") && fs::read_to_string(&saved).unwrap() == theirs, "{saved}");
        assert!(office.run(&run.id).unwrap().flags().next().is_none(), "a card in place of the flag");

        let told = office.answer_card(&run.id, 0, CardReply::Choice { index: 0 }).unwrap();
        assert_eq!(fs::read_to_string(&own).unwrap(), theirs, "Use it writes their version in");
        assert!(told[0].prompt.ends_with("Used: your update to what you own is in what-you-own.md now."), "{}", told[0].prompt);
        assert_eq!(office.skill_history(&e.id).unwrap()[0].after.as_deref(), Some(theirs.as_str()), "recorded, so it can be undone");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_company_line_goes_under_its_heading_newest_first_where_the_section_says_so() {
        let page = "# Company page\n\nIntro.\n\n## Who to ask\n\n- Bo: refunds.\n\n## Right now\n\nDated lines, newest first.\n\n- 1 Oct: old news.\n";
        let added = with_line(page, "who to ask", "Cy: the rota.");
        assert!(added.contains("## Who to ask\n\n- Bo: refunds.\n- Cy: the rota.\n\n## Right now"), "{added}");
        let newest = with_line(page, "Right now", "2 Oct: new news.");
        assert!(newest.contains("Dated lines, newest first.\n\n- 2 Oct: new news.\n- 1 Oct: old news.\n"), "{newest}");
        assert_eq!(with_line(&added, "Who to ask", "Cy: the rota."), added, "a line already there is not added again");
        let new_section = with_line(page, "Tools", "We use Acme Mail.");
        assert!(new_section.ends_with("- 1 Oct: old news.\n\n## Tools\n\n- We use Acme Mail.\n"), "{new_section}");
        assert_eq!(with_line("## What we sell\n\n## Who to ask\n", "What we sell", "Cakes."), "## What we sell\n\n- Cakes.\n\n## Who to ask\n");
        assert_eq!(
            with_line("## Right now\n\nDated lines, newest first.\n\n## Decisions\n", "Right now", "3 Oct: moved."),
            "## Right now\n\nDated lines, newest first.\n\n- 3 Oct: moved.\n\n## Decisions\n"
        );
        assert_eq!(with_line("", "Tools", "We use Acme Mail."), "## Tools\n\n- We use Acme Mail.\n");
    }

    #[tokio::test]
    async fn a_save_in_you_is_seen_by_a_runs_guard() {
        let dir = temp_path("page");
        let office = open_office(&dir).await;
        let started = now();
        assert!(!office.company_saved_since(started), "nothing saved yet");
        office.save_company_page("# Company page\n\n- Releases: Sam.\n").unwrap();
        assert!(office.company_saved_since(started), "a run that started before the save counts it as your edit");
        assert!(!office.company_saved_since(now() + chrono::Duration::seconds(1)), "a run that started after it does not");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn guarded_action_puts_back_only_what_this_run_wrote_alone() {
        assert_eq!(guarded_action(false, false, false), GuardedAction::Leave);
        assert_eq!(guarded_action(false, true, true), GuardedAction::Leave);
        assert_eq!(guarded_action(true, false, false), GuardedAction::PutBack);
        assert_eq!(guarded_action(true, true, false), GuardedAction::FlagOnly);
        assert_eq!(guarded_action(true, false, true), GuardedAction::FlagOnly);
    }

    #[test]
    fn line_changes_quote_added_and_removed_lines() {
        let before = "# Company page\n\n## Who to ask\n\n- Releases: Sam.\n";
        assert_eq!(line_changes(Some(before), Some(&format!("{before}- Billing: Kim.\n- Copy: Lee.\n"))), ("- Billing: Kim.; - Copy: Lee.".into(), String::new()));
        assert_eq!(line_changes(Some(before), Some("# Company page\n\n## Who to ask\n")), (String::new(), "- Releases: Sam.".into()));
        assert_eq!(line_changes(Some(before), Some("# Company\n\n## Ask\n\n- Releases: Kim.\n")), ("- Releases: Kim.".into(), "- Releases: Sam.".into()));
        assert_eq!(line_changes(None, Some("# Page\n## Decisions\n")), (String::new(), String::new()), "headings are skipped");
    }
}
