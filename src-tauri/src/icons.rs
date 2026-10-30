//! Site icons for favorites: a site's app icon (`apple-touch-icon`, else the
//! largest `<link rel="icon">`), fetched once per origin and kept for the
//! run as a data URL. Without one the tile falls back to `/favicon.ico`.

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::Duration,
};

use tauri::{AppHandle, Manager, Url};

/// Icons by origin; `None` when the site has none we can use. Sites that
/// couldn't be reached aren't remembered, so they're tried again.
#[derive(Default)]
pub struct Icons(Mutex<HashMap<String, Option<String>>>);

/// Bytes read from a page looking for its icon links (they're in `<head>`).
const MAX_HTML: usize = 512 * 1024;
/// Largest icon accepted.
const MAX_ICON: usize = 512 * 1024;

#[tauri::command]
pub async fn site_icon(app: AppHandle, url: String) -> Option<String> {
    let url = Url::parse(&url)
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https"))?;
    let origin = url.origin().ascii_serialization();
    if let Some(icon) = app.state::<Icons>().0.lock().unwrap().get(&origin) {
        return icon.clone();
    }
    let fetched = fetch(&url).await?;
    app.state::<Icons>()
        .0
        .lock()
        .unwrap()
        .insert(origin, fetched.clone());
    fetched
}

/// One client for every lookup (it pools connections).
fn client() -> Option<&'static reqwest::Client> {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            // reqwest is built without a TLS provider of its own (as the updater does).
            if rustls::crypto::CryptoProvider::get_default().is_none() {
                let _ = rustls::crypto::ring::default_provider().install_default();
            }
            reqwest::ClientBuilder::new()
                .user_agent(crate::browser::USER_AGENT)
                .timeout(Duration::from_secs(8))
                .build()
                .ok()
        })
        .as_ref()
}

/// The site's icon, `Some(None)` when it has none, `None` when it couldn't
/// be reached.
async fn fetch(url: &Url) -> Option<Option<String>> {
    let client = client()?;
    // The conventional location first: right host, no page to parse.
    if let Ok(touch) = url.join("/apple-touch-icon.png")
        && let Some(icon) = image(client, touch).await
    {
        return Some(Some(icon));
    }
    let (base, _, html) = get(client, url.clone(), MAX_HTML).await?;
    let html = String::from_utf8_lossy(&html);
    for href in icon_links(&html) {
        if let Ok(href) = base.join(&href)
            && let Some(icon) = image(client, href).await
        {
            return Some(Some(icon));
        }
    }
    Some(None)
}

/// GET `url`, reading at most `limit` bytes: the final URL (after
/// redirects), the content type and the body.
async fn get(client: &reqwest::Client, url: Url, limit: usize) -> Option<(Url, String, Vec<u8>)> {
    let mut response = client.get(url).send().await.ok()?.error_for_status().ok()?;
    let base = response.url().clone();
    let kind = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        body.extend_from_slice(&chunk);
        if body.len() >= limit {
            break;
        }
    }
    Some((base, kind, body))
}

/// An image as a data URL, if `url` serves one of a sensible size.
async fn image(client: &reqwest::Client, url: Url) -> Option<String> {
    let (_, kind, body) = get(client, url, MAX_ICON + 1).await?;
    if !kind.starts_with("image/") || body.is_empty() || body.len() > MAX_ICON {
        return None;
    }
    let kind = kind.split(';').next().unwrap_or(&kind).trim();
    Some(format!("data:{kind};base64,{}", base64(&body)))
}

/// The page's icon links, best first: app icons (`apple-touch-icon`), then
/// icons by size, larger first.
fn icon_links(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut found: Vec<(u32, String)> = Vec::new();
    let mut from = 0;
    while let Some(start) = lower[from..].find("<link").map(|i| from + i) {
        let end = lower[start..].find('>').map_or(lower.len(), |i| start + i);
        let tag = &html[start..end];
        from = end;
        let (Some(rel), Some(href)) = (attr(tag, "rel"), attr(tag, "href")) else {
            continue;
        };
        let rel = rel.to_ascii_lowercase();
        let tokens: Vec<&str> = rel.split_whitespace().collect();
        let size = attr(tag, "sizes")
            .and_then(|s| {
                s.split(['x', 'X'])
                    .next()
                    .and_then(|n| n.trim().parse::<u32>().ok())
            })
            .unwrap_or(if href.ends_with(".svg") { 256 } else { 16 });
        let score = if tokens.iter().any(|t| t.starts_with("apple-touch-icon")) {
            10_000 + size
        } else if tokens.contains(&"icon") {
            size
        } else {
            continue;
        };
        found.push((score, href.replace("&amp;", "&")));
    }
    found.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    found.into_iter().map(|(_, href)| href).collect()
}

/// An attribute's value in a tag (quoted or not), matched case-insensitively.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name).map(|i| from + i) {
        from = i + name.len();
        let before = lower[..i].chars().next_back();
        let rest = lower[from..].trim_start();
        if !before.is_some_and(char::is_whitespace) || !rest.starts_with('=') {
            continue;
        }
        let value = tag[tag.len() - rest.len() + 1..].trim_start();
        return Some(match value.chars().next()? {
            q @ ('"' | '\'') => value[1..].split(q).next()?.to_string(),
            _ => value.split_whitespace().next()?.to_string(),
        });
    }
    None
}

fn base64(bytes: &[u8]) -> String {
    const CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = u32::from(chunk[0]) << 16
            | u32::from(*chunk.get(1).unwrap_or(&0)) << 8
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            out.push(if i <= chunk.len() {
                CHARS[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_icons_come_first_then_larger_icons() {
        let html = r#"<head>
            <link rel="icon" href="/favicon-16.png" sizes="16x16">
            <LINK REL='shortcut icon' HREF='/fav.ico'>
            <link rel="mask-icon" href="/mask.svg">
            <link href="/icon-192.png?a=1&amp;b=2" rel="icon" sizes="192x192" />
            <link rel="apple-touch-icon" href=/touch.png>
            <link rel="stylesheet" href="/x.css">
        </head>"#;
        assert_eq!(
            icon_links(html),
            [
                "/touch.png",
                "/icon-192.png?a=1&b=2",
                "/favicon-16.png",
                "/fav.ico"
            ]
        );
    }

    #[test]
    fn attributes_must_be_whole_names() {
        assert_eq!(
            attr(r#"<link data-rel="x" rel="icon""#, "rel").as_deref(),
            Some("icon")
        );
        assert_eq!(attr("<link rel = icon", "rel").as_deref(), Some("icon"));
    }

    #[test]
    fn base64_pads() {
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"Man"), "TWFu");
    }
}
