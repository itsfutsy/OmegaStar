//! An employee's own skills: editing, history, importing, organizing and tidying.

use super::*;

impl Office {
    fn employee(&self, id: &str) -> Result<Employee> {
        self.db().employee(id)?.ok_or_else(|| AppError::not_found("employee not found"))
    }

    pub fn employee_skills(&self, employee_id: &str) -> Result<Vec<OwnSkill>> {
        let e = self.employee(employee_id)?;
        Ok(own_skills::own_skills(&self.skills_dir(&e.id), &e.slug))
    }

    /// What can be noticed about the employee's skills and notes without a model (see `health`).
    pub fn skill_health(&self, employee_id: &str) -> Result<SkillHealth> {
        let e = self.employee(employee_id)?;
        let at = now();
        let (edits, checks) = {
            let db = self.db();
            (db.edits_since(&e.id, at - chrono::Duration::hours(24))?, db.checks(&e.id, Some(CheckState::Open))?)
        };
        let mut h = health::check(&self.skills_dir(&e.id), &self.employee_dir(&e.id).join("workspace"), &e.slug, &edits, at);
        h.findings.extend(health::overdue_checks(&checks, at));
        h.findings.sort_by(|a, b| (a.kind, &a.skill, &a.file).cmp(&(b.kind, &b.skill, &b.file)));
        Ok(h)
    }

    pub fn read_skill_file(&self, employee_id: &str, skill: &str, file: &str) -> Result<String> {
        let e = self.employee(employee_id)?;
        let path = own_skills::recorded_path(&self.employee_dir(&e.id), skill, file)?;
        fs::read_to_string(&path).map_err(|_| AppError::not_found(format!("{skill}/{file} not found")))
    }

    fn record_edit(&self, employee_id: &str, skill: &str, file: &str, text: Option<&str>) -> Result<Option<SkillRevision>> {
        record(&self.db(), &self.employee_dir(employee_id), employee_id, skill, file, text)
    }

    pub fn save_skill_file(&self, employee_id: &str, skill: &str, file: &str, text: &str) -> Result<Option<SkillRevision>> {
        let e = self.employee(employee_id)?;
        self.record_edit(&e.id, skill, file, Some(text))
    }

    pub fn skill_history(&self, employee_id: &str) -> Result<Vec<SkillRevision>> {
        self.db().revisions(employee_id)
    }

    pub fn revert_revision(&self, revision_id: &str) -> Result<SkillRevision> {
        let (employee_id, rev) = self.db().revision(revision_id)?.ok_or_else(|| AppError::not_found("revision not found"))?;
        if !safe_to_undo(&self.employee_dir(&employee_id), &rev)? {
            return Err(AppError::conflict("This changed again since then, so undoing it would lose the newer change. Edit it instead."));
        }
        self.record_edit(&employee_id, &rev.change.skill, &rev.change.file, rev.before.as_deref())?
            .ok_or_else(|| AppError::conflict("the file already matches that revision"))
    }

    pub fn promote_skill(&self, employee_id: &str, skill: &str) -> Result<SkillInfo> {
        let e = self.employee(employee_id)?;
        let catalog = self.skill_catalog();
        own_skills::promote(&self.skills_dir(&e.id), skill, &catalog::config_dir(), &catalog)
    }

    /// The page every employee reads at the start of every run; empty until it exists.
    pub fn company_page(&self) -> Result<CompanyPage> {
        let text = fs::read_to_string(own_skills::company_page_path(&self.0.data_dir.join("employees"))).unwrap_or_default();
        Ok(CompanyPage { findings: health::page(&text), text })
    }

    /// You edit the company page yourself. A plain write with no revision: you are its author.
    pub fn save_company_page(&self, text: &str) -> Result<CompanyPage> {
        fs::write(own_skills::company_page_path(&self.0.data_dir.join("employees")), text)?;
        self.company_page()
    }

    // ------------------------------------------------------------------ importing and organizing

    /// Catalog skills behind `ids`, with the folder each is copied from. User and synced skills only.
    pub(super) fn resolve_imports(catalog: &[SkillInfo], ids: &[String]) -> Result<Vec<(SkillInfo, PathBuf)>> {
        let config = catalog::config_dir();
        let mut out: Vec<(SkillInfo, PathBuf)> = vec![];
        for id in ids {
            let s = catalog.iter().find(|s| &s.id == id).ok_or_else(|| AppError::not_found(format!("no skill '{id}' in your Claude Code skills")))?;
            if s.source == SkillSource::Plugin {
                return Err(AppError::invalid(format!("'{}' is a plugin: grant it instead of importing it", s.name)));
            }
            let dir = catalog::source_dir(&config, s).ok_or_else(|| AppError::not_found(format!("could not find the folder of '{}'", s.name)))?;
            if !out.iter().any(|(x, _)| x.name == s.name) {
                out.push((s.clone(), dir));
            }
        }
        Ok(out)
    }

    /// Copies the skills into `imports/` and queues the Organize run (or adds them to a queued one).
    pub(super) fn stage_and_organize(&self, e: &Employee, picked: &[(SkillInfo, PathBuf)]) -> Result<Run> {
        let imports = own_skills::imports_dir(&self.employee_dir(&e.id));
        if let Some((s, _)) = picked.iter().find(|(s, _)| imports.join(&s.name).exists()) {
            return Err(AppError::conflict(format!("'{}' is already waiting to be organized", s.name)));
        }
        let mut imported = vec![];
        for (s, dir) in picked {
            own_skills::stage_import(&imports, dir, &s.name)?;
            imported.push(ImportedSkill { id: s.id.clone(), name: s.name.clone() });
        }
        self.enqueue_organize(&e.id, imported)
    }

    fn organize_prompt(imported: &[ImportedSkill]) -> String {
        let names: Vec<String> = imported.iter().map(|i| format!("`{}`", i.name)).collect();
        format!("Organize the {} skills waiting in imports/ into your skill family: {}.", imported.len(), names.join(", "))
    }

    fn enqueue_organize(&self, employee_id: &str, imported: Vec<ImportedSkill>) -> Result<Run> {
        let queued = {
            let db = self.db();
            let mut queued = db.runs_in_state(RunState::Queued)?.into_iter().find(|r| r.employee_id == employee_id && matches!(r.kind, RunKind::Organize { .. }));
            if let Some(r) = queued.as_mut() {
                if let RunKind::Organize { imported: have, .. } = &mut r.kind {
                    for i in &imported {
                        if !have.iter().any(|h| h.name == i.name) {
                            have.push(i.clone());
                        }
                    }
                    r.prompt = Self::organize_prompt(have);
                }
                db.save_run(r)?;
            }
            queued
        };
        if let Some(r) = queued {
            self.sink().run_updated(&r);
            return Ok(r);
        }
        let prompt = Self::organize_prompt(&imported);
        let run = Run::new(employee_id, RunKind::Organize { imported, settled: false }, prompt, now());
        let session = run.id.clone();
        self.enqueue(run, session)
    }

    /// Copies catalog skills into the employee's `imports/`, drops them from the grant, and queues an
    /// Organize run. Your originals are never touched.
    pub fn import_skills(&self, employee_id: &str, ids: &[String]) -> Result<Run> {
        let mut e = self.employee(employee_id)?;
        if ids.is_empty() {
            return Err(AppError::invalid("pick at least one skill to import"));
        }
        let picked = Self::resolve_imports(&self.skill_catalog(), ids)?;
        if e.skills.keys.iter().any(|k| ids.contains(k)) {
            e.skills.keys.retain(|k| !ids.contains(k));
            e = self.update_employee(e)?;
        }
        self.stage_and_organize(&e, &picked)
    }

    pub fn staged_imports(&self, employee_id: &str) -> Result<Vec<StagedImport>> {
        let e = self.employee(employee_id)?;
        Ok(own_skills::staged_imports(&own_skills::imports_dir(&self.employee_dir(&e.id))))
    }

    /// Organizes whatever is still waiting in `imports/`.
    pub fn organize(&self, employee_id: &str) -> Result<Run> {
        let e = self.employee(employee_id)?;
        let staged = self.staged_imports(&e.id)?;
        if staged.is_empty() {
            return Err(AppError::invalid("nothing is waiting to be organized"));
        }
        let catalog = self.skill_catalog();
        let id_of = |name: &str| {
            catalog.iter().find(|c| c.name == name && c.source != SkillSource::Plugin).map(|c| c.id.clone()).unwrap_or_else(|| format!("user:{name}"))
        };
        let imported = staged.into_iter().map(|s| ImportedSkill { id: id_of(&s.name), name: s.name }).collect();
        self.enqueue_organize(&e.id, imported)
    }

    /// Reverts every skill change a run made, back to the state before it. For an Organize run,
    /// the imports it consumed wait in `imports/` again.
    pub fn revert_run(&self, run_id: &str) -> Result<u32> {
        let run = self.run(run_id)?;
        if !run.state.is_finished() {
            return Err(AppError::conflict("wait for the run to finish before reverting it"));
        }
        let revs: Vec<SkillRevision> = self.db().revisions(&run.employee_id)?.into_iter().filter(|r| r.run_id.as_deref() == Some(run_id)).collect();
        let n = undo(&self.db(), &self.employee_dir(&run.employee_id), &run.employee_id, &revs)?;
        let skills_dir = self.skills_dir(&run.employee_id);
        // A folder the run created can still hold files no revision tracks (binaries, big files).
        let created: HashSet<&str> =
            revs.iter().filter(|r| r.change.change == ChangeKind::Added && r.change.file == "SKILL.md").map(|r| r.change.skill.as_str()).collect();
        for skill in created {
            let dir = skills_dir.join(skill);
            if dir.is_dir() && !dir.join("SKILL.md").exists() {
                let dest = self.run_dir(run_id).join("reverted").join(skill);
                fs::create_dir_all(dest.parent().unwrap())?;
                fs::rename(&dir, &dest)?;
            }
        }
        if let RunKind::Organize { imported, .. } = &run.kind {
            let names: Vec<String> = imported.iter().map(|i| i.name.clone()).collect();
            let imports = own_skills::imports_dir(&self.employee_dir(&run.employee_id));
            own_skills::restore_imports(&self.run_dir(run_id).join("imports"), &imports, &names, &skills_dir)?;
        }
        self.sink().org_changed();
        Ok(n)
    }

    fn organize_run(&self, run_id: &str) -> Result<(Run, Vec<ImportedSkill>)> {
        let run = self.run(run_id)?;
        let RunKind::Organize { imported, .. } = &run.kind else {
            return Err(AppError::invalid("that run did not import skills"));
        };
        let imported = imported.clone();
        Ok((run, imported))
    }

    /// Your originals behind an Organize run, with what removing each could break.
    pub fn import_originals(&self, run_id: &str) -> Result<Vec<OriginalSkill>> {
        let (run, imported) = self.organize_run(run_id)?;
        let catalog = self.skill_catalog();
        let config = catalog::config_dir();
        let employees = self.db().employees()?;
        let names: HashSet<&str> = imported.iter().map(|i| i.name.as_str()).collect();
        let others: Vec<(String, String)> = catalog
            .iter()
            .filter(|c| c.source == SkillSource::User && !names.contains(c.name.as_str()))
            .filter_map(|c| Some((c.name.clone(), fs::read_to_string(config.join("skills").join(&c.name).join("SKILL.md")).ok()?)))
            .collect();
        Ok(imported
            .iter()
            .map(|i| {
                let found = catalog.iter().find(|c| c.id == i.id);
                let source = found.map(|c| c.source).unwrap_or(if i.id.starts_with("synced:") { SkillSource::Synced } else { SkillSource::User });
                let present = found.and_then(|c| catalog::source_dir(&config, c)).is_some();
                let mut notes: Vec<String> =
                    employees.iter().filter(|x| x.id != run.employee_id && x.skills.keys.contains(&i.id)).map(|x| format!("Also granted to {}", x.name)).collect();
                let callers: Vec<String> = others.iter().filter(|(_, text)| mentions(text, &i.name)).map(|(n, _)| format!("`{n}`")).take(3).collect();
                if !callers.is_empty() {
                    notes.push(format!("Called by {}", callers.join(", ")));
                }
                if source == SkillSource::Synced {
                    notes.push("Synced from claude.ai: remove it there, or it comes back".into());
                }
                OriginalSkill { id: i.id.clone(), name: i.name.clone(), source, present, removable: present && source == SkillSource::User, notes }
            })
            .collect())
    }

    /// Moves the chosen originals from your Claude Code skills to the Recycle Bin, and records that
    /// you answered the offer.
    pub fn remove_originals(&self, run_id: &str, names: &[String]) -> Result<Vec<String>> {
        let offered = self.import_originals(run_id)?;
        let config = catalog::config_dir();
        let mut removed = vec![];
        let mut failed = vec![];
        for o in offered.iter().filter(|o| o.removable && names.contains(&o.name)) {
            match trash::delete(config.join("skills").join(&o.name)) {
                Ok(()) => removed.push(o.name.clone()),
                Err(e) => failed.push(format!("{}: {e}", o.name)),
            }
        }
        self.settle_originals(run_id)?;
        if !failed.is_empty() {
            return Err(AppError::internal(format!("could not remove {}", failed.join("; "))));
        }
        Ok(removed)
    }

    /// Records that you answered the "remove your originals?" offer.
    pub fn settle_originals(&self, run_id: &str) -> Result<Run> {
        let (mut run, _) = self.organize_run(run_id)?;
        if let RunKind::Organize { settled, .. } = &mut run.kind {
            *settled = true;
        }
        self.db().save_run(&run)?;
        self.sink().run_updated(&run);
        Ok(run)
    }

    pub fn move_skill_tool(&self, caller: &Employee, from: &str, to: &str) -> Result<Value> {
        let dir = self.employee_dir(&caller.id);
        let dest = own_skills::move_skill(&own_skills::skills_dir(&dir), &own_skills::imports_dir(&dir), &caller.slug, from, to)?;
        let files: Vec<String> = own_skills::staged_imports(dest.parent().unwrap()).into_iter().find(|s| s.name == to).map(|s| s.files).unwrap_or_default();
        Ok(json!({ "moved": from, "to": to, "files": files, "note": "Frontmatter name set. Update every reference to the old name and old paths." }))
    }

    pub fn retire_skill_tool(&self, caller_run: &str, caller: &Employee, name: &str) -> Result<Value> {
        let dir = self.employee_dir(&caller.id);
        let archive = self.run_dir(caller_run).join("retired");
        own_skills::retire_skill(&own_skills::skills_dir(&dir), &own_skills::imports_dir(&dir), &caller.slug, name, &archive)?;
        Ok(json!({ "retired": name, "note": "Kept in the run's archive; the change is recorded and can be reverted." }))
    }

    // ------------------------------------------------------------------ tidy

    /// What a Tidy run works from; none for any other run.
    pub(super) fn tidy_brief(&self, run: &Run, e: &Employee) -> Result<Option<claude::TidyBrief>> {
        let RunKind::Tidy { plan, .. } = &run.kind else { return Ok(None) };
        // An overdue check is work of its own, not something for the tidy to do.
        let findings = self.skill_health(&e.id)?.findings.into_iter().filter(|f| f.kind != HealthKind::OverdueCheck).collect();
        let db = self.db();
        // What you taught since the last tidy that got as far as confirming it.
        let since = db.tidy_runs(&e.id)?.into_iter().find(|r| r.id != run.id && r.state == RunState::Succeeded).map(|r| r.created_at);
        let taught = db.taught_since(&e.id, since, tidy::TAUGHT_LIMIT)?;
        Ok(Some(claude::TidyBrief { findings, taught, plan: plan.clone() }))
    }

    /// Raises a flag on a live run without a desktop notification.
    pub(super) fn flag(&self, run_id: &str, message: String) {
        if let Some(a) = lock(&self.0.active).get_mut(run_id) {
            a.needs_you.push(message);
        }
    }

    /// A finished tidy goes to your Inbox when it proposes changes, and for an employee's first few
    /// tidies, so you see what one does before you stop looking.
    pub(super) fn review_tidy(&self, run: &mut Run) {
        let (name, earlier) = {
            let db = self.db();
            let name = db.employee(&run.employee_id).ok().flatten().map(|e| e.name).unwrap_or_default();
            let earlier = db
                .tidy_runs(&run.employee_id)
                .map(|v| v.iter().filter(|r| r.id != run.id && r.created_at < run.created_at && r.state == RunState::Succeeded).count())
                .unwrap_or(usize::MAX);
            (name, earlier)
        };
        if run.proposed.is_some() {
            run.needs_you.push("Proposes bigger changes that need your OK.".into());
            self.sink().notify(&format!("{name} proposes tidy changes"), "Open the report in your Inbox to approve them.");
        }
        if earlier < tidy::REVIEWED_FIRST {
            run.needs_you.push(format!("One of {name}'s first tidies: open it to see what changed. Every change can be reverted."));
        }
    }

    /// Queues a tidy for the employee, unless one is already waiting or running.
    fn enqueue_tidy(&self, employee_id: &str, reason: TidyReason, why: &str, plan: Option<String>) -> Result<Run> {
        let signature = tidy::signature(&self.skill_health(employee_id)?.findings);
        {
            let db = self.db();
            let e = db.employee(employee_id)?.ok_or_else(|| AppError::not_found("employee not found"))?;
            if db.active_runs_of(employee_id)?.iter().any(|r| matches!(r.kind, RunKind::Tidy { .. })) {
                return Err(AppError::conflict(format!("{} is already tidying", e.name)));
            }
        }
        let prompt = match reason {
            TidyReason::Approved => "Make the changes you proposed and I approved.".to_string(),
            _ => format!("Tidy your skills and notes: {}.", why.trim_end_matches('.')),
        };
        let run = Run::new(employee_id, RunKind::Tidy { reason, signature, plan }, prompt, now());
        let session = run.id.clone();
        self.enqueue(run, session)
    }

    /// You asked for a tidy.
    pub fn tidy(&self, employee_id: &str) -> Result<Run> {
        self.enqueue_tidy(employee_id, TidyReason::Manual, "you asked for one", None)
    }

    /// You approved the bigger changes a tidy proposed: a second run makes exactly those.
    pub fn approve_tidy(&self, run_id: &str) -> Result<Run> {
        let (run, plan) = {
            let db = self.db();
            let run = db.run(run_id)?.ok_or_else(|| AppError::not_found("run not found"))?.run;
            if !matches!(run.kind, RunKind::Tidy { .. }) {
                return Err(AppError::invalid("that run was not a tidy"));
            }
            if !run.state.is_finished() {
                return Err(AppError::conflict("wait for the tidy to finish"));
            }
            let plan = run.proposed.clone().filter(|p| !p.trim().is_empty()).ok_or_else(|| AppError::invalid("that tidy proposed nothing"))?;
            if run.replied_at.is_some() {
                return Err(AppError::conflict("you already answered that tidy"));
            }
            (run, plan)
        };
        // Queue the follow-up first: if that fails, the proposal stays in the Inbox to try again.
        let approved = self.enqueue_tidy(&run.employee_id, TidyReason::Approved, "", Some(plan))?;
        let answered = {
            let db = self.db();
            let mut answered = db.run(run_id)?.map(|r| r.run).unwrap_or(run);
            answered.replied_at = Some(now());
            answered.archived_at = Some(now());
            db.save_run(&answered)?;
            answered
        };
        self.sink().run_updated(&answered);
        Ok(approved)
    }

    /// The employee's tidy runs, newest first, for the Skills tab.
    pub fn tidy_runs(&self, employee_id: &str) -> Result<Vec<Run>> {
        self.db().tidy_runs(employee_id)
    }

    /// One pass over the employees who tidy on their own: queue a tidy for each that is due. Real work
    /// comes first: nobody is tidied while busy, or while the plan's windows are nearly full.
    pub fn tidy_tick(&self, at: Ts) -> Result<usize> {
        if self.db().settings()?.map(|s| s.paused).unwrap_or(false) || !tidy::room_for_a_tidy(&lock(&self.0.usage)) {
            return Ok(0);
        }
        let mut queued = 0;
        let employees = self.db().employees()?;
        for e in employees.into_iter().filter(|e| e.auto_tidy) {
            let (busy, last, taught) = {
                let db = self.db();
                let runs = db.tidy_runs(&e.id)?;
                let since = runs.iter().find(|r| r.state == RunState::Succeeded).map(|r| r.created_at);
                (!db.active_runs_of(&e.id)?.is_empty(), runs.into_iter().next(), db.taught_since(&e.id, since, tidy::TAUGHT_LIMIT)?.len())
            };
            if busy {
                continue;
            }
            let findings = self.skill_health(&e.id)?.findings;
            // A tidy that failed saw nothing through, so its findings still count as new.
            let seen: Vec<String> = match &last {
                Some(Run { kind: RunKind::Tidy { signature, .. }, state: RunState::Succeeded, .. }) => signature.clone(),
                _ => vec![],
            };
            let situation = tidy::Situation { now: at, findings: &findings, last: last.as_ref().map(|r| (r.created_at, seen.as_slice())), taught };
            if let Some(why) = tidy::due(&situation) {
                match self.enqueue_tidy(&e.id, TidyReason::Auto, &why, None) {
                    Ok(_) => queued += 1,
                    Err(err) => log!("queueing a tidy for {} failed: {err}", e.name),
                }
            }
        }
        Ok(queued)
    }

    /// The bigger changes a tidy proposes, recorded on its run for you to approve.
    pub fn propose_changes_tool(&self, caller_run: &str, plan: &str) -> Result<Value> {
        let plan = plan.trim();
        if plan.is_empty() {
            return Err(AppError::invalid("plan is empty"));
        }
        if !matches!(self.run(caller_run)?.kind, RunKind::Tidy { reason, .. } if reason != TidyReason::Approved) {
            return Err(AppError::invalid("propose_changes is only for a tidy run that has not been approved yet"));
        }
        let mut active = lock(&self.0.active);
        let a = active.get_mut(caller_run).ok_or_else(|| AppError::invalid("this run has ended"))?;
        match &mut a.proposed {
            Some(p) => {
                p.push_str("\n\n");
                p.push_str(plan);
            }
            None => a.proposed = Some(plan.to_string()),
        }
        Ok(json!({ "recorded": true, "note": "They see it in their Inbox with an Approve button. Do not make these changes in this run." }))
    }
}

/// `text` names the skill `name` as a whole word (not as part of a longer skill name).
fn mentions(text: &str, name: &str) -> bool {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    text.match_indices(name).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + name.len()..].chars().next();
        !before.is_some_and(word) && !after.is_some_and(word)
    })
}

/// Writes `text` to a recorded file (`None` deletes it) and records the change. `None` when the file
/// already holds it.
fn record(db: &Store, employee_dir: &Path, employee_id: &str, skill: &str, file: &str, text: Option<&str>) -> Result<Option<SkillRevision>> {
    if skill == own_skills::CRAFT_NAME {
        return Err(AppError::invalid("omega-skill-craft is managed by Omega Star"));
    }
    let path = own_skills::recorded_path(employee_dir, skill, file)?;
    let before = fs::read_to_string(&path).ok();
    if before.as_deref() == text {
        return Ok(None);
    }
    let change = match (&before, text) {
        (None, _) => ChangeKind::Added,
        (Some(_), None) => ChangeKind::Removed,
        _ => ChangeKind::Modified,
    };
    own_skills::write_recorded(&path, skill, text)?;
    let fc = own_skills::FileChange {
        change: SkillChange { skill: skill.into(), file: file.replace('\\', "/"), change },
        before,
        after: text.map(String::from),
    };
    let rev = own_skills::revision(fc, None, now());
    db.insert_revision(employee_id, &rev)?;
    Ok(Some(rev))
}

/// The file holds `after` (a missing file holds `None`).
fn unchanged_since(path: &Path, after: Option<&str>) -> bool {
    fs::read_to_string(path).ok().as_deref() == after
}

/// Undo writes back the whole earlier file, so a guarded file (the company page, what an employee
/// owns) is put back only while it still holds what the change wrote, or is already back.
fn safe_to_undo(employee_dir: &Path, r: &SkillRevision) -> Result<bool> {
    if !own_skills::guarded(&r.change.skill) {
        return Ok(true);
    }
    let path = own_skills::recorded_path(employee_dir, &r.change.skill, &r.change.file)?;
    Ok(unchanged_since(&path, r.after.as_deref()) || unchanged_since(&path, r.before.as_deref()))
}

/// Puts back what the revisions changed, skipping a guarded file that changed again since. Returns
/// how many files changed.
fn undo(db: &Store, employee_dir: &Path, employee_id: &str, revs: &[SkillRevision]) -> Result<u32> {
    let mut n = 0;
    for r in revs {
        if safe_to_undo(employee_dir, r)? && record(db, employee_dir, employee_id, &r.change.skill, &r.change.file, r.before.as_deref())?.is_some() {
            n += 1;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_since_compares_the_whole_file() {
        let dir = std::env::temp_dir().join(format!("omegastar-undo-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("CLAUDE.md");
        assert!(unchanged_since(&path, None), "a missing file is None");
        fs::write(&path, "one").unwrap();
        assert!(unchanged_since(&path, Some("one")));
        assert!(!unchanged_since(&path, Some("one more")));
        assert!(!unchanged_since(&path, None));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_keeps_a_newer_change_to_the_company_page() {
        let root = std::env::temp_dir().join(format!("omegastar-undo-{}", uuid::Uuid::new_v4()));
        let employee_dir = root.join("employees").join("e1");
        fs::create_dir_all(own_skills::skills_dir(&employee_dir).join("ada")).unwrap();
        let db = Store::open(&root.join("office.db")).unwrap();
        let page = own_skills::recorded_path(&employee_dir, own_skills::COMPANY, own_skills::COMPANY_FILE).unwrap();
        let skill = own_skills::recorded_path(&employee_dir, "ada", "SKILL.md").unwrap();
        fs::write(&page, "# Company page\n").unwrap();
        fs::write(&skill, "# Ada\n").unwrap();

        // A run changes a skill and the page, then the page changes again in a later conversation.
        let mut run_revs = vec![];
        for (key, path, after) in [(("ada", "SKILL.md"), &skill, "# Ada\n\nA new step.\n"), ((own_skills::COMPANY, own_skills::COMPANY_FILE), &page, "# Company page\n\n- Run line.\n")] {
            let before = fs::read_to_string(path).ok();
            fs::write(path, after).unwrap();
            let fc = own_skills::FileChange {
                change: SkillChange { skill: key.0.into(), file: key.1.into(), change: ChangeKind::Modified },
                before,
                after: Some(after.into()),
            };
            let rev = own_skills::revision(fc, Some("run-1".into()), now());
            db.insert_revision("e1", &rev).unwrap();
            run_revs.push(rev);
        }
        record(&db, &employee_dir, "e1", own_skills::COMPANY, own_skills::COMPANY_FILE, Some("# Company page\n\n- Run line.\n- Newer line.\n")).unwrap();

        let revs: Vec<SkillRevision> = db.revisions("e1").unwrap().into_iter().filter(|r| r.run_id.as_deref() == Some("run-1")).collect();
        assert_eq!(undo(&db, &employee_dir, "e1", &revs).unwrap(), 1, "only the skill file is put back");
        assert_eq!(fs::read_to_string(&skill).unwrap(), "# Ada\n");
        assert_eq!(fs::read_to_string(&page).unwrap(), "# Company page\n\n- Run line.\n- Newer line.\n");
        let page_rev = run_revs.iter().find(|r| r.change.skill == own_skills::COMPANY).unwrap();
        assert!(!safe_to_undo(&employee_dir, page_rev).unwrap(), "revert_revision refuses it too");
        let _ = fs::remove_dir_all(&root);
    }
}
