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
        let next = {
            let mut u = lock(&self.0.usage);
            *u = merge_usage(&u, sample, from_probe, now());
            u.clone()
        };
        self.sink().usage_updated(&next);
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
        *lock(&self.0.account) = Some((now(), account));
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
fn merge_usage(old: &UsageLimits, sample: claude::UsageSample, from_probe: bool, now: Ts) -> UsageLimits {
    let subscription = sample.five_hour.is_some() || sample.seven_day.is_some();
    if from_probe && !subscription {
        return UsageLimits { five_hour: None, seven_day: None, observed_at: Some(now), subscription: false };
    }
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
    }
}
