//! Settings, Claude Code's status and account, backups and usage limits.

use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeStatus {
    pub found: bool,
    pub path: Option<String>,
    pub version: Option<String>,
}

/// A model Claude Code offers: the exact ID `--model` takes, and the name its own picker shows.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelOption {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub logged_in: bool,
    pub email: Option<String>,
    pub org_name: Option<String>,
    pub plan: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupStatus {
    pub claude: ClaudeStatus,
    pub account: Option<Account>,
    pub company: Option<Company>,
    pub background: bool,
    pub build: Build,
    /// A test copy that never starts Claude Code, backs up or watches anyone (`sandbox`).
    pub sandbox: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub max_concurrent_runs: u32,
    pub claude_path: Option<String>,
    /// Informational: the data dir is chosen at startup (OMEGASTAR_DATA_DIR or the OS default).
    pub data_dir: String,
    pub paused: bool,
    /// Absent from settings saved before backups existed.
    pub backup: BackupSettings,
    /// Absent from settings saved before requests were watched.
    pub watch: WatchSettings,
    /// Absent from settings saved before the CLI proxy existed.
    pub proxy: ProxySettings,
    /// Days a finished run's folder (its transcript, log and the pages it showed) is kept; `None` = forever.
    /// Its row and report stay either way.
    pub transcript_days: Option<u32>,
    /// Absent from settings saved before notifications could be batched.
    pub notifications: NotificationSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            max_concurrent_runs: 3,
            claude_path: None,
            data_dir: String::new(),
            paused: false,
            backup: BackupSettings::default(),
            watch: WatchSettings::default(),
            proxy: ProxySettings::default(),
            transcript_days: None,
            notifications: NotificationSettings::default(),
        }
    }
}

/// How desktop notifications reach you (`notices::Notices`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct NotificationSettings {
    pub mode: NotifyMode,
    /// No notification shows between these times; one summary of what came in shows when they end. `None` = never quiet.
    pub quiet: Option<QuietHours>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum NotifyMode {
    /// Each one as it comes.
    All,
    /// An employee's decisions in the 90 seconds after their first show as one.
    #[default]
    Batched,
    /// None at all.
    Off,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QuietHours {
    /// Local minutes since midnight, under 1440. A start later than the end runs overnight, e.g. 10 pm to 8 am.
    pub start_minute: u16,
    pub end_minute: u16,
}

impl QuietHours {
    /// Whether `minute` (local minutes since midnight) falls in them. Pure.
    pub fn contains(&self, minute: u16) -> bool {
        if self.start_minute <= self.end_minute {
            self.start_minute <= minute && minute < self.end_minute
        } else {
            minute >= self.start_minute || minute < self.end_minute
        }
    }
}

/// A CLI proxy (CLIProxyAPI) that Claude Code goes through instead of the signed-in account, so runs
/// share the proxy's pool of accounts.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ProxySettings {
    /// Its address, e.g. `http://127.0.0.1:8317`. Empty = no proxy.
    pub url: String,
    /// One of its API keys, which Claude Code sends in an `x-api-key` header.
    pub api_key: String,
    /// Its management key, to read each account's usage limits. Empty = none are shown.
    pub management_key: String,
}

impl ProxySettings {
    /// The variables that send Claude Code through the proxy; none when there is no proxy. The key
    /// goes in a header because ANTHROPIC_AUTH_TOKEN would replace the claude.ai sign-in, and
    /// Claude in Chrome and claude.ai connectors only load with that sign-in.
    pub fn env(&self) -> Vec<(String, String)> {
        if self.url.is_empty() {
            return vec![];
        }
        let mut env = vec![("ANTHROPIC_BASE_URL".to_string(), self.url.clone())];
        if !self.api_key.is_empty() {
            env.push(("ANTHROPIC_CUSTOM_HEADERS".into(), format!("x-api-key: {}", self.api_key)));
        }
        env
    }
}

/// How OmegaStar watches requests to people for replies.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct WatchSettings {
    pub hours: BusinessHours,
    pub frequency: CheckFrequency,
    /// Business days until the automatic nudge when the ask has no due date. 0 = never nudge. Default 1. Max 10.
    pub nudge_after_days: u32,
    /// Business days until giving up, counted from max(start, due date, acknowledgement). Default 3. 1..=30,
    /// and greater than nudge_after_days when nudging is on.
    pub give_up_after_days: u32,
}

impl Default for WatchSettings {
    fn default() -> Self {
        WatchSettings { hours: BusinessHours::default(), frequency: CheckFrequency::Normal, nudge_after_days: 1, give_up_after_days: 3 }
    }
}

impl WatchSettings {
    /// Sorts the days and checks every field, as `save_settings` requires.
    pub fn validate(&mut self) -> Result<()> {
        let h = &mut self.hours;
        h.days.sort_unstable();
        h.days.dedup();
        if h.end_minute > 1440 || h.start_minute >= h.end_minute {
            return Err(AppError::invalid("business hours must start before they end, within the day"));
        }
        if h.days.is_empty() || h.days.iter().any(|d| !(1..=7).contains(d)) {
            return Err(AppError::invalid("pick at least one business day (1 = Monday to 7 = Sunday)"));
        }
        if self.nudge_after_days > 10 {
            return Err(AppError::invalid("nudge after must be 0 to 10 business days"));
        }
        if !(1..=30).contains(&self.give_up_after_days) {
            return Err(AppError::invalid("give up after must be 1 to 30 business days"));
        }
        if self.nudge_after_days > 0 && self.give_up_after_days <= self.nudge_after_days {
            return Err(AppError::invalid("give up after must be later than nudge after"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct BusinessHours {
    /// Local minutes since midnight.
    pub start_minute: u16,
    pub end_minute: u16,
    /// ISO weekdays, 1 = Monday to 7 = Sunday.
    pub days: Vec<u8>,
}

impl Default for BusinessHours {
    fn default() -> Self {
        BusinessHours { start_minute: 540, end_minute: 1020, days: vec![1, 2, 3, 4, 5] }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CheckFrequency {
    Eager,
    #[default]
    Normal,
    Relaxed,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BackupFrequency {
    #[default]
    Off,
    Hourly,
    Daily,
    Weekly,
}

impl BackupFrequency {
    pub fn interval(self) -> Option<chrono::Duration> {
        match self {
            BackupFrequency::Off => None,
            BackupFrequency::Hourly => Some(chrono::Duration::hours(1)),
            BackupFrequency::Daily => Some(chrono::Duration::days(1)),
            BackupFrequency::Weekly => Some(chrono::Duration::weeks(1)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct BackupSettings {
    pub frequency: BackupFrequency,
    /// Where backup zips go. Never inside the office folder.
    pub dir: String,
    /// How many backups to keep; older ones are deleted after each successful backup.
    pub keep: u32,
}

/// The backup folder in `base`: the one an older release made under the old name when it is there, else
/// "OmegaStar Backups".
fn default_backup_dir(base: &std::path::Path) -> std::path::PathBuf {
    let old = base.join(format!("{} Backups", crate::OLD_PRODUCT_NAME));
    if old.exists() { old } else { base.join("OmegaStar Backups") }
}

impl Default for BackupSettings {
    fn default() -> Self {
        let dir = default_backup_dir(&dirs::document_dir().or_else(dirs::home_dir).unwrap_or_default());
        BackupSettings { frequency: BackupFrequency::Off, dir: dir.to_string_lossy().into_owned(), keep: 7 }
    }
}

/// The last backup attempt, stored in `meta`; `running` and `next_at` are filled in when read.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct BackupStatus {
    pub running: bool,
    /// The last successful backup.
    pub last_at: Option<Ts>,
    pub last_file: Option<String>,
    pub last_bytes: Option<u64>,
    /// Files in the last successful backup that couldn't be read.
    pub skipped: u32,
    pub last_attempt_at: Option<Ts>,
    /// Set when the last attempt failed; cleared by the next success.
    pub last_error: Option<String>,
    pub next_at: Option<Ts>,
    /// backup::fingerprint() of the office at the last successful backup.
    pub fingerprint: Option<String>,
    /// The last time an automatic backup was due but nothing had changed.
    pub checked_at: Option<Ts>,
    /// Installers and build folders the last successful backup left out.
    pub left_out: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    /// 0-100.
    pub percent: f64,
    pub resets_at: Option<Ts>,
    /// The CLI's own wording, e.g. "Sep 28, 2:59pm (Europe/London)".
    pub resets_text: Option<String>,
}

/// Claude subscription usage limits: the 5-hour session window and the 7-day window.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageLimits {
    pub five_hour: Option<UsageWindow>,
    pub seven_day: Option<UsageWindow>,
    pub observed_at: Option<Ts>,
    /// `false` when the account has no subscription limits (API key billing).
    pub subscription: bool,
    /// The office waits until then for the usage limit to reset, or for a rate limit to ease: no run starts.
    pub held_until: Option<Ts>,
    /// The hold is a rate limit's, which only its own time ends: the usage windows say nothing about it.
    pub held_for_rate_limit: bool,
    /// Claude Code is signed out or on another account than the one set up: routines wait.
    pub account_issue: Option<AccountIssue>,
    /// Why the CLI proxy could not report its accounts' usage, when it could not.
    pub proxy_issue: Option<String>,
    /// Each of a CLI proxy's accounts, whose readings the windows above average. Empty without a proxy.
    pub accounts: Vec<AccountUsage>,
}

/// One CLI proxy account's own usage windows.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountUsage {
    /// The account's email, or the proxy's label for it.
    pub name: String,
    pub five_hour: Option<UsageWindow>,
    pub seven_day: Option<UsageWindow>,
    /// Weekly limits on one model, such as Fable, beside the 7-day limit on all of them.
    pub model_weeks: Vec<ModelWeek>,
}

/// A weekly limit on one model.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelWeek {
    /// The model's display name, e.g. "Fable".
    pub model: String,
    pub window: UsageWindow,
}

impl Default for UsageLimits {
    fn default() -> Self {
        // Unknown until the first probe; assume a subscription so the UI waits rather than switching to spend.
        UsageLimits { five_hour: None, seven_day: None, observed_at: None, subscription: true, held_until: None, held_for_rate_limit: false, account_issue: None, proxy_issue: None, accounts: vec![] }
    }
}

/// Why routines are paused for the account, and the account Claude Code is signed in to instead.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountIssue {
    pub message: String,
    pub signed_in_as: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backups_keep_going_to_a_folder_an_older_release_made() {
        let base = std::env::temp_dir().join(format!("omegastar-docs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        assert_eq!(default_backup_dir(&base), base.join("OmegaStar Backups"));
        std::fs::create_dir(base.join("Omega Star Backups")).unwrap();
        assert_eq!(default_backup_dir(&base), base.join("Omega Star Backups"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_proxy_key_goes_in_a_header_so_runs_keep_the_claude_ai_sign_in() {
        let proxy = ProxySettings { url: "http://127.0.0.1:8317".into(), api_key: "k1".into(), management_key: String::new() };
        let env = |vars: &[(&str, &str)]| vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
        assert_eq!(proxy.env(), env(&[("ANTHROPIC_BASE_URL", "http://127.0.0.1:8317"), ("ANTHROPIC_CUSTOM_HEADERS", "x-api-key: k1")]));
        let keyless = ProxySettings { api_key: String::new(), ..proxy };
        assert_eq!(keyless.env(), env(&[("ANTHROPIC_BASE_URL", "http://127.0.0.1:8317")]));
        assert!(ProxySettings::default().env().is_empty());
    }
}
