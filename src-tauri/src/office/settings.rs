//! Setup, settings, backups, usage limits and the skill catalog.

use super::*;

impl Office {
    pub fn company(&self) -> Result<Option<Company>> {
        self.db().company()
    }

    pub async fn setup_status(&self, background: bool) -> Result<SetupStatus> {
        let path = claude::locate(self.settings()?.claude_path.as_deref());
        let (version, account) = match &path {
            Some(p) => tokio::join!(claude::version(p), claude::auth_status(p)),
            None => (None, None),
        };
        Ok(SetupStatus {
            claude: ClaudeStatus {
                found: path.is_some(),
                path: path.map(|p| p.to_string_lossy().into_owned()),
                version,
            },
            account,
            company: self.company()?,
            background,
            build: self.0.build.clone(),
        })
    }

    pub async fn complete_setup(&self, company_name: String, ceo_name: String, title: Option<String>) -> Result<Company> {
        if company_name.trim().is_empty() || ceo_name.trim().is_empty() {
            return Err(AppError::invalid("company name and your name are required"));
        }
        let email = match claude::locate(self.settings()?.claude_path.as_deref()) {
            Some(p) => claude::auth_status(&p).await.and_then(|a| a.email),
            None => None,
        };
        let c = Self::edit_org(&self.db(), |org| {
            org.company = Company {
                name: company_name.trim().into(),
                ceo_name: ceo_name.trim().into(),
                title: Company::title_or_default(title.as_deref()),
                email,
                reports_to: org.company.reports_to.take(),
            };
            Ok(org.company.clone())
        })?;
        self.sink().org_changed();
        Ok(c)
    }

    /// Your name, title and company name, and who you report to (a person, or nobody).
    pub fn update_company(&self, name: String, ceo_name: String, title: String, reports_to: Option<String>) -> Result<Company> {
        if name.trim().is_empty() || ceo_name.trim().is_empty() {
            return Err(AppError::invalid("company name and your name are required"));
        }
        let c = {
            let db = self.db();
            if db.company()?.is_none() {
                return Err(AppError::invalid("finish setting up first"));
            }
            Self::edit_org(&db, |org| {
                org.company.name = name.trim().into();
                org.company.ceo_name = ceo_name.trim().into();
                org.company.title = Company::title_or_default(Some(&title));
                org.set_reports_to(reports_to.as_deref().map(str::trim).filter(|r| !r.is_empty()))?;
                Ok(org.company.clone())
            })?
        };
        self.sink().org_changed();
        Ok(c)
    }

    pub fn settings(&self) -> Result<AppSettings> {
        let mut s = self.db().settings()?.unwrap_or_default();
        s.data_dir = self.0.data_dir.to_string_lossy().into_owned();
        Ok(s)
    }

    pub fn save_settings(&self, mut s: AppSettings) -> Result<AppSettings> {
        s.max_concurrent_runs = s.max_concurrent_runs.clamp(1, 16);
        s.claude_path = s.claude_path.filter(|p| !p.trim().is_empty());
        s.data_dir = self.0.data_dir.to_string_lossy().into_owned();
        s.backup.keep = s.backup.keep.clamp(1, 100);
        s.backup.dir = s.backup.dir.trim().to_string();
        s.watch.validate()?;
        let old = self.settings()?.backup;
        // Only a changed folder, or turning backups on, is checked: an unusable default folder must
        // not block unrelated settings.
        let enabling = old.frequency == BackupFrequency::Off && s.backup.frequency != BackupFrequency::Off;
        if s.backup.dir != old.dir || enabling {
            backup::validate_dir(&s.backup.dir, self.data_dir())?;
        }
        self.db().set_settings(&s)?;
        self.sink().settings_changed(&s);
        if s.backup != old {
            self.sink().backup_updated(&self.backup_status()?);
        }
        self.pump();
        Ok(s)
    }

    pub fn set_paused(&self, paused: bool) -> Result<AppSettings> {
        let s = self.settings()?;
        self.save_settings(AppSettings { paused, ..s })
    }

    // ------------------------------------------------------------------ backups

    pub fn backup_status(&self) -> Result<BackupStatus> {
        let running = self.0.backing_up.try_lock().is_err();
        self.status_of_backups(running)
    }

    fn status_of_backups(&self, running: bool) -> Result<BackupStatus> {
        let freq = self.settings()?.backup.frequency;
        let mut st = self.db().backup_status()?.unwrap_or_default();
        st.running = running;
        st.next_at = backup::next_at(freq, &st, now());
        Ok(st)
    }

    /// Backs up now, whatever the schedule says. Fails if a backup is already running.
    pub async fn backup_now(&self) -> Result<BackupStatus> {
        let Ok(_backing_up) = self.0.backing_up.try_lock() else {
            return Err(AppError::conflict("A backup is already running."));
        };
        self.run_backup(now()).await
    }

    /// Backs up if an automatic backup is due at `at` and the last one is out of date or gone.
    /// Returns whether one was attempted.
    pub async fn backup_tick(&self, at: Ts) -> Result<bool> {
        let settings = self.settings()?.backup;
        let st = self.db().backup_status()?.unwrap_or_default();
        if !backup::is_due(settings.frequency, &st, at) {
            return Ok(false);
        }
        let Ok(_backing_up) = self.0.backing_up.try_lock() else { return Ok(false) };
        let dir = self.data_dir().to_path_buf();
        let fingerprint = tokio::task::spawn_blocking(move || backup::fingerprint(&dir))
            .await
            .map_err(|e| AppError::internal(format!("backup check: {e}")))?;
        // Read again: a backup by hand may have finished since.
        let mut st = self.db().backup_status()?.unwrap_or_default();
        if backup::up_to_date(&st, &fingerprint, &settings.dir, self.data_dir()) {
            st.checked_at = Some(at);
            self.db().set_backup_status(&st)?;
            self.sink().backup_updated(&self.status_of_backups(false)?);
            return Ok(false);
        }
        if let Err(e) = self.run_backup(at).await {
            log!("backup failed: {e}");
        }
        Ok(true)
    }

    /// Callers hold `backing_up`. Records the outcome either way.
    async fn run_backup(&self, at: Ts) -> Result<BackupStatus> {
        self.sink().backup_updated(&self.status_of_backups(true)?);
        let result = self.write_backup(at).await;
        let mut st = self.db().backup_status()?.unwrap_or_default();
        st.last_attempt_at = Some(at);
        match &result {
            Ok((file, sum, fingerprint)) => {
                st.last_at = Some(at);
                st.last_file = Some(file.clone());
                st.last_bytes = Some(sum.bytes);
                st.skipped = sum.skipped;
                st.left_out = sum.left_out;
                st.fingerprint = Some(fingerprint.clone());
                st.last_error = None;
            }
            Err(e) => st.last_error = Some(e.message.clone()),
        }
        self.db().set_backup_status(&st)?;
        let st = self.status_of_backups(false)?;
        self.sink().backup_updated(&st);
        match result {
            Ok(_) => Ok(st),
            Err(e) => {
                self.sink().notify("Backup failed", &e.message);
                Err(e)
            }
        }
    }

    /// Writes the zip beside older backups, then prunes. Returns the zip's path, and the office's
    /// fingerprint, taken before the snapshot so a change during the backup is caught next time.
    async fn write_backup(&self, at: Ts) -> Result<(String, backup::Summary, String)> {
        let settings = self.settings()?.backup;
        let dir = backup::validate_dir(&settings.dir, self.data_dir())?;
        fs::create_dir_all(&dir).map_err(|e| AppError::invalid(format!("Could not create the backup folder {}: {e}", dir.display())))?;
        let mut stamp = at.with_timezone(&Local);
        while dir.join(backup::file_name(stamp)).exists() {
            stamp += chrono::Duration::seconds(1);
        }
        let file = dir.join(backup::file_name(stamp));
        let manifest = json!({
            "app": "Omega Star",
            "version": env!("CARGO_PKG_VERSION"),
            "createdAt": at,
            "officeFolder": self.data_dir(),
            "restore": backup::RESTORE_STEPS,
        });
        let office = self.clone();
        let target = file.clone();
        let task = tokio::task::spawn_blocking(move || -> Result<(backup::Summary, String)> {
            let fingerprint = backup::fingerprint(office.data_dir());
            let snapshot = std::env::temp_dir().join(format!("omegastar-snapshot-{}.db", uuid::Uuid::new_v4()));
            let partial = PathBuf::from(format!("{}{}", target.display(), backup::PARTIAL));
            let written = office.db().snapshot_to(&snapshot).and_then(|_| backup::write_archive(office.data_dir(), &snapshot, &partial, manifest));
            let _ = fs::remove_file(&snapshot);
            let done = written.and_then(|sum| Ok(fs::rename(&partial, &target).map(|_| sum)?));
            if done.is_err() {
                let _ = fs::remove_file(&partial);
            }
            done.map(|sum| (sum, fingerprint))
        });
        let (sum, fingerprint) = task.await.map_err(|e| AppError::internal(format!("backup task: {e}")))??;
        if let Err(e) = backup::prune(&dir, settings.keep) {
            log!("could not prune old backups: {e}");
        }
        Ok((file.to_string_lossy().into_owned(), sum, fingerprint))
    }

    // ------------------------------------------------------------------ usage limits

    pub fn usage_limits(&self) -> UsageLimits {
        lock(&self.0.usage).clone()
    }

    /// Probes now, unless a probe finished while this call waited for the one in flight.
    pub async fn refresh_usage(&self) -> UsageLimits {
        let seen = self.usage_limits().observed_at;
        let _probing = self.0.probing.lock().await;
        if self.usage_limits().observed_at == seen {
            self.probe_usage().await;
        }
        self.usage_limits()
    }

    /// Callers hold `probing`.
    pub(super) async fn probe_usage(&self) {
        let Ok(settings) = self.settings() else { return };
        let Some(program) = claude::locate(settings.claude_path.as_deref()) else { return };
        if let Some(sample) = claude::probe_usage(&program, &self.0.data_dir).await {
            self.observe_usage(sample, true);
        }
    }

    pub(super) fn observe_usage(&self, sample: claude::UsageSample, from_probe: bool) {
        let (next, released) = {
            let mut u = lock(&self.0.usage);
            let held = u.held_until;
            *u = merge_usage(&u, sample, from_probe, now());
            (u.clone(), held.is_some() && u.held_until.is_none())
        };
        self.sink().usage_updated(&next);
        if released {
            if let Err(e) = self.db().set_held_until(None) {
                log!("could not clear the usage hold: {e}");
            }
            self.pump();
        }
    }

    /// Holds the office until the usage limit resets at `until`: no run starts before then. Tells you
    /// once, when the hold is new.
    pub(super) fn hold_until(&self, until: Ts) {
        let at = now();
        let (next, new) = {
            let mut u = lock(&self.0.usage);
            let new = u.held_until.is_none_or(|t| t <= at);
            if u.held_until.is_none_or(|t| until > t) {
                u.held_until = Some(until);
            }
            (u.clone(), new)
        };
        if let Err(e) = self.db().set_held_until(next.held_until) {
            log!("could not save the usage hold: {e}");
        }
        self.sink().usage_updated(&next);
        if new {
            let time = until.with_timezone(&Local).format("%-I:%M %P");
            self.sink().notify("Waiting for the usage limit", &format!("Work starts again at {time}, when the limit resets."));
        }
    }

    /// Ends a hold whose reset time has come. Returns whether there was one.
    pub(super) fn release_hold(&self, now: Ts) -> bool {
        let next = {
            let mut u = lock(&self.0.usage);
            if !u.held_until.is_some_and(|t| t <= now) {
                return false;
            }
            u.held_until = None;
            u.clone()
        };
        if let Err(e) = self.db().set_held_until(None) {
            log!("could not clear the usage hold: {e}");
        }
        self.sink().usage_updated(&next);
        true
    }

    // ------------------------------------------------------------------ account

    /// Asks Claude Code which account it is signed in to and caches the answer, `None` when it
    /// could not tell. The only `claude auth status` cache in the app.
    pub(super) async fn refresh_account(&self) {
        let program = self.settings().ok().and_then(|s| claude::locate(s.claude_path.as_deref()));
        let account = match program {
            Some(p) => claude::auth_status(&p).await,
            None => None,
        };
        *lock(&self.0.account) = Some((now(), account.clone()));
        let recorded = match self.db().company() {
            Ok(c) => c.and_then(|c| c.email),
            Err(e) => {
                log!("reading the account set up for the office failed: {e}");
                return;
            }
        };
        let issue = account_issue(recorded.as_deref(), account.as_ref());
        let changed = {
            let mut u = lock(&self.0.usage);
            (u.account_issue != issue).then(|| {
                u.account_issue = issue.clone();
                u.clone()
            })
        };
        if let Some(next) = changed {
            self.sink().usage_updated(&next);
            if let Some(i) = &issue {
                self.sink().notify("Routines are paused", &i.message);
            }
        }
        // Background work waits for the first check, so this may be what starts it.
        self.pump();
    }

    /// Makes the account Claude Code is signed in to now the office's own, which ends an account pause.
    pub async fn use_signed_in_account(&self) -> Result<Company> {
        let account = match claude::locate(self.settings()?.claude_path.as_deref()) {
            Some(p) => claude::auth_status(&p).await,
            None => None,
        };
        let email = account
            .as_ref()
            .filter(|a| a.logged_in)
            .and_then(|a| a.email.clone())
            .ok_or_else(|| AppError::invalid("Claude Code is not signed in."))?;
        let c = {
            let db = self.db();
            if db.company()?.is_none() {
                return Err(AppError::invalid("finish setting up first"));
            }
            Self::edit_org(&db, |org| {
                org.company.email = Some(email);
                Ok(org.company.clone())
            })?
        };
        *lock(&self.0.account) = Some((now(), account));
        let next = {
            let mut u = lock(&self.0.usage);
            u.account_issue = None;
            u.clone()
        };
        self.sink().org_changed();
        self.sink().usage_updated(&next);
        self.pump();
        Ok(c)
    }

    /// The last account check and when it ran; `None` when it has not run yet.
    pub(super) fn account_cached(&self) -> Option<(Ts, Option<Account>)> {
        lock(&self.0.account).clone()
    }

    /// The stamp for a run starting now. Records the organisation, never the email.
    pub(super) fn run_build(&self) -> RunBuild {
        let org = self.account_cached().and_then(|(_, a)| a).and_then(|a| a.org_name);
        RunBuild { app: self.0.build.clone(), claude: None, org }
    }

    pub fn skill_catalog(&self) -> Vec<SkillInfo> {
        catalog::scan(&catalog::config_dir())
    }

    pub fn skill_sources(&self) -> SkillSources {
        catalog::sources(&catalog::config_dir())
    }
}

/// Folds a usage observation into the cached limits. A run's `rate_limit_event` updates only the
/// windows it reports. A `/usage` probe replaces both; its reset time is only text, so a still
/// future exact reset time from an earlier run is kept.
///
/// The hold and the account issue carry over, except that a probe showing both windows under 100%
/// ends the hold.
fn merge_usage(old: &UsageLimits, sample: claude::UsageSample, from_probe: bool, now: Ts) -> UsageLimits {
    let subscription = sample.five_hour.is_some() || sample.seven_day.is_some();
    let account_issue = old.account_issue.clone();
    if from_probe && !subscription {
        return UsageLimits { five_hour: None, seven_day: None, observed_at: Some(now), subscription: false, held_until: old.held_until, account_issue };
    }
    let under = |w: &Option<UsageWindow>| w.as_ref().is_some_and(|w| w.percent < 100.0);
    let held_until = if from_probe && under(&sample.five_hour) && under(&sample.seven_day) { None } else { old.held_until };
    let fold = |old: &Option<UsageWindow>, new: Option<UsageWindow>| match new {
        None if from_probe => None,
        None => old.clone(),
        Some(mut w) => {
            if w.resets_at.is_none() {
                w.resets_at = old.as_ref().and_then(|o| o.resets_at).filter(|t| *t > now);
            }
            Some(w)
        }
    };
    UsageLimits {
        five_hour: fold(&old.five_hour, sample.five_hour),
        seven_day: fold(&old.seven_day, sample.seven_day),
        observed_at: Some(now),
        subscription: true,
        held_until,
        account_issue,
    }
}

/// Why routines must wait for the account: Claude Code is signed out, or signed in to another account
/// than `recorded`, the one set up. `None` when it is the same, or when either is unknown.
fn account_issue(recorded: Option<&str>, now: Option<&Account>) -> Option<AccountIssue> {
    let (recorded, now) = (recorded?, now?);
    if !now.logged_in {
        return Some(AccountIssue { message: "Claude Code is signed out, so routines are paused until you sign in again.".into(), signed_in_as: None });
    }
    let current = now.email.as_deref()?;
    (!current.eq_ignore_ascii_case(recorded)).then(|| AccountIssue {
        message: format!("Claude Code is signed in as {current} instead of {recorded}, so routines are paused until you switch back."),
        signed_in_as: Some(current.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(percent: f64) -> Option<UsageWindow> {
        Some(UsageWindow { percent, resets_at: None, resets_text: None })
    }

    #[test]
    fn the_hold_and_account_issue_survive_a_merge_until_a_probe_shows_room() {
        let at = Utc::now();
        let issue = AccountIssue { message: "Paused.".into(), signed_in_as: None };
        let old = UsageLimits { held_until: Some(at + chrono::Duration::hours(2)), account_issue: Some(issue.clone()), ..UsageLimits::default() };
        let from_run = merge_usage(&old, claude::UsageSample { five_hour: window(100.0), seven_day: None }, false, at);
        assert_eq!((from_run.held_until, from_run.account_issue.as_ref()), (old.held_until, Some(&issue)));
        let full = merge_usage(&old, claude::UsageSample { five_hour: window(100.0), seven_day: window(40.0) }, true, at);
        assert_eq!((full.held_until, full.account_issue.as_ref()), (old.held_until, Some(&issue)));
        let room = merge_usage(&old, claude::UsageSample { five_hour: window(12.0), seven_day: window(40.0) }, true, at);
        assert_eq!((room.held_until, room.account_issue), (None, Some(issue)));
    }

    fn account(logged_in: bool, email: Option<&str>) -> Account {
        Account { logged_in, email: email.map(String::from), org_name: None, plan: None }
    }

    #[test]
    fn account_issue_covers_each_case() {
        let ada = account(true, Some("ada@example.com"));
        assert_eq!(account_issue(Some("ada@example.com"), None), None);
        assert_eq!(account_issue(None, Some(&ada)), None);
        let out = account_issue(Some("ada@example.com"), Some(&account(false, None))).unwrap();
        assert_eq!(out.message, "Claude Code is signed out, so routines are paused until you sign in again.");
        assert_eq!(out.signed_in_as, None);
        let other = account_issue(Some("ada@example.com"), Some(&account(true, Some("bob@example.com")))).unwrap();
        assert_eq!(other.message, "Claude Code is signed in as bob@example.com instead of ada@example.com, so routines are paused until you switch back.");
        assert_eq!(other.signed_in_as.as_deref(), Some("bob@example.com"));
        assert_eq!(account_issue(Some("Ada@Example.com"), Some(&ada)), None);
    }
}
