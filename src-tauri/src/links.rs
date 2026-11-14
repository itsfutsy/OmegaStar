//! `omegastar://` links back into the app: "Chat with Hugo about this" in a PR description or a report opens Hugo's
//! conversation, the thread it came from when there is one, or a new conversation with a first line written.
//!
//! Each build channel has its own scheme, so a test or developer build never takes a release's links. A link is
//! outside content: it can only open a conversation and write a draft you send yourself, never send, approve or
//! change anything. What it carries is checked, trimmed and capped here.

use crate::model::{BuildChannel, Employee};
use std::sync::Mutex;
use tauri::Url;

/// The longest link read at all.
const MAX_LINK: usize = 4096;
/// The longest task text kept from a link, in characters.
const MAX_TASK: usize = 200;
/// The longest pull request link kept.
const MAX_PR: usize = 300;
/// The longest machine name kept from `from`.
const MAX_NODE: usize = 64;

/// This build's link scheme: `omegastar` for a release, `omegastar-dogfood` and `omegastar-dev` for the others.
/// Mirrors the schemes in tauri.conf.json (release) and scripts/dogfood-build.sh (dogfood).
pub fn scheme(channel: BuildChannel) -> &'static str {
    match channel {
        BuildChannel::Release => "omegastar",
        BuildChannel::Dogfood => "omegastar-dogfood",
        BuildChannel::Dev => "omegastar-dev",
    }
}

/// A link as read: `omegastar://chat?employee=<uuid>&slug=<slug>&name=<name>&run=<runId>&thread=<rootId>&task=<text>
/// &pr=<url>&repo=<owner/repo>&from=<node>`. Every part is optional.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppLink {
    pub employee: Option<String>,
    pub slug: Option<String>,
    pub name: Option<String>,
    pub run: Option<String>,
    pub thread: Option<String>,
    pub task: Option<String>,
    /// Only an `https://` link.
    pub pr: Option<String>,
    /// `owner/repo`.
    pub repo: Option<String>,
    /// The machine the link was made for. Kept for the mesh, which will send a link to the right machine; nothing
    /// reads it yet.
    pub from: Option<String>,
}

/// Where a link opens: a thread that exists, or a new conversation with the employee and an optional first line.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "type")]
pub enum LinkTarget {
    Thread { employee_id: String, root_id: String },
    NewConversation { employee_id: String, draft: Option<String> },
}

/// The newest link opened, kept until the window reads it with `take_app_link`: a link that starts the app arrives
/// before the window can listen.
#[derive(Default)]
pub struct PendingLink(Mutex<Option<String>>);

impl PendingLink {
    pub fn put(&self, raw: String) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(raw);
    }

    pub fn take(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

/// The last of `args` that is a link of this build's scheme. Links reach the app as launch arguments on Windows and
/// Linux, mixed in with flags such as `--background`.
pub fn ours(args: impl IntoIterator<Item = String>, channel: BuildChannel) -> Option<String> {
    let prefix = format!("{}:", scheme(channel));
    args.into_iter().filter(|a| a.trim().get(..prefix.len()).is_some_and(|p| p.eq_ignore_ascii_case(&prefix))).last()
}

/// Reads a link of this build's scheme. Anything else (another scheme, another action, too long) is refused with
/// the reason, which goes to the log. Parts it does not know are ignored.
pub fn parse(raw: &str, channel: BuildChannel) -> Result<AppLink, String> {
    let raw = raw.trim();
    if raw.len() > MAX_LINK {
        return Err("the link is too long".into());
    }
    let url = Url::parse(raw).map_err(|_| "not a link".to_string())?;
    if !url.scheme().eq_ignore_ascii_case(scheme(channel)) {
        return Err(format!("a {} link, not this build's {}", url.scheme(), scheme(channel)));
    }
    // `omegastar://chat?...`, or `omegastar:chat?...` and `omegastar:///chat?...` as some apps write them.
    let action = url.host_str().filter(|h| !h.is_empty()).unwrap_or_else(|| url.path().trim_matches('/'));
    if !action.eq_ignore_ascii_case("chat") {
        return Err(format!("'{action}' is not something a link can do"));
    }
    let mut link = AppLink::default();
    for (k, v) in url.query_pairs() {
        let v = clean(&v);
        if v.is_empty() {
            continue;
        }
        match k.as_ref() {
            "employee" => link.employee = id_like(&v),
            "slug" => link.slug = id_like(&v),
            "name" => link.name = Some(clip(&v, 80)),
            "run" => link.run = id_like(&v),
            "thread" => link.thread = id_like(&v),
            "task" => link.task = Some(clip(&v, MAX_TASK)),
            "pr" => link.pr = (v.starts_with("https://") && v.len() <= MAX_PR && Url::parse(&v).is_ok()).then_some(v),
            "repo" => link.repo = repo_like(&v),
            "from" => link.from = (v.len() <= MAX_NODE && v.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))).then_some(v),
            _ => {}
        }
    }
    Ok(link)
}

/// The employee a link names, tried in order: their id, then their slug, then their name (only when one employee
/// has it). Case does not matter for slug or name.
pub fn employee<'a>(link: &AppLink, employees: &'a [Employee]) -> Option<&'a Employee> {
    if let Some(id) = &link.employee {
        if let Some(e) = employees.iter().find(|e| e.id == *id) {
            return Some(e);
        }
    }
    if let Some(slug) = &link.slug {
        if let Some(e) = employees.iter().find(|e| e.slug.eq_ignore_ascii_case(slug)) {
            return Some(e);
        }
    }
    let name = link.name.as_deref()?;
    let mut named = employees.iter().filter(|e| e.name.eq_ignore_ascii_case(name));
    match (named.next(), named.next()) {
        (Some(e), None) => Some(e),
        _ => None,
    }
}

/// Where a link opens. `thread_of` gives the thread a run id is in, as (root run id, employee id), when it exists.
/// The thread opens when it exists and is the named employee's (or no one was named); otherwise a new conversation
/// with the named employee, with "About <task> (<pr>)" written for you to send or change.
pub fn route(link: &AppLink, employees: &[Employee], thread_of: impl Fn(&str) -> Option<(String, String)>) -> Result<LinkTarget, String> {
    let named = employee(link, employees);
    let thread = [&link.thread, &link.run].into_iter().flatten().find_map(|id| thread_of(id));
    if let Some((root_id, employee_id)) = thread {
        let theirs = employees.iter().any(|e| e.id == employee_id);
        if theirs && named.is_none_or(|e| e.id == employee_id) {
            return Ok(LinkTarget::Thread { employee_id, root_id });
        }
    }
    let e = named.ok_or_else(|| "That link is for someone who doesn't work here.".to_string())?;
    Ok(LinkTarget::NewConversation { employee_id: e.id.clone(), draft: draft(link) })
}

/// The first line a link writes in a new conversation: "About <task> (<pr>)", with the repo when there is no pull
/// request. `None` when it carries none of them.
fn draft(link: &AppLink) -> Option<String> {
    let task = link.task.as_deref();
    let where_ = link.pr.as_deref().or(link.repo.as_deref());
    match (task, where_) {
        (Some(t), Some(w)) => Some(format!("About {t} ({w})")),
        (Some(t), None) => Some(format!("About {t}")),
        (None, Some(w)) => Some(format!("About {w}")),
        (None, None) => None,
    }
}

/// The link back to an employee's thread, for `app_link`: their id and slug, so it still finds them after a rename,
/// the thread, and what it is about.
pub fn build(channel: BuildChannel, e: &Employee, thread: Option<&str>, task: Option<&str>, pr: Option<&str>) -> String {
    let mut url = Url::parse(&format!("{}://chat", scheme(channel))).expect("a fixed link parses");
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("employee", &e.id).append_pair("slug", &e.slug);
        if let Some(t) = thread {
            q.append_pair("thread", t);
        }
        if let Some(t) = task.map(clean).filter(|t| !t.is_empty()) {
            q.append_pair("task", &clip(&t, MAX_TASK));
        }
        if let Some(p) = pr.map(clean).filter(|p| p.starts_with("https://")) {
            q.append_pair("pr", &p);
        }
    }
    url.to_string()
}

/// One line, without control characters, trimmed.
fn clean(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect::<String>().trim_end().to_string()
}

/// An id or slug: letters, digits, `-` and `_`, at most 64.
fn id_like(s: &str) -> Option<String> {
    (s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')).then(|| s.to_string())
}

/// `owner/repo`, as GitHub allows them.
fn repo_like(s: &str) -> Option<String> {
    let (owner, name) = s.split_once('/')?;
    let ok = |p: &str| !p.is_empty() && p.len() <= 100 && p.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    (ok(owner) && ok(name)).then(|| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::employee_named;

    fn hugo() -> Employee {
        let mut e = employee_named("Hugo Park");
        e.id = "6f1c2a4e-0d1b-4c55-9a43-2f0b7d1e9c10".into();
        e.slug = "hugo".into();
        e
    }

    #[test]
    fn each_build_channel_has_its_own_scheme() {
        assert_eq!([BuildChannel::Release, BuildChannel::Dogfood, BuildChannel::Dev].map(scheme), ["omegastar", "omegastar-dogfood", "omegastar-dev"]);
    }

    #[test]
    fn the_release_and_dogfood_schemes_match_what_their_builds_register() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["plugins"]["deep-link"]["desktop"]["schemes"], serde_json::json!([scheme(BuildChannel::Release)]));
        let dogfood = include_str!("../../scripts/dogfood-build.sh");
        assert!(dogfood.contains(&format!("\\\"schemes\\\":[\\\"{}\\\"]", scheme(BuildChannel::Dogfood))), "the dogfood build registers its own scheme");
    }

    #[test]
    fn only_this_builds_links_are_picked_from_what_the_app_was_opened_with() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let opened = args(&["/Applications/Omega Star.app", "--background", "omegastar-dev://chat?slug=a", "OmegaStar://chat?slug=b"]);
        assert_eq!(ours(opened.clone(), BuildChannel::Release).as_deref(), Some("OmegaStar://chat?slug=b"));
        assert_eq!(ours(opened.clone(), BuildChannel::Dev).as_deref(), Some("omegastar-dev://chat?slug=a"));
        assert_eq!(ours(opened, BuildChannel::Dogfood), None);
        let pending = PendingLink::default();
        pending.put("omegastar://chat?slug=a".into());
        pending.put("omegastar://chat?slug=b".into());
        assert_eq!((pending.take().as_deref(), pending.take()), (Some("omegastar://chat?slug=b"), None), "the newest link is read once");
    }

    #[test]
    fn a_link_is_read_with_every_part_it_knows() {
        let raw = "omegastar://chat?employee=6f1c2a4e-0d1b-4c55-9a43-2f0b7d1e9c10&slug=hugo&name=Hugo%20Park&run=r1&thread=t1\
&task=Fix+the+login+bug&pr=https%3A%2F%2Fgithub.com%2Fo%2Fr%2Fpull%2F12&repo=o/r&from=jacks-mac&colour=blue";
        let link = parse(raw, BuildChannel::Release).unwrap();
        assert_eq!(
            link,
            AppLink {
                employee: Some("6f1c2a4e-0d1b-4c55-9a43-2f0b7d1e9c10".into()),
                slug: Some("hugo".into()),
                name: Some("Hugo Park".into()),
                run: Some("r1".into()),
                thread: Some("t1".into()),
                task: Some("Fix the login bug".into()),
                pr: Some("https://github.com/o/r/pull/12".into()),
                repo: Some("o/r".into()),
                from: Some("jacks-mac".into()),
            }
        );
        assert_eq!(parse("omegastar:chat?slug=hugo", BuildChannel::Release).unwrap().slug.as_deref(), Some("hugo"));
        assert_eq!(parse("OmegaStar://Chat/?slug=hugo", BuildChannel::Release).unwrap().slug.as_deref(), Some("hugo"));
    }

    #[test]
    fn a_link_for_another_build_or_action_is_refused() {
        assert!(parse("omegastar://chat?slug=hugo", BuildChannel::Dev).unwrap_err().contains("not this build's omegastar-dev"));
        assert!(parse("omegastar-dogfood://chat?slug=hugo", BuildChannel::Release).is_err());
        assert!(parse("https://example.com/chat?slug=hugo", BuildChannel::Release).is_err());
        assert_eq!(parse("omegastar://approve?run=r1", BuildChannel::Release).unwrap_err(), "'approve' is not something a link can do");
        assert_eq!(parse("omegastar://send?task=x", BuildChannel::Release).unwrap_err(), "'send' is not something a link can do");
        assert_eq!(parse(&format!("omegastar://chat?task={}", "a".repeat(5000)), BuildChannel::Release).unwrap_err(), "the link is too long");
    }

    #[test]
    fn what_a_link_carries_is_trimmed_and_anything_odd_is_dropped() {
        let raw = format!(
            "omegastar://chat?slug=hu%20go&run=..%2Fx&task=Line%0Aone%09{}&pr=javascript%3Aalert(1)&repo=a/b/c&from=jack%27s%20mac",
            "x".repeat(400)
        );
        let link = parse(&raw, BuildChannel::Release).unwrap();
        assert_eq!((link.slug, link.run, link.pr, link.repo, link.from), (None, None, None, None, None));
        let task = link.task.unwrap();
        assert!(task.starts_with("Line one x") && task.chars().count() <= MAX_TASK, "{task}");
        assert_eq!(parse("omegastar://chat?pr=http%3A%2F%2Fexample.com", BuildChannel::Release).unwrap().pr, None, "only https");
    }

    #[test]
    fn the_employee_is_found_by_id_then_slug_then_a_name_only_one_has() {
        let (h, mut other) = (hugo(), employee_named("Ada"));
        other.slug = "ada".into();
        let team = [other.clone(), h.clone()];
        let find = |l: AppLink| employee(&l, &team).map(|e| e.name.clone());
        assert_eq!(find(AppLink { employee: Some(h.id.clone()), slug: Some("ada".into()), ..Default::default() }).as_deref(), Some("Hugo Park"));
        assert_eq!(find(AppLink { employee: Some("gone".into()), slug: Some("HUGO".into()), ..Default::default() }).as_deref(), Some("Hugo Park"), "a stale id falls back to the slug");
        assert_eq!(find(AppLink { slug: Some("gone".into()), name: Some("hugo park".into()), ..Default::default() }).as_deref(), Some("Hugo Park"));
        let mut twin = employee_named("Hugo Park");
        twin.slug = "hugo-2".into();
        let twins = [h.clone(), twin];
        assert_eq!(employee(&AppLink { name: Some("Hugo Park".into()), ..Default::default() }, &twins), None, "a name two people have finds no one");
    }

    #[test]
    fn a_link_opens_its_thread_or_a_new_conversation_with_a_first_line() {
        let (h, ada) = (hugo(), employee_named("Ada"));
        let team = [h.clone(), ada.clone()];
        let threads = |id: &str| match id {
            "reply-1" | "root-1" => Some(("root-1".to_string(), h.id.clone())),
            "ada-root" => Some(("ada-root".to_string(), ada.id.clone())),
            _ => None,
        };
        let link = |thread: Option<&str>| AppLink { slug: Some("hugo".into()), thread: thread.map(String::from), task: Some("the login bug".into()), pr: Some("https://github.com/o/r/pull/12".into()), ..Default::default() };
        assert_eq!(route(&link(Some("reply-1")), &team, threads).unwrap(), LinkTarget::Thread { employee_id: h.id.clone(), root_id: "root-1".into() });
        let fresh = LinkTarget::NewConversation { employee_id: h.id.clone(), draft: Some("About the login bug (https://github.com/o/r/pull/12)".into()) };
        assert_eq!(route(&link(Some("deleted")), &team, threads).unwrap(), fresh, "a thread that's gone starts a new conversation");
        assert_eq!(route(&link(Some("ada-root")), &team, threads).unwrap(), fresh, "another employee's thread is not opened from Hugo's link");
        let by_run = AppLink { run: Some("ada-root".into()), ..Default::default() };
        assert_eq!(route(&by_run, &team, threads).unwrap(), LinkTarget::Thread { employee_id: ada.id.clone(), root_id: "ada-root".into() }, "a run alone finds its employee");
        let plain = AppLink { slug: Some("hugo".into()), repo: Some("o/r".into()), ..Default::default() };
        assert_eq!(route(&plain, &team, threads).unwrap(), LinkTarget::NewConversation { employee_id: h.id.clone(), draft: Some("About o/r".into()) });
        assert_eq!(route(&AppLink { slug: Some("nobody".into()), ..Default::default() }, &team, threads).unwrap_err(), "That link is for someone who doesn't work here.");
    }

    #[test]
    fn the_window_reads_where_a_link_opens_in_its_own_words() {
        let t = serde_json::to_value(LinkTarget::Thread { employee_id: "e1".into(), root_id: "r1".into() }).unwrap();
        assert_eq!(t, serde_json::json!({ "type": "thread", "employeeId": "e1", "rootId": "r1" }));
        let n = serde_json::to_value(LinkTarget::NewConversation { employee_id: "e1".into(), draft: None }).unwrap();
        assert_eq!(n, serde_json::json!({ "type": "newConversation", "employeeId": "e1", "draft": null }));
    }

    #[test]
    fn a_built_link_reads_back_to_the_same_thread() {
        let h = hugo();
        let raw = build(BuildChannel::Dogfood, &h, Some("root-1"), Some("Fix the login & signup\nbug"), Some("https://github.com/o/r/pull/12"));
        assert!(raw.starts_with("omegastar-dogfood://chat?employee="), "{raw}");
        let link = parse(&raw, BuildChannel::Dogfood).unwrap();
        assert_eq!((link.employee.as_deref(), link.slug.as_deref(), link.thread.as_deref()), (Some(h.id.as_str()), Some("hugo"), Some("root-1")));
        assert_eq!((link.task.as_deref(), link.pr.as_deref()), (Some("Fix the login & signup bug"), Some("https://github.com/o/r/pull/12")));
        assert_eq!(build(BuildChannel::Release, &h, None, None, Some("not a link")), format!("omegastar://chat?employee={}&slug=hugo", h.id));
    }
}
