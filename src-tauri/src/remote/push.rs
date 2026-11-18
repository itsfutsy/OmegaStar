//! Phone pushes through the relay's `POST /v1/push` (section 10 of `docs/mobile-companion.md`). Only the title leaves
//! the computer; the relay adds the body. A push that fails is logged and dropped: there is no retry queue.

use super::store::PushTarget;
use crate::log::log;
use serde_json::json;
use std::time::Duration;

/// The longest title a push carries, in characters.
pub const MAX_TITLE: usize = 120;

/// The title as sent: cut to `MAX_TITLE` characters.
fn title_of(title: &str) -> String {
    title.chars().take(MAX_TITLE).collect()
}

/// Sends `title` to `target` through the relay's push route at `url`. True when the relay says the token no longer
/// works, so it is forgotten.
pub async fn send(client: &reqwest::Client, url: &str, target: &PushTarget, title: &str) -> bool {
    let body = json!({ "platform": target.platform, "token": target.token, "sandbox": target.sandbox, "title": title_of(title) });
    let status = match client.post(url).json(&body).timeout(Duration::from_secs(15)).send().await {
        Ok(res) => res.status(),
        Err(e) => {
            log!("phone push failed: {}", e.without_url());
            return false;
        }
    };
    match status.as_u16() {
        200..=299 => false,
        410 => true,
        code => {
            log!("phone push dropped: the relay answered HTTP {code}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A push route on this computer that answers one request with `status`.
    async fn answering(status: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let _ = s.read(&mut [0; 4096]).await;
            let _ = s.write_all(format!("HTTP/1.1 {status}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").as_bytes()).await;
        });
        format!("http://{addr}/v1/push")
    }

    #[tokio::test]
    async fn only_a_410_forgets_the_token() {
        let client = crate::proxy::client();
        let target = PushTarget { platform: "ios".into(), token: "t1".into(), sandbox: false };
        for (status, gone) in [("204 No Content", false), ("410 Gone", true), ("429 Too Many Requests", false), ("500 Internal Server Error", false)] {
            assert_eq!(send(&client, &answering(status).await, &target, "Ada needs you").await, gone, "{status}");
        }
    }

    #[test]
    fn titles_are_cut_to_120_characters() {
        assert_eq!(title_of("Ada needs you"), "Ada needs you");
        let long = "é".repeat(200);
        assert_eq!(title_of(&long).chars().count(), MAX_TITLE);
    }
}
