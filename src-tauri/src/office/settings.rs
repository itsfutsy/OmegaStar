//! Setup, settings, backups, transcripts, usage limits and the skill catalog.

use super::*;

/// The first transcript sweep, a while after launch, and the time between sweeps.
const SWEEP_FIRST: Duration = Duration::from_secs(300);
const SWEEP_EVERY: Duration = Duration::from_secs(24 * 3600);
/// How long a backup may take before Omega Star gives up on it, so a folder the system holds up fails loudly.
const BACKUP_LIMIT: Duration = Duration::from_secs(600);

impl Office {
    pub async fn setup_status(&self, background: bool) -> Result<SetupStatus> {
        let settings = self.settings()?;
        let path = claude::locate(settings.claude_path.as_deref());
        let proxy_env = settings.proxy.env();
        let (version, account) = match &path {
            Some(p) => tokio::join!(claude::version(p), claude::auth_status(p, &proxy_env)),
            None => (None, None),
        };
        Ok(SetupStatus {
            claude: ClaudeStatus {
                found: path.is_some(),
                path: path.map(|p| p.to_string_lossy().into_owned()),
                version,
            },
            account,
            company: self.db().company()?,
            background,
            build: self.0.build.clone(),
            sandbox: self.0.sandbox,
        })
    }

    /// The models an employee can be given: Claude Code's own list.
    pub async fn models(&self) -> Result<Vec<ModelOption>> {
        let program = claude::locate(self.settings()?.claude_path.as_deref()).ok_or_else(|| AppError::invalid("Claude Code was not found"))?;
        claude::models(&program).await
    }

    pub async fn complete_setup(&self, company_name: String, ceo_name: String, title: Option<String>) -> Result<Company> {
        if company_name.trim().is_empty() || ceo_name.trim().is_empty() {
            return Err(AppError::invalid("company name and your name are required"));
        }
        let email = signed_in(&self.settings()?).await.and_then(|a| a.email);
        let c = Self::edit_org(&self.db(), |org| {
            org.company = Company {
                name: company_name.trim().into(),
                ceo_name: ceo_name.trim().into(),
                title: Company::title_or_default(title.as_deref()),
                email,
                reports_to: org.company.reports_to.take(),
                image: org.company.image.take(),
            };
            Ok(org.company.clone())
        })?;
        self.sink().org_changed();
        Ok(c)
    }

    /// Your name, title and company name, and who you report to (a person, or nobody).
    pub fn update_company(&self, name: String, ceo_name: String, title: String, reports_to: Option<String>, image: Option<String>) -> Result<Company> {
        if name.trim().is_empty() || ceo_name.trim().is_empty() {
            return Err(AppError::invalid("company name and your name are required"));
        }
        check_image(image.as_deref())?;
        let c = {
            let db = self.db();
            if db.company()?.is_none() {
                return Err(AppError::invalid("finish setting up first"));
            }
            Self::edit_org(&db, |org| {
                org.company.name = name.trim().into();
                org.company.ceo_name = ceo_name.trim().into();
                org.company.title = Company::title_or_default(Some(&title));
                org.company.image = image;
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
        s.transcript_days = s.transcript_days.map(|d| d.max(1));
        s.watch.validate()?;
        if s.notifications.quiet.is_some_and(|q| q.start_minute >= 1440 || q.end_minute >= 1440 || q.start_minute == q.end_minute) {
            return Err(AppError::invalid("quiet hours must start and end at different times of the day"));
        }
        let p = &mut s.proxy;
        p.url = p.url.trim().trim_end_matches('/').to_string();
        p.api_key = p.api_key.trim().to_string();
        p.management_key = p.management_key.trim().to_string();
        if !p.url.is_empty() && !p.url.starts_with("http://") && !p.url.starts_with("https://") {
            return Err(AppError::invalid("the CLI proxy address must start with http:// or https://"));
        }
        if !p.management_key.is_empty() && p.management_key == p.api_key {
            return Err(AppError::invalid("the management key is the proxy's management secret, not one of its API keys"));
        }
        let prev = self.settings()?;
        let old = prev.backup;
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
        if s.proxy != prev.proxy {
            self.proxy_changed(s.proxy.url != prev.proxy.url);
            // Probed again even when a probe is in flight: it may still be reading the old source.
            let office = self.clone();
            self.0.rt.spawn(async move {
                let _probing = office.0.probing.lock().await;
                office.probe_usage().await;
            });
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
        if self.0.sandbox {
            return Err(AppError::conflict("This office is a sandbox copy, so it never backs up."));
        }
        let Ok(backing_up) = self.0.backing_up.clone().try_lock_owned() else {
            return Err(AppError::conflict("A backup is already running."));
        };
        self.run_backup(backing_up, now(), BACKUP_LIMIT).await
    }

    /// Backs up if an automatic backup is due at `at` and the last one is out of date or gone.
    pub async fn backup_tick(&self, at: Ts) -> Result<()> {
        if self.0.sandbox {
            return Ok(());
        }
        let settings = self.settings()?.backup;
        let st = self.db().backup_status()?.unwrap_or_default();
        if !backup::is_due(settings.frequency, &st, at) {
            return Ok(());
        }
        let Ok(backing_up) = self.0.backing_up.clone().try_lock_owned() else { return Ok(()) };
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
            return Ok(());
        }
        if let Err(e) = self.run_backup(backing_up, at, BACKUP_LIMIT).await {
            log!("backup failed: {e}");
        }
        Ok(())
    }

    /// Records the outcome either way, a write still going after `limit` as a failure. The write holds
    /// `backing_up` until it ends, so a stuck one stops the next from starting on top of it.
    async fn run_backup(&self, backing_up: tokio::sync::OwnedMutexGuard<()>, at: Ts, limit: Duration) -> Result<BackupStatus> {
        self.sink().backup_updated(&self.status_of_backups(true)?);
        let office = self.clone();
        let write = tokio::task::spawn_blocking(move || {
            let _backing_up = backing_up;
            office.write_backup(at)
        });
        let result = match tokio::time::timeout(limit, write).await {
            Ok(done) => done.unwrap_or_else(|e| Err(AppError::internal(format!("backup task: {e}")))),
            Err(_) => Err(AppError::internal(format!(
                "The backup took over {} minutes, so Omega Star stopped waiting. Check that the backup folder is reachable and Omega Star is allowed to use it.",
                limit.as_secs() / 60
            ))),
        };
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
        let st = self.backup_status()?;
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
    fn write_backup(&self, at: Ts) -> Result<(String, backup::Summary, String)> {
        let settings = self.settings()?.backup;
        let dir = backup::validate_dir(&settings.dir, self.data_dir())?;
        fs::create_dir_all(&dir).map_err(|e| AppError::invalid(format!("Could not create the backup folder {}: {e}", dir.display())))?;
        let last_bytes = self.db().backup_status()?.unwrap_or_default().last_bytes;
        backup::make_room(&dir, settings.keep, last_bytes, free_bytes, backup::on_disk)?;
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
        let fingerprint = backup::fingerprint(self.data_dir());
        let snapshot = std::env::temp_dir().join(format!("omegastar-snapshot-{}.db", uuid::Uuid::new_v4()));
        let partial = PathBuf::from(format!("{}{}", file.display(), backup::PARTIAL));
        // Its own statement, so the store is free again before the zip goes to a folder that may hang.
        let snapped = self.db().snapshot_to(&snapshot);
        let written = snapped.and_then(|_| backup::write_archive(self.data_dir(), &snapshot, &partial, manifest));
        let _ = fs::remove_file(&snapshot);
        let done = written.and_then(|sum| Ok(fs::rename(&partial, &file).map(|_| sum)?));
        if done.is_err() {
            let _ = fs::remove_file(&partial);
        }
        let sum = done?;
        if let Err(e) = backup::prune(&dir, settings.keep) {
            log!("could not prune old backups: {e}");
        }
        Ok((file.to_string_lossy().into_owned(), sum, fingerprint))
    }

    // ------------------------------------------------------------------ transcripts

    /// Sweeps old transcripts five minutes after launch, then once a day.
    pub fn spawn_transcript_sweep(&self) {
        let office = self.clone();
        self.0.rt.spawn(async move {
            let mut every = tokio::time::interval_at(tokio::time::Instant::now() + SWEEP_FIRST, SWEEP_EVERY);
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                every.tick().await;
                let o = office.clone();
                match tokio::task::spawn_blocking(move || o.sweep_transcripts(now())).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => log!("deleting old transcripts failed: {e}"),
                    Err(e) => log!("deleting old transcripts failed: {e}"),
                }
            }
        });
    }

    /// Deletes the transcripts and logs of runs that finished more than `transcript_days` before `at`. The rest
    /// of each run's folder (the pages it showed, archives) and its row and report stay. Returns how many went.
    pub(super) fn sweep_transcripts(&self, at: Ts) -> Result<usize> {
        let Some(days) = self.settings()?.transcript_days else { return Ok(0) };
        let runs_dir = self.0.data_dir.join("runs");
        let ids: Vec<String> = fs::read_dir(&runs_dir)?
            .flatten()
            .filter(|e| e.path().join(TRANSCRIPT).is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        let runs = self.db().runs_by_id(&ids)?;
        let mut deleted = 0;
        for id in expired(&runs, at - chrono::Duration::days(days.into())) {
            let dir = runs_dir.join(id);
            match fs::remove_file(dir.join(TRANSCRIPT)) {
                Ok(()) => deleted += 1,
                Err(e) => log!("could not delete the transcript of run {id}: {e}"),
            }
            let _ = fs::remove_file(dir.join("stderr.log"));
        }
        if deleted > 0 {
            log!("deleted the transcripts of {deleted} runs that finished over {days} days ago");
        }
        Ok(deleted)
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

    /// Callers hold `probing`. Behind a CLI proxy, its accounts' limits are the office's, not those of
    /// the account Claude Code is signed in to.
    pub(super) async fn probe_usage(&self) {
        let Ok(settings) = self.settings() else { return };
        let sample = if settings.proxy.url.is_empty() {
            let Some(program) = claude::locate(settings.claude_path.as_deref()) else { return };
            claude::probe_usage(&program, &self.0.data_dir).await
        } else {
            let read = proxy::usage(&settings.proxy).await;
            self.0.proxy_refused.store(read.as_ref().is_err_and(|f| f.refused), Ordering::Relaxed);
            self.set_proxy_issue(read.as_ref().err().map(|f| f.message.clone()));
            read.ok()
        };
        if let Some(sample) = sample {
            self.observe_usage(sample, true);
        }
    }

    fn set_proxy_issue(&self, issue: Option<String>) {
        let next = {
            let mut u = lock(&self.0.usage);
            if u.proxy_issue == issue {
                return;
            }
            u.proxy_issue = issue;
            u.clone()
        };
        self.sink().usage_updated(&next);
    }

    /// The proxy settings changed, so a key it refused may work now. With another source of limits
    /// (`new_source`), the old one's readings and hold mean nothing: a run that meets a limit holds again.
    fn proxy_changed(&self, new_source: bool) {
        self.0.proxy_refused.store(false, Ordering::Relaxed);
        let next = {
            let mut u = lock(&self.0.usage);
            *u = if new_source {
                UsageLimits { account_issue: u.account_issue.take(), ..UsageLimits::default() }
            } else {
                UsageLimits { proxy_issue: None, ..u.clone() }
            };
            u.clone()
        };
        if new_source {
            if let Err(e) = self.db().set_held_until(None) {
                log!("could not clear the usage hold: {e}");
            }
        }
        self.sink().usage_updated(&next);
    }

    pub(super) fn observe_usage(&self, sample: claude::UsageSample, from_probe: bool) {
        let (next, released) = {
            let mut u = lock(&self.0.usage);
            let held = u.held_until;
            *u = merge_usage(&u, sample, from_probe, now());
            // A rate limit is not in the usage windows, so room there says nothing about it.
            if self.0.transient_hold.load(Ordering::SeqCst) {
                u.held_until = held;
            }
            (u.clone(), held.filter(|_| u.held_until.is_none()))
        };
        self.sink().usage_updated(&next);
        if let Some(until) = released {
            log!("hold for the usage limit ended early: a usage check showed room (it held work until {})", until.with_timezone(&Local).format("%Y-%m-%d %H:%M"));
            if let Err(e) = self.db().set_held_until(None) {
                log!("could not clear the usage hold: {e}");
            }
            self.pump();
        }
    }

    /// Holds the office until `until`, when the usage limit resets or a rate limit has had time to ease: no run
    /// starts before then. Tells you once, when the hold is new.
    pub(super) fn hold_until(&self, until: Ts, why: Limit) {
        let at = now();
        let (next, new) = {
            let mut u = lock(&self.0.usage);
            let new = u.held_until.is_none_or(|t| t <= at);
            if u.held_until.is_none_or(|t| until > t) {
                u.held_until = Some(until);
                self.0.transient_hold.store(why == Limit::Transient, Ordering::SeqCst);
            }
            (u.clone(), new)
        };
        if let Err(e) = self.db().set_held_until(next.held_until) {
            log!("could not save the usage hold: {e}");
        }
        self.save_rate_limits();
        self.sink().usage_updated(&next);
        if new {
            log!("holding work for {} until {}", why.as_str(), until.with_timezone(&Local).format("%Y-%m-%d %H:%M"));
            let time = until.with_timezone(&Local).format("%-I:%M %P");
            match why {
                Limit::Usage => self.sink().notify("Waiting for the usage limit", &format!("Work starts again at {time}, when the limit resets.")),
                Limit::Transient => self.sink().notify("Waiting out a rate limit", &format!("Claude is turning requests away for now, so work starts again at {time}.")),
            }
        }
    }

    /// Keeps the rate limits in a row and whether the hold is for one in the store, so a restart keeps the backoff and a
    /// usage check after it cannot end a rate limit's hold.
    pub(super) fn save_rate_limits(&self) {
        let (n, holding) = (self.0.transient_limits.load(Ordering::SeqCst), self.0.transient_hold.load(Ordering::SeqCst));
        if let Err(e) = self.db().set_rate_limits(n, holding) {
            log!("could not save the rate limits: {e}");
        }
    }

    /// Ends a hold whose reset time has come. Returns whether there was one.
    pub(super) fn release_hold(&self, now: Ts) -> bool {
        let (next, until) = {
            let mut u = lock(&self.0.usage);
            let Some(until) = u.held_until.filter(|t| *t <= now) else { return false };
            u.held_until = None;
            (u.clone(), until)
        };
        let why = if self.0.transient_hold.load(Ordering::SeqCst) { Limit::Transient } else { Limit::Usage };
        log!("hold for {} ended (it held work until {})", why.as_str(), until.with_timezone(&Local).format("%Y-%m-%d %H:%M"));
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
        let settings = self.settings().unwrap_or_default();
        let account = signed_in(&settings).await;
        *lock(&self.0.account) = Some((now(), account.clone()));
        let recorded = match self.db().company() {
            Ok(c) => c.and_then(|c| c.email),
            Err(e) => {
                log!("reading the account set up for the office failed: {e}");
                return;
            }
        };
        // Behind the CLI proxy, runs work whichever account Claude Code is signed in to, or none.
        let issue = if settings.proxy.url.is_empty() { account_issue(recorded.as_deref(), account.as_ref()) } else { None };
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
        let account = signed_in(&self.settings()?).await;
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
/// The hold and the account issue carry over, except that a probe showing room in every window the
/// account has ends the hold. A window it does not report holds nothing back: a pool of Team seats has
/// no 7-day limit.
fn merge_usage(old: &UsageLimits, sample: claude::UsageSample, from_probe: bool, now: Ts) -> UsageLimits {
    let subscription = sample.five_hour.is_some() || sample.seven_day.is_some();
    let account_issue = old.account_issue.clone();
    let proxy_issue = old.proxy_issue.clone();
    if from_probe && !subscription {
        return UsageLimits { five_hour: None, seven_day: None, observed_at: Some(now), subscription: false, held_until: old.held_until, account_issue, proxy_issue };
    }
    let room = |w: &Option<UsageWindow>| w.as_ref().is_none_or(|w| w.percent < 100.0);
    let held_until = if from_probe && room(&sample.five_hour) && room(&sample.seven_day) { None } else { old.held_until };
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
        proxy_issue,
    }
}

/// The runs whose transcripts the sweep deletes: those that finished before `cutoff`. Pure.
fn expired(runs: &[Run], cutoff: Ts) -> Vec<&str> {
    runs.iter().filter(|r| r.state.is_finished() && r.ended_at.is_some_and(|t| t < cutoff)).map(|r| r.id.as_str()).collect()
}

/// The account Claude Code is signed in to; `None` when it is not found or could not tell.
async fn signed_in(settings: &AppSettings) -> Option<Account> {
    let program = claude::locate(settings.claude_path.as_deref())?;
    claude::auth_status(&program, &settings.proxy.env()).await
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
    use crate::fixtures::{open_office, temp_dir};

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
        let no_week = merge_usage(&old, claude::UsageSample { five_hour: window(20.0), seven_day: None }, true, at);
        assert_eq!(no_week.held_until, None, "an account with no 7-day window is not held for one");
        let full_week = merge_usage(&old, claude::UsageSample { five_hour: None, seven_day: window(100.0) }, true, at);
        assert_eq!(full_week.held_until, old.held_until);
    }

    fn account(logged_in: bool, email: Option<&str>) -> Account {
        Account { logged_in, email: email.map(String::from), org_name: None, plan: None }
    }

    #[tokio::test]
    async fn turning_on_a_proxy_drops_the_old_hold_and_a_refused_key_stops_the_monitor() {
        let dir = temp_dir("proxy");
        Store::open(&dir.join("office.db")).unwrap().set_held_until(Some(Utc::now() + chrono::Duration::hours(3))).unwrap();
        let office = open_office(&dir).await;
        let url = proxy::fake_proxy().await;
        let with = |api_key: &str, management_key: &str| AppSettings {
            proxy: ProxySettings { url: url.clone(), api_key: api_key.into(), management_key: management_key.into() },
            ..office.settings().unwrap()
        };

        assert!(office.save_settings(with("api-key", "api-key")).is_err(), "an API key is not the management key");
        office.save_settings(with("api-key", "wrong-key")).unwrap();
        assert_eq!(office.usage_limits().held_until, None, "the signed-in account's hold means nothing for the pool");
        assert_eq!(office.db().held_until().unwrap(), None);
        office.refresh_usage().await;
        assert!(office.0.proxy_refused.load(Ordering::Relaxed));
        assert!(office.usage_limits().proxy_issue.is_some_and(|i| i.contains("invalid management key")));

        office.save_settings(with("api-key", "right-key")).unwrap();
        assert!(!office.0.proxy_refused.load(Ordering::Relaxed));
        office.refresh_usage().await;
        let u = office.usage_limits();
        assert_eq!((u.proxy_issue, u.five_hour.map(|w| w.percent)), (None, Some(60.0)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn quiet_hours_start_and_end_at_different_times() {
        let dir = temp_dir("quiet");
        let office = open_office(&dir).await;
        let with = |start_minute: u16, end_minute: u16| AppSettings {
            notifications: NotificationSettings { mode: NotifyMode::All, quiet: Some(QuietHours { start_minute, end_minute }) },
            ..office.settings().unwrap()
        };
        assert!(office.save_settings(with(480, 480)).is_err());
        assert!(office.save_settings(with(1320, 1440)).is_err(), "midnight is minute 0");
        let saved = office.save_settings(with(1320, 480)).unwrap();
        assert_eq!(office.settings().unwrap().notifications, saved.notifications, "overnight, 10 pm to 8 am");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_sweep_takes_only_runs_that_finished_before_the_cutoff() {
        let cutoff: Ts = "2026-09-01T00:00:00Z".parse().unwrap();
        let day = chrono::Duration::days(1);
        let run = |state: RunState, ended_at: Option<Ts>| Run { state, ended_at, ..Run::new("e1", RunKind::Manual, "go".into(), cutoff - day * 60) };
        let runs = [
            run(RunState::Succeeded, Some(cutoff - day)),
            run(RunState::Failed, Some(cutoff - day * 30)),
            run(RunState::Cancelled, Some(cutoff - day)),
            run(RunState::Lost, Some(cutoff - day)),
            run(RunState::Succeeded, Some(cutoff + day)),
            run(RunState::Running, Some(cutoff - day)),
            run(RunState::Queued, Some(cutoff - day)),
            run(RunState::Succeeded, None),
        ];
        let old: Vec<&str> = runs[..4].iter().map(|r| r.id.as_str()).collect();
        assert_eq!(expired(&runs, cutoff), old, "finished before the cutoff, and only those");
    }

    #[tokio::test]
    async fn a_backup_past_its_limit_fails_and_says_why() {
        let (dir, backups) = (temp_dir("stall-from"), temp_dir("stall-zips"));
        let office = open_office(&dir).await;
        let backup = BackupSettings { dir: backups.to_string_lossy().into_owned(), ..BackupSettings::default() };
        office.save_settings(AppSettings { backup, ..office.settings().unwrap() }).unwrap();
        let backing_up = office.0.backing_up.clone().try_lock_owned().unwrap();
        // No backup ends in no time, so a zero limit stands in for a folder the system holds up.
        let e = office.run_backup(backing_up, Utc::now(), Duration::ZERO).await.unwrap_err();
        assert!(e.message.starts_with("The backup took over 0 minutes"), "{}", e.message);
        assert_eq!(office.backup_status().unwrap().last_error, Some(e.message), "recorded as any failed backup is");
        // The write still holds the backup lock, so no second backup starts until it ends.
        drop(office.0.backing_up.lock().await);
        drop(office);
        for d in [dir, backups] {
            let _ = fs::remove_dir_all(d);
        }
    }

    #[tokio::test]
    async fn the_sweep_deletes_old_transcripts_and_keeps_their_rows() {
        let dir = temp_dir("sweep");
        let office = open_office(&dir).await;
        let at = Utc::now();
        let finished = |days: i64| {
            let mut r = Run::new("e1", RunKind::Manual, "go".into(), at - chrono::Duration::days(days));
            r.state = RunState::Succeeded;
            r.started_at = Some(r.created_at);
            r.ended_at = Some(r.created_at);
            r.summary = Some("All done.".into());
            r
        };
        let (old, recent) = (finished(40), finished(10));
        for r in [&old, &recent] {
            office.db().insert_run(r, &r.id).unwrap();
            fs::create_dir_all(dir.join("runs").join(&r.id)).unwrap();
            fs::write(dir.join("runs").join(&r.id).join("transcript.jsonl"), "{}\n").unwrap();
        }
        fs::create_dir_all(dir.join("runs/not-a-run")).unwrap();

        assert_eq!(office.sweep_transcripts(at).unwrap(), 0, "kept forever unless you choose otherwise");
        office.save_settings(AppSettings { transcript_days: Some(30), ..office.settings().unwrap() }).unwrap();
        assert_eq!(office.sweep_transcripts(at).unwrap(), 1);
        assert!(!dir.join("runs").join(&old.id).join("transcript.jsonl").exists());
        assert!(dir.join("runs").join(&old.id).exists(), "the rest of its folder, such as pages it showed, stays");
        assert!(dir.join("runs").join(&recent.id).join("transcript.jsonl").exists());
        assert!(dir.join("runs/not-a-run").exists(), "a folder no run owns is left alone");

        let gone = office.run_detail(&old.id).unwrap();
        assert!(gone.events.is_empty());
        assert_eq!(gone.run.summary.as_deref(), Some("All done."), "the row and its report stay");
        drop(office);
        let _ = fs::remove_dir_all(&dir);
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
