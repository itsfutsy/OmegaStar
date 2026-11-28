//! The usage limits of a CLI proxy's (CLIProxyAPI) Claude accounts, read through its management API.

use crate::claude::UsageSample;
use crate::model::{AccountUsage, ModelWeek, ProxySettings, UsageWindow};
use serde_json::{json, Value};
use std::time::Duration;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

const NO_MANAGEMENT_KEY: &str = "Add the CLI proxy's management key in Settings to see its accounts' usage limits.";

/// Why the proxy could not report usage.
#[derive(Debug)]
pub struct Failure {
    pub message: String,
    /// It turned the management key down: asking again with the same key only brings on its ban.
    pub refused: bool,
}

fn failure(message: impl Into<String>) -> Failure {
    Failure { message: message.into(), refused: false }
}

/// An HTTP client. reqwest's TLS comes with no crypto provider of its own (the updater's features
/// choose it), so one is installed first, as the updater does: without it, building a client panics.
pub(crate) fn client() -> reqwest::Client {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    reqwest::Client::new()
}

/// A management API call and its JSON answer.
async fn management(client: &reqwest::Client, p: &ProxySettings, path: &str, body: Option<Value>) -> Result<Value, Failure> {
    let url = format!("{}/v0/management/{path}", p.url);
    let req = match body {
        Some(b) => client.post(url).json(&b),
        None => client.get(url),
    };
    let res = req
        .bearer_auth(&p.management_key)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|_| failure(format!("OmegaStar could not reach the CLI proxy at {}.", p.url)))?;
    let status = res.status();
    let answer: Value = res.json().await.unwrap_or(Value::Null);
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        let why = answer["error"].as_str().unwrap_or("no reason given");
        return Err(Failure { message: format!("The CLI proxy refused the management key ({why}). Check it in Settings."), refused: true });
    }
    if !status.is_success() {
        return Err(failure(format!("The CLI proxy answered HTTP {}.", status.as_u16())));
    }
    Ok(answer)
}

/// The pool's usage, as if it were one account, and each account's own. An empty sample when there is no Claude account,
/// so the office has no limits to wait for. Without a management key nothing can be read: that is a failure, so the
/// sidebar says how to show the limits instead of taking the pool for an API-key account with none.
pub async fn usage(p: &ProxySettings) -> Result<UsageSample, Failure> {
    if p.management_key.is_empty() {
        return Err(failure(NO_MANAGEMENT_KEY));
    }
    let client = client();
    let files = management(&client, p, "auth-files", None).await?;
    let claude: Vec<&Value> = files["files"]
        .as_array()
        .ok_or_else(|| failure("The CLI proxy did not list its accounts."))?
        .iter()
        .filter(|f| f["provider"] == "claude" && f["disabled"] != true)
        .collect();
    let mut accounts = vec![];
    for f in &claude {
        // The proxy fills in the account's own token for `$TOKEN$`.
        let call = json!({
            "auth_index": f["auth_index"],
            "method": "GET",
            "url": USAGE_URL,
            "header": { "Authorization": "Bearer $TOKEN$", "anthropic-beta": "oauth-2025-04-20" },
        });
        let res = match management(&client, p, "api-call", Some(call)).await {
            Ok(res) => res,
            Err(f) if f.refused => return Err(f),
            Err(_) => continue,
        };
        if !(200..300).contains(&res["status_code"].as_u64().unwrap_or(0)) {
            continue;
        }
        if let Ok(u) = serde_json::from_str::<Value>(res["body"].as_str().unwrap_or("")) {
            accounts.push(account(f, &u));
        }
    }
    if accounts.is_empty() && !claude.is_empty() {
        return Err(failure("The CLI proxy could not read its Claude accounts' usage."));
    }
    Ok(pool(accounts))
}

/// A usage window from the usage API, its use as 0-100 under `key` (`utilization` on a window, `percent` in `limits`).
fn window(w: &Value, key: &str) -> Option<UsageWindow> {
    let resets_at = w["resets_at"].as_str().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|t| t.to_utc());
    Some(UsageWindow { percent: w[key].as_f64()?.clamp(0.0, 100.0), resets_at, resets_text: None })
}

/// An account's usage: its auth file names it, the usage API's answer `u` holds its windows. A weekly limit on one
/// model is only in `limits`, as `weekly_scoped` with the model in its scope.
fn account(file: &Value, u: &Value) -> AccountUsage {
    let name = ["email", "label", "name"].iter().filter_map(|k| file[k].as_str()).find(|n| !n.is_empty()).unwrap_or("Claude account");
    let model_weeks = u["limits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|l| l["kind"] == "weekly_scoped")
        .filter_map(|l| {
            Some(ModelWeek { model: l["scope"]["model"]["display_name"].as_str()?.to_string(), window: window(l, "percent")? })
        })
        .collect();
    AccountUsage { name: name.to_string(), five_hour: window(&u["five_hour"], "utilization"), seven_day: window(&u["seven_day"], "utilization"), model_weeks }
}

/// Each window's use averaged over the accounts, so it is full only when every account is, and
/// resetting when the first of them does.
fn pool(accounts: Vec<AccountUsage>) -> UsageSample {
    let average = |pick: fn(&AccountUsage) -> &Option<UsageWindow>| {
        let seen: Vec<&UsageWindow> = accounts.iter().filter_map(|a| pick(a).as_ref()).collect();
        if seen.is_empty() {
            return None;
        }
        let percent = seen.iter().map(|w| w.percent).sum::<f64>() / seen.len() as f64;
        Some(UsageWindow { percent, resets_at: seen.iter().filter_map(|w| w.resets_at).min(), resets_text: None })
    };
    UsageSample { five_hour: average(|a| &a.five_hour), seven_day: average(|a| &a.seven_day), accounts }
}

#[cfg(test)]
/// A proxy on a free port that knows the management key `right-key` and has two enabled Claude
/// accounts at 90% and 30% of their 5-hour window, a disabled one and a Codex one.
pub(crate) async fn fake_proxy() -> String {
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{get, post};
    use axum::Json;
    let files = |h: HeaderMap| async move {
        if h.get("authorization").is_none_or(|v| v != "Bearer right-key") {
            return (StatusCode::UNAUTHORIZED, Json(json!({ "error": "invalid management key" })));
        }
        let files = json!([
            { "auth_index": "a", "provider": "claude", "email": "a@example.com", "label": "a label" },
            { "auth_index": "b", "provider": "claude", "disabled": true },
            { "auth_index": "c", "provider": "codex" },
            { "auth_index": "d", "provider": "claude", "email": "", "label": "d label" },
        ]);
        (StatusCode::OK, Json(json!({ "files": files })))
    };
    let call = |Json(c): Json<Value>| async move {
        if c["url"] != USAGE_URL || c["header"]["Authorization"] != "Bearer $TOKEN$" {
            return Json(json!({ "status_code": 400, "body": "" }));
        }
        let used = if c["auth_index"] == "a" { 90.0 } else { 30.0 };
        Json(json!({ "status_code": 200, "body": json!({ "five_hour": { "utilization": used, "resets_at": null } }).to_string() }))
    };
    let app = axum::Router::new().route("/v0/management/auth-files", get(files)).route("/v0/management/api-call", post(call));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pool_is_full_only_when_every_account_is() {
        let file = json!({ "email": "a@example.com" });
        let accounts = vec![
            account(&file, &json!({ "five_hour": { "utilization": 100.0, "resets_at": "2030-01-01T12:00:00+00:00" }, "seven_day": { "utilization": 40.0, "resets_at": null } })),
            account(&file, &json!({ "five_hour": { "utilization": 20.0, "resets_at": "2030-01-01T10:00:00+00:00" }, "seven_day": null })),
        ];
        let s = pool(accounts);
        let five = s.five_hour.unwrap();
        assert_eq!(five.percent, 60.0);
        assert_eq!(five.resets_at.unwrap().to_rfc3339(), "2030-01-01T10:00:00+00:00");
        let week = s.seven_day.unwrap();
        assert_eq!((week.percent, week.resets_at), (40.0, None), "an account without the window does not count");
        assert_eq!(pool(vec![]), UsageSample::default());
        assert_eq!(s.accounts.len(), 2, "each account is kept as well");
    }

    #[test]
    fn a_weekly_limit_on_one_model_is_kept_by_its_name() {
        let u = json!({
            "five_hour": { "utilization": 50.0, "resets_at": null },
            "limits": [
                { "kind": "session", "percent": 50, "resets_at": null, "scope": null },
                { "kind": "weekly_scoped", "percent": 95, "resets_at": "2030-01-05T20:59:59.7+00:00", "scope": { "model": { "id": null, "display_name": "Fable" } } },
                { "kind": "weekly_scoped", "percent": 10, "resets_at": null, "scope": { "model": null, "surface": "x" } },
            ],
        });
        let a = account(&json!({ "label": "spare" }), &u);
        assert_eq!(a.name, "spare");
        assert_eq!(a.model_weeks.len(), 1, "a scope that names no model is not a model's limit");
        assert_eq!((a.model_weeks[0].model.as_str(), a.model_weeks[0].window.percent), ("Fable", 95.0));
        assert!(a.model_weeks[0].window.resets_at.is_some());
        assert_eq!(a.seven_day, None);
    }

    #[tokio::test]
    async fn usage_comes_from_enabled_claude_accounts_and_a_wrong_key_is_refused() {
        let url = fake_proxy().await;
        let proxy = |url: &str, key: &str| ProxySettings { url: url.into(), api_key: "api-key".into(), management_key: key.into() };
        let sample = usage(&proxy(&url, "right-key")).await.unwrap();
        assert_eq!(sample.five_hour.map(|w| w.percent), Some(60.0));
        let each: Vec<(String, Option<f64>)> = sample.accounts.iter().map(|a| (a.name.clone(), a.five_hour.as_ref().map(|w| w.percent))).collect();
        assert_eq!(each, vec![("a@example.com".into(), Some(90.0)), ("d label".into(), Some(30.0))], "named by email, else by label");
        let wrong = usage(&proxy(&url, "api-key")).await.unwrap_err();
        assert!(wrong.refused && wrong.message.contains("invalid management key"), "{}", wrong.message);
        let away = usage(&proxy("http://127.0.0.1:1", "right-key")).await.unwrap_err();
        assert!(!away.refused, "a proxy that is down is asked again later");
        let keyless = usage(&proxy(&url, "")).await.unwrap_err();
        assert_eq!((keyless.message.as_str(), keyless.refused), (NO_MANAGEMENT_KEY, false), "no key says how to show the limits");
    }
}
