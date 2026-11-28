//! Pages an employee shows in a message (`show`): saved with the run, and served on the `omega-visual` scheme
//! to a sealed frame, in OmegaStar's look, with no way out to the network or the app. A click on a web link is
//! the one thing a page passes out: its address, for the app to open in your browser.

use super::*;
use base64::Engine;
use std::sync::OnceLock;

/// Largest page `show` takes, before its images are embedded.
const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_IMAGE_BYTES: u64 = 25 * 1024 * 1024;
const MAX_TITLE_CHARS: usize = 80;

/// What a page may do: run its own inline script and style, and show the images and fonts embedded in it.
/// Nothing it loads or sends leaves the machine.
pub const PAGE_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; \
font-src data:; media-src data: blob:; form-action 'none'; base-uri 'none'";

const TOKENS: &str = include_str!("../../../src/design/tokens.css");
/// Inter with both its weight and optical-size axes, so a page can use the Display cut (`--display`) as the app does.
const INTER: &[u8] = include_bytes!("../../../node_modules/@fontsource-variable/inter/files/inter-latin-opsz-normal.woff2");
const BASE: &str = "html { color: var(--ink); background: transparent; font-family: var(--font-ui); font-size: var(--t-base); \
line-height: 1.5; font-feature-settings: var(--font-features); -webkit-font-smoothing: antialiased; }\nbody { margin: 0; }";
/// Tells the frame how tall the page is, whenever that changes.
const HEIGHT: &str = "new ResizeObserver(() => parent.postMessage({ omegaHeight: \
Math.ceil(document.documentElement.getBoundingClientRect().height) }, \"*\")).observe(document.documentElement);";
/// Tells the frame which web link was clicked, since the sealed frame can open nothing itself: not a new window,
/// and not the link in its own place, which would only blank the page. A link into the page itself (on Windows the
/// page's own address is a web address too) and a click the page handled are left alone.
const LINKS: &str = "addEventListener(\"click\", (e) => { const a = e.target.closest?.(\"a[href]\"); \
const url = a && String(a.href.baseVal ?? a.href); \
if (url && /^https?:/.test(url) && !url.startsWith(location.origin + \"/\") && !e.defaultPrevented) { e.preventDefault(); \
parent.postMessage({ omegaOpen: url }, \"*\"); } });";

/// The MIME type of a local image, by its extension.
pub fn image_mime(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

/// The file at `path`, read through the one handle that was checked, or `None` when it is over `limit` bytes. It
/// must be a regular file: opened without waiting, a pipe put in its place is refused rather than read forever.
pub fn read_file(path: &Path, limit: u64) -> Result<Option<Vec<u8>>> {
    use std::io::Read;
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut opts, libc::O_NONBLOCK);
    let gone = || AppError::not_found(format!("Nothing is at {} any more.", path.display()));
    let file = opts.open(path).map_err(|_| gone())?;
    if !file.metadata()?.is_file() {
        return Err(gone());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    Ok((bytes.len() as u64 <= limit).then_some(bytes))
}

/// A local image, by its full path, as a data URL.
pub fn image_data_url(path: &Path) -> Result<String> {
    let shown = path.display();
    let mime = image_mime(path).filter(|_| path.is_absolute()).ok_or_else(|| AppError::invalid(format!("{shown} is not a local image")))?;
    let meta = fs::metadata(path).map_err(|_| AppError::not_found(format!("No image at {shown}")))?;
    if meta.len() > MAX_IMAGE_BYTES {
        return Err(AppError::invalid(format!("{shown} is larger than {} MB", MAX_IMAGE_BYTES / (1024 * 1024))));
    }
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(fs::read(path)?)))
}

impl Office {
    /// Saves the page under the run and puts it in the transcript, where the UI shows it above the message.
    pub fn show_tool(&self, caller_run: &str, args: &Value) -> Result<Value> {
        let title = claude::one_line(args.get("title").and_then(Value::as_str).unwrap_or("").trim());
        let html = args.get("html").and_then(Value::as_str).unwrap_or("");
        if title.is_empty() || html.trim().is_empty() {
            return Err(AppError::invalid("Give the page a title and its html."));
        }
        if title.chars().count() > MAX_TITLE_CHARS {
            return Err(AppError::invalid(format!("title is at most {MAX_TITLE_CHARS} characters.")));
        }
        if html.len() > MAX_PAGE_BYTES {
            return Err(AppError::invalid(format!("The page is larger than {} MB. Show less, or split it into two pages.", MAX_PAGE_BYTES / (1024 * 1024))));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let dir = self.run_dir(caller_run).join("visuals");
        fs::create_dir_all(&dir)
            .and_then(|_| fs::write(dir.join(format!("{id}.html")), inline_images(html)))
            .map_err(|e| AppError::internal(format!("Could not save the page: {e}")))?;
        self.append_event(caller_run, RunEvent::Visual { id, title });
        Ok(json!({ "shown": true, "note": "It appears above your message. Do not repeat in words what it shows." }))
    }

    /// A page as its frame loads it: OmegaStar's look and the height and link reporters first, then the page.
    pub fn visual_page(&self, run_id: &str, id: &str, dark: bool) -> Result<Vec<u8>> {
        let plain = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if !plain(run_id) || !plain(id) {
            return Err(AppError::not_found("No such page."));
        }
        let page = fs::read_to_string(self.run_dir(run_id).join("visuals").join(format!("{id}.html")))
            .map_err(|_| AppError::not_found("This page is no longer there."))?;
        Ok(format!("{}{page}", head(dark)).into_bytes())
    }
}

/// What goes before every page. A page's own `<html>`, `<head>` and styles after it still apply, and win.
fn head(dark: bool) -> String {
    static STYLE: OnceLock<String> = OnceLock::new();
    let style = STYLE.get_or_init(|| {
        let inter = base64::engine::general_purpose::STANDARD.encode(INTER);
        format!(
            "{TOKENS}\n@font-face {{ font-family: \"Inter Variable\"; font-style: normal; font-weight: 100 900; font-display: block; \
src: url(data:font/woff2;base64,{inter}) format(\"woff2\"); }}\n{BASE}"
        )
    });
    let theme = if dark { "dark" } else { "light" };
    format!("<!doctype html><html data-theme=\"{theme}\"><head><meta charset=\"utf-8\"><style>{style}</style><script>{HEIGHT}{LINKS}</script></head>")
}

/// The page with every local image it names by full path (`src="/Users/me/shot.png"`, `url(/Users/me/bg.webp)`,
/// or a quoted string in a script) embedded, so it needs nothing outside itself. A path that is not there stays.
fn inline_images(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(i) = rest.find(['"', '\'', '(']) {
        let close = match rest.as_bytes()[i] {
            b'(' => ')',
            q => q as char,
        };
        out.push_str(&rest[..=i]);
        rest = &rest[i + 1..];
        let end = rest.find([close, '\n']).filter(|&e| rest[e..].starts_with(close));
        if let Some((end, data)) = end.and_then(|e| embedded(&rest[..e]).map(|d| (e, d))) {
            out.push_str(&data);
            out.push(close);
            rest = &rest[end + 1..];
        }
    }
    out.push_str(rest);
    out
}

/// `s` as a data URL, when it is the full path of a local image.
fn embedded(s: &str) -> Option<String> {
    let path = Path::new(s.trim());
    (s.len() < 1024 && path.is_absolute() && image_mime(path).is_some()).then(|| image_data_url(path).ok()).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::temp_path;

    #[test]
    fn local_images_are_embedded_wherever_the_page_names_them() {
        let dir = temp_path("visuals");
        fs::create_dir_all(dir.join("with space")).unwrap();
        let shot = dir.join("with space").join("shot.png");
        fs::write(&shot, b"png").unwrap();
        let p = shot.display();
        let data = "data:image/png;base64,cG5n";
        let html = format!(
            "<img src=\"{p}\" alt=\"it's here\"><div style=\"background: url({p})\"></div><script>const s = '{p}';</script>\
<img src=\"{gone}\"><img src=\"https://example.com/a.png\"><p>Don't (touch) this.</p>",
            gone = dir.join("gone.png").display()
        );
        let out = inline_images(&html);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(out.matches(data).count(), 3, "src, url() and a script string: {out}");
        assert!(out.contains(&format!("<img src=\"{data}\" alt=\"it's here\">")), "{out}");
        assert!(out.contains(&format!("url({data})")), "{out}");
        assert!(out.contains("gone.png\"") && out.contains("https://example.com/a.png"), "a missing or remote image stays: {out}");
        assert!(out.ends_with("<p>Don't (touch) this.</p>"), "{out}");
    }

    #[test]
    fn a_page_gets_the_theme_and_the_height_and_link_reporters_first() {
        let h = head(true);
        assert!(h.starts_with("<!doctype html><html data-theme=\"dark\"><head>"), "{h}");
        assert!(h.contains("--ink:") && h.contains("font-family: \"Inter Variable\"") && h.contains("omegaHeight") && h.contains("omegaOpen"), "{h}");
        assert!(head(false).contains("data-theme=\"light\""));
    }
}
