//! Skill health: what can be noticed about an employee's skills and notes without a model.
//!
//! OmegaStar measures; the employee judges. Everything here is a fact about files (sizes, links,
//! repeated lines, edit counts), so it costs nothing to run after every run. A finding is a reason to
//! look, never an order: a long runbook may be one procedure that belongs together, and the tidy that
//! reads these findings decides.

use crate::catalog;
use crate::claude::{clip, one_line};
use crate::model::*;
use crate::own_skills::{self, CRAFT_NAME};
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::fs;
use std::path::Path;

/// A leaf's SKILL.md past this is worth a second look: several procedures, or detail every run does
/// not need. Half of a real family sits above 5k and works, so the line is set where it stops being one runbook.
const LEAF_CHARS: usize = 10_000;
/// The role skill is read at the start of every task, so it should route and not hold procedures.
pub(crate) const ROLE_CHARS: usize = 4_000;
/// A skill no run has used for this many days, once it is this old, may be one nobody needs.
const UNUSED_DAYS: i64 = 30;
/// Reference files are read on demand; past this they are probably a log.
const REFERENCE_CHARS: usize = 20_000;
/// Every description is loaded on every turn.
const DESCRIPTION_CHARS: usize = 400;
const DESCRIPTIONS_TOTAL_CHARS: usize = 12_000;
/// Dated lines in one procedure before they count as sediment.
const DATED_LINES: usize = 3;
/// Lines of a notes file that repeat an earlier one before it counts as a diary.
const REPEATED_LINES: usize = 5;
/// Edits to one file within a day before it counts as churn.
const CHURN_EDITS: usize = 3;
/// Every run of every employee reads the company page.
const PAGE_CHARS: usize = 6_000;
/// Files larger than this are binaries or data, not notes.
const TEXT_MAX_BYTES: u64 = 256 * 1024;

/// A skill as the check reads it: its name, its SKILL.md text, and its other markdown files as
/// (relative path, text).
type SkillText = (String, String, Vec<(String, String)>);

fn is_markdown(rel: &str) -> bool {
    rel.rsplit('.').next().map(|e| e.eq_ignore_ascii_case("md")).unwrap_or(false)
}

fn read_text(path: &Path) -> Option<String> {
    let small = fs::metadata(path).map(|m| m.len() <= TEXT_MAX_BYTES).unwrap_or(false);
    small.then(|| fs::read_to_string(path).ok()).flatten()
}

fn load(skills_dir: &Path) -> Vec<SkillText> {
    let mut out = vec![];
    for (name, dir) in own_skills::skill_dirs(skills_dir).into_iter().filter(|(n, _)| n != CRAFT_NAME) {
        let Some(main) = fs::read_to_string(dir.join("SKILL.md")).ok() else { continue };
        let files = own_skills::skill_files(&dir)
            .into_iter()
            .filter(|(rel, _)| is_markdown(rel) && rel != "SKILL.md")
            .filter_map(|(rel, path)| Some((rel, read_text(&path)?)))
            .collect();
        out.push((name, main, files));
    }
    out
}

/// The notes an employee keeps outside their skills: markdown directly in `workspace/`.
fn load_notes(notes_dir: &Path) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = fs::read_dir(notes_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_file() && is_markdown(&e.file_name().to_string_lossy()))
        .filter_map(|e| Some((format!("workspace/{}", e.file_name().to_string_lossy()), read_text(&e.path())?)))
        .collect();
    v.sort();
    v
}

/// Lines of prose with their 1-based numbers: frontmatter and fenced code are skipped.
fn prose(text: &str) -> Vec<(usize, &str)> {
    let mut out = vec![];
    let (mut fence, mut front) = (false, false);
    for (i, line) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        let t = line.trim();
        if i == 0 && t == "---" {
            front = true;
            continue;
        }
        if front {
            front = t != "---";
            continue;
        }
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = !fence;
            continue;
        }
        if !fence {
            out.push((i + 1, line));
        }
    }
    out
}

fn has_date(line: &str) -> bool {
    line.as_bytes().windows(10).any(|w| {
        w[0] == b'2' && w[1] == b'0' && w[2].is_ascii_digit() && w[3].is_ascii_digit() && w[4] == b'-' && w[5].is_ascii_digit()
            && w[6].is_ascii_digit() && w[7] == b'-' && w[8].is_ascii_digit() && w[9].is_ascii_digit()
    })
}

/// The line without its "(taught ...)", "(checked ...)" and "(confirmed by ...)" marks: the date in a
/// mark is bookkeeping, not a fact that ages.
fn without_marks(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    let (mut out, mut i) = (String::new(), 0);
    while let Some(start) = ["(taught", "(checked", "(confirmed by"].iter().filter_map(|m| lower[i..].find(m)).min().map(|at| i + at) {
        out.push_str(&line[i..start]);
        i = lower[start..].find(')').map(|end| start + end + 1).unwrap_or(line.len());
    }
    out.push_str(&line[i..]);
    out
}

/// The line with every backtick span blanked to spaces, so a file name or command in code is not read
/// as words. Same number of characters, so positions still point into the original line.
fn without_code(line: &str) -> String {
    let (mut out, mut rest) = (String::new(), line);
    while let Some(open) = rest.find('`') {
        let Some(len) = rest[open + 1..].find('`') else { break };
        let close = open + len + 2;
        out.push_str(&rest[..open]);
        out.extend(rest[open..close].chars().map(|_| ' '));
        rest = &rest[close..];
    }
    out.push_str(rest);
    out
}

/// Where `word` stands on its own as a word, not as part of a name or path: `todo` in `my-todo-list`,
/// `workspace/todo.md` or `todo.md` is a file, while `TODO.` ending a sentence is a note.
fn find_bare_word(hay: &str, word: &str) -> Option<usize> {
    hay.match_indices(word).map(|(i, _)| i).find(|&i| {
        let before = hay[..i].chars().next_back();
        let mut after = hay[i + word.len()..].chars();
        let next = after.next();
        let joined = |c: char| c.is_alphanumeric() || ['_', '-', '/', '\\'].contains(&c);
        !before.map(|c| joined(c) || c == '.').unwrap_or(false)
            && !next.map(|c| joined(c) || (c == '.' && after.next().map(char::is_alphanumeric).unwrap_or(false))).unwrap_or(false)
    })
}

const LOOSE_PHRASES: [&str; 13] = [
    "not yet verified", "not yet confirmed", "not yet tested", "not yet checked", "to be verified", "to be confirmed", "to be checked",
    "verify next", "check next", "verify later", "check later", "revisit", "to be decided",
];

/// Where, in characters, a line reads like a reminder to self rather than an instruction. Deliberately
/// narrow: a rule such as "never mark an unverified check as passed" is not a loose end. It will still
/// catch an instruction now and then ("when the mapping needs to be verified visually"), which is why
/// a finding only says a line may be unfinished.
fn loose_note(line: &str) -> Option<usize> {
    let l = without_code(line).to_lowercase();
    let word = ["todo", "tbc", "tbd", "undecided", "settles it"].into_iter().filter_map(|w| find_bare_word(&l, w));
    let phrase = LOOSE_PHRASES.into_iter().filter_map(|p| l.find(p));
    word.chain(phrase).min().map(|i| l[..i].chars().count())
}

/// The stretch of `line` around character `at`, so a long table row shows the words that matched.
fn excerpt(line: &str, at: usize) -> String {
    let chars: Vec<char> = line.chars().collect();
    let start = at.saturating_sub(40);
    let end = (at + 60).min(chars.len());
    let body = one_line(&chars[start.min(end)..end].iter().collect::<String>());
    format!("{}{body}{}", if start > 0 { "..." } else { "" }, if end < chars.len() { "..." } else { "" })
}

/// (line number, excerpt) of every line that looks like an unfinished note.
fn loose_notes(text: &str) -> Vec<(usize, String)> {
    prose(text).into_iter().filter_map(|(n, l)| loose_note(l).map(|at| (n, excerpt(l, at)))).collect()
}

fn sample(loose: &[(usize, String)]) -> String {
    loose.iter().take(2).map(|(n, x)| format!("line {n}: \"{x}\"")).collect::<Vec<_>>().join("; ")
}

/// A line with its digits blanked, so "3:03 pm ... fine" and "3:13 pm ... fine" are the same entry.
fn shape(line: &str) -> Option<String> {
    let t = line.trim().trim_start_matches(['-', '*', '•', ' ']).to_lowercase();
    (t.chars().count() >= 20).then(|| t.chars().map(|c| if c.is_ascii_digit() { '#' } else { c }).collect())
}

/// Lines beyond the first of any entry that appears three or more times. Pairs are ordinary: a table
/// of similar rows repeats a shape twice; a diary repeats the same one every run.
fn repeats(text: &str) -> usize {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for s in prose(text).into_iter().filter_map(|(_, l)| shape(l)) {
        *counts.entry(s).or_default() += 1;
    }
    counts.values().filter(|n| **n >= 3).map(|n| n - 1).sum()
}

fn k(n: usize) -> String {
    if n >= 1000 { format!("{:.1}k", n as f64 / 1000.0) } else { n.to_string() }
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
}

/// Skill names between backticks that begin with the family prefix.
fn named_in_code(text: &str, prefix: &str) -> BTreeSet<String> {
    text.split('`')
        .skip(1)
        .step_by(2)
        .map(str::trim)
        .filter(|s| s.starts_with(prefix) && s.len() > prefix.len() && s.chars().all(is_name_char) && !s.ends_with('-'))
        .map(String::from)
        .collect()
}

/// Whether `text` mentions the skill `name` as a whole name (`a-b` is not a mention of `a-b-c`).
fn mentions(text: &str, name: &str) -> bool {
    text.match_indices(name).any(|(i, _)| {
        !text[..i].chars().next_back().map(is_name_char).unwrap_or(false) && !text[i + name.len()..].chars().next().map(is_name_char).unwrap_or(false)
    })
}

fn is_junk(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let ext = name.rsplit('.').next().unwrap_or("");
    rel.split('/').any(|p| p == "__pycache__")
        || ["pyc", "pyo", "bak", "orig", "tmp", "swp"].iter().any(|e| ext.eq_ignore_ascii_case(e))
        || name == ".DS_Store"
        || name == "Thumbs.db"
}

/// Open checks past their date, as findings. They come from the database, not the folders, so the
/// Office adds them to what `check` found.
pub fn overdue_checks(open: &[Check], now: Ts) -> Vec<HealthFinding> {
    open.iter()
        .filter(|c| c.due_at <= now)
        .map(|c| {
            let days = (now - c.due_at).num_days();
            let late = match days {
                0 => "due today".to_string(),
                1 => "a day overdue".to_string(),
                n => format!("{n} days overdue"),
            };
            HealthFinding {
                kind: HealthKind::OverdueCheck,
                skill: None,
                file: None,
                detail: format!("\"{}\" is {late}. How to check: {}", clip(&one_line(&c.what), 140), clip(&one_line(&c.how), 160)),
            }
        })
        .collect()
}

/// What is worth a look on the company page. Kept out of `check`, which is per employee and decides
/// when they tidy: nobody tidies the company page alone.
pub fn page(text: &str) -> Vec<HealthFinding> {
    let mut f = vec![];
    let file = Some("company page".to_string());
    let loose = loose_notes(text);
    if !loose.is_empty() {
        f.push(HealthFinding { kind: HealthKind::LooseNotes, skill: None, file: file.clone(), detail: format!("{} line(s) may be unfinished notes ({}).", loose.len(), sample(&loose)) });
    }
    let n = text.chars().count();
    if n > PAGE_CHARS {
        let detail = format!("The company page is {} characters, and every run of every employee reads it. Keep one line per fact and move detail to the skill that owns it.", k(n));
        f.push(HealthFinding { kind: HealthKind::TooLong, skill: None, file, detail });
    }
    f
}

/// `used` holds when runs last used each skill, as counted since `counted_since`.
pub fn check(skills_dir: &Path, notes_dir: &Path, slug: &str, edits: &[(String, String, Ts)], used: &[(String, Ts)], counted_since: Ts, now: Ts) -> SkillHealth {
    let skills = load(skills_dir);
    let notes = load_notes(notes_dir);
    let mut f: Vec<HealthFinding> = vec![];
    let mut add = |kind: HealthKind, skill: Option<&str>, file: Option<&str>, detail: String| {
        f.push(HealthFinding { kind, skill: skill.map(String::from), file: file.map(String::from), detail });
    };
    let names: BTreeSet<&str> = skills.iter().map(|(n, _, _)| n.as_str()).collect();
    let prefix = format!("{slug}-");

    let mut description_chars = 0;
    for (name, main, files) in &skills {
        let role = name == slug;
        let fm = catalog::frontmatter(main);
        let size = main.chars().count();
        let limit = if role { ROLE_CHARS } else { LEAF_CHARS };
        if size > limit {
            let why = if role {
                "It is read at the start of every task, so it should route to procedures and hold none."
            } else {
                "Check whether it is several procedures, or holds detail that only some runs need."
            };
            add(HealthKind::TooLong, Some(name), Some("SKILL.md"), format!("SKILL.md is {} characters (about {} is comfortable). {why}", k(size), k(limit)));
        }
        let desc = fm.get("description").map(|d| d.chars().count()).unwrap_or(0);
        description_chars += desc;
        if desc == 0 {
            add(HealthKind::Description, Some(name), Some("SKILL.md"), "It has no description, so Claude Code cannot tell when to use it.".into());
        } else if desc > DESCRIPTION_CHARS {
            add(HealthKind::Description, Some(name), Some("SKILL.md"), format!("The description is {desc} characters and is loaded on every turn. Keep one trigger per situation."));
        }
        match fm.get("name") {
            Some(n) if n == name => {}
            Some(n) => add(HealthKind::NameMismatch, Some(name), Some("SKILL.md"), format!("The frontmatter name is `{n}` but the folder is `{name}`; they should match.")),
            None => add(HealthKind::NameMismatch, Some(name), Some("SKILL.md"), "The frontmatter has no name.".into()),
        }
        let dated: Vec<usize> = prose(main).into_iter().filter(|(_, l)| has_date(&without_marks(l))).map(|(n, _)| n).collect();
        if dated.len() >= DATED_LINES {
            add(
                HealthKind::DatedLines,
                Some(name),
                Some("SKILL.md"),
                format!("{} lines carry a date (first at line {}). Dated facts age; they belong in a facts file with the day they were last checked.", dated.len(), dated[0]),
            );
        }
        let junk: Vec<String> = own_skills::skill_files(&skills_dir.join(name)).into_iter().map(|(rel, _)| rel).filter(|r| is_junk(r)).collect();
        if !junk.is_empty() {
            add(HealthKind::Junk, Some(name), None, format!("{} generated or leftover files, safe to delete: {}.", junk.len(), clip(&one_line(&junk.iter().take(3).cloned().collect::<Vec<_>>().join(", ")), 120)));
        }

        for (rel, text) in std::iter::once(("SKILL.md", main)).chain(files.iter().map(|(r, t)| (r.as_str(), t))) {
            let dangling: Vec<String> = named_in_code(text, &prefix).into_iter().filter(|n| !names.contains(n.as_str())).collect();
            if !dangling.is_empty() {
                let list = dangling.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ");
                add(HealthKind::Dangling, Some(name), Some(rel), format!("It points to {list}, which {} exist.", if dangling.len() == 1 { "does not" } else { "do not" }));
            }
            let loose = loose_notes(text);
            if !loose.is_empty() {
                add(HealthKind::LooseNotes, Some(name), Some(rel), format!("{} line(s) may be unfinished notes ({}).", loose.len(), sample(&loose)));
            }
            if rel != "SKILL.md" {
                if text.chars().count() > REFERENCE_CHARS {
                    add(HealthKind::TooLong, Some(name), Some(rel), format!("{rel} is {} characters. If it keeps growing it is a log, not a reference.", k(text.chars().count())));
                }
                let n = repeats(text);
                if n >= REPEATED_LINES {
                    add(HealthKind::RepeatedEntries, Some(name), Some(rel), format!("{n} lines repeat an earlier entry."));
                }
            }
        }
    }

    for (rel, text) in &notes {
        let loose = loose_notes(text);
        if !loose.is_empty() {
            add(HealthKind::LooseNotes, None, Some(rel), format!("{} line(s) may be unfinished notes ({}).", loose.len(), sample(&loose)));
        }
        let n = repeats(text);
        if n >= REPEATED_LINES {
            add(HealthKind::RepeatedEntries, None, Some(rel), format!("{n} of its lines repeat an earlier entry. Keep the open conditions and recent days, and summarize the rest in a line."));
        }
    }

    // Reachability: every skill should be found by following names from the role skill.
    if names.contains(slug) {
        let mentioned: BTreeMap<&str, Vec<&str>> = skills
            .iter()
            .map(|(n, main, files)| {
                let hits = names.iter().copied().filter(|o| *o != n.as_str() && (mentions(main, o) || files.iter().any(|(_, t)| mentions(t, o)))).collect();
                (n.as_str(), hits)
            })
            .collect();
        let mut seen: HashSet<&str> = HashSet::from([slug]);
        let mut queue = VecDeque::from([slug]);
        while let Some(n) = queue.pop_front() {
            for &m in mentioned.get(n).into_iter().flatten() {
                if seen.insert(m) {
                    queue.push_back(m);
                }
            }
        }
        for n in names.iter().copied().filter(|n| !seen.contains(n)) {
            add(HealthKind::Unreachable, Some(n), None, format!("Nothing that `{slug}` points to leads here, so {slug} will not reach for it."));
        }
    }

    // Every task starts with the role skill, so only the others can go unused.
    let month_ago = now - chrono::Duration::days(UNUSED_DAYS);
    for n in names.iter().copied().filter(|n| *n != slug) {
        let last = used.iter().find(|(s, _)| s == n).map_or(counted_since, |(_, at)| *at);
        let made = fs::metadata(skills_dir.join(n)).and_then(|m| m.created().or_else(|_| m.modified())).map(Ts::from);
        if last < month_ago && made.is_ok_and(|t| t < month_ago) {
            add(HealthKind::Unused, Some(n), None, format!("No run has used it in the last {UNUSED_DAYS} days. If none will, retire it (`retire_skill`)."));
        }
    }

    let total = skills.len();
    if description_chars > DESCRIPTIONS_TOTAL_CHARS {
        add(
            HealthKind::DescriptionsHeavy,
            None,
            None,
            format!("The {total} descriptions total {} characters, and every turn loads all of them. Tighten the longest, or fold near-duplicates together.", k(description_chars)),
        );
    }

    let day_ago = now - chrono::Duration::hours(24);
    let mut recent: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for (skill, file, _) in edits.iter().filter(|(_, _, at)| *at >= day_ago) {
        *recent.entry((skill.as_str(), file.as_str())).or_default() += 1;
    }
    for ((skill, file), n) in recent.into_iter().filter(|(_, n)| *n >= CHURN_EDITS) {
        add(HealthKind::Churn, Some(skill), Some(file), format!("{file} was edited {n} times in the last day. Repeated rewrites usually mean the rule is not settled."));
    }

    SkillHealth { findings: f }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A family holding only the role skill `ada`, with `body` under its frontmatter. Returns the
    /// folder, which the caller removes.
    fn family(body: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("omegastar-health-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join("skills/ada")).unwrap();
        fs::create_dir_all(dir.join("workspace")).unwrap();
        fs::write(dir.join("skills/ada/SKILL.md"), format!("---\nname: ada\ndescription: Role skill for Ada.\n---\n\n{body}")).unwrap();
        dir
    }

    fn findings(dir: &Path, kind: HealthKind) -> Vec<HealthFinding> {
        let now = chrono::Utc::now();
        check(&dir.join("skills"), &dir.join("workspace"), "ada", &[], &[], now, now).findings.into_iter().filter(|f| f.kind == kind).collect()
    }

    #[test]
    fn a_skill_no_run_used_for_a_month_is_worth_a_look() {
        let dir = family("Reach for `ada-invoices` and `ada-payroll`.\n");
        for leaf in ["ada-invoices", "ada-payroll"] {
            fs::create_dir_all(dir.join("skills").join(leaf)).unwrap();
            fs::write(dir.join("skills").join(leaf).join("SKILL.md"), format!("---\nname: {leaf}\ndescription: A leaf.\n---\n")).unwrap();
        }
        let made = chrono::Utc::now();
        let day = |n: i64| made + chrono::Duration::days(n);
        let unused = |used: &[(String, Ts)], counted_since: Ts, now: Ts| -> Vec<String> {
            let h = check(&dir.join("skills"), &dir.join("workspace"), "ada", &[], used, counted_since, now);
            h.findings.into_iter().filter(|f| f.kind == HealthKind::Unused).filter_map(|f| f.skill).collect()
        };
        assert!(unused(&[], made, day(20)).is_empty(), "no skill is unused before a month of counting");
        assert_eq!(unused(&[], made, day(40)), ["ada-invoices", "ada-payroll"], "never the role skill");
        assert_eq!(unused(&[("ada-invoices".into(), day(15))], made, day(40)), ["ada-payroll"]);
        assert!(unused(&[], day(-60), day(10)).is_empty(), "a skill gets its own month");
        let f = check(&dir.join("skills"), &dir.join("workspace"), "ada", &[], &[], made, day(40)).findings;
        let detail = &f.iter().find(|f| f.kind == HealthKind::Unused).unwrap().detail;
        assert!(detail.contains("retire_skill") && !detail.contains('\u{2014}'), "{detail}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn marks_are_not_dated_lines() {
        let taught = "## Standing preferences\n\n- Send the weekly summary on Fridays (taught 2026-10-01)\n- Keep replies short (taught 2026-10-01)\n\
            - Copy the finance lead on invoices (taught 2026-10-01)\n- Use the shared calendar (Taught 2026-10-01)\n";
        let dir = family(taught);
        assert!(findings(&dir, HealthKind::DatedLines).is_empty(), "taught marks are not dates that age");
        let _ = fs::remove_dir_all(&dir);

        assert!(!has_date(&without_marks("- Keep replies short (confirmed by Bo 2026-10-03)")));
        assert!(has_date(&without_marks("Updated 2026-10-01 (checked 2026-10-02)")), "a dated fact still counts once its mark is gone");
        let dated = "Updated 2026-10-01 (checked 2026-10-02)\n\nRates 2026-10-01\n\nPrices 2026-10-01\n";
        let dir = family(dated);
        let found = findings(&dir, HealthKind::DatedLines);
        assert_eq!(found.len(), 1);
        assert!(found[0].detail.starts_with("3 lines carry a date"), "{}", found[0].detail);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_names_are_not_loose_notes() {
        for line in ["Keep the list in `todo.md` and read it first.", "see `todo.md`", "Notes live in workspace/todo.md", "Run my-todo-list each morning"] {
            assert_eq!(loose_note(line), None, "{line}");
        }
        for line in ["TODO: confirm the rate.", "fix this TODO."] {
            assert!(loose_note(line).is_some(), "{line}");
        }
        let line = "| Supplier rates | `rates.md` keeps the full table for every region we ship to | TODO: confirm the rate. |";
        let at = loose_note(line).unwrap();
        assert_eq!(line.chars().skip(at).take(4).collect::<String>(), "TODO", "the code span keeps positions");
        assert!(excerpt(line, at).contains("TODO: confirm the rate."), "{}", excerpt(line, at));
    }

    #[test]
    fn a_doubt_written_into_a_rule_is_a_loose_note() {
        for line in ["The newer rule may cover them, so until Pat settles it, leave them unticked.", "Local or served: undecided.", "Who signs: to be decided."] {
            assert!(loose_note(line).is_some(), "{line}");
        }
        for line in ["Nothing goes out until Pat approves the exact text.", "The gain is booked when the payable is settled.", "The bank settles items overnight."] {
            assert_eq!(loose_note(line), None, "{line}");
        }
    }

    #[test]
    fn company_page_findings() {
        let short = "# Company page\n\n## Who to ask\n\n- Invoices: the finance lead.\n";
        assert!(page(short).is_empty());
        let long = format!("# Company page\n\n{}", "- One line about how things work here.\n".repeat(180));
        let found = page(&long);
        assert_eq!(found.iter().map(|f| f.kind).collect::<Vec<_>>(), vec![HealthKind::TooLong]);
        assert!(found[0].detail.starts_with("The company page is 7.0k characters"), "{}", found[0].detail);
        assert_eq!(found[0].file.as_deref(), Some("company page"));
        assert_eq!(found[0].skill, None);
        let loose = page("## Right now\n\n- 2026-10-01 New supplier. TODO confirm the terms.\n");
        assert_eq!(loose.iter().map(|f| f.kind).collect::<Vec<_>>(), vec![HealthKind::LooseNotes]);
    }
}
