//! Site icons for favorites: a site's app icon (`apple-touch-icon`, else the
//! largest `<link rel="icon">`), fetched once per origin and kept for the
//! run as a data URL. Without one the tile falls back to `/favicon.ico`.
//!
//! Icon links come from the favorite's live page when it's loaded: fetched
//! here without the user's cookies, many sites show a sign-in page (Gmail),
//! refuse (TradingView) or add their icons with scripts (single-page apps).
//! Otherwise from the page's HTML, then the site's home page.

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

/// `tab` is the favorite showing `url`, whose page is asked for its icon
/// links if it's loaded.
#[tauri::command]
pub async fn site_icon(app: AppHandle, url: String, tab: Option<u32>) -> Option<String> {
    let url = Url::parse(&url)
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https"))?;
    let origin = url.origin().ascii_serialization();
    if let Some(icon) = app.state::<Icons>().0.lock().unwrap().get(&origin) {
        return icon.clone();
    }
    let page = match tab {
        Some(id) => crate::browser::page_icon_links(&app, id).await,
        None => None,
    };
    let fetched = fetch(&url, page.as_deref()).await?;
    // "No icon" is only final once the live page was asked: an unloaded
    // favorite is asked again when its page loads.
    if fetched.is_some() || page.is_some() {
        app.state::<Icons>()
            .0
            .lock()
            .unwrap()
            .insert(origin, fetched.clone());
    }
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
/// be reached. `page` holds the live page's icon links, if known.
async fn fetch(url: &Url, page: Option<&str>) -> Option<Option<String>> {
    let client = client()?;
    if let Some(page) = page {
        for href in icon_links(page) {
            if let Ok(href) = Url::parse(&href)
                && let Some(icon) = image(client, href).await
            {
                return Some(Some(icon));
            }
        }
    }
    // The conventional location: right host, no page to parse.
    if let Ok(touch) = url.join("/apple-touch-icon.png")
        && let Some(icon) = image(client, touch).await
    {
        return Some(Some(icon));
    }
    // The page's HTML, then the home page's (a deep link may need signing
    // in, or be refused).
    let mut reached = page.is_some();
    let home = url.join("/").ok().filter(|home| home != url);
    for url in std::iter::once(url.clone()).chain(home) {
        let Some((base, _, html)) = get(client, url, MAX_HTML).await else {
            continue;
        };
        reached = true;
        let html = String::from_utf8_lossy(&html);
        for href in icon_links(&html) {
            if let Ok(href) = base.join(&href)
                && let Some(icon) = image(client, href).await
            {
                return Some(Some(icon));
            }
        }
    }
    reached.then_some(None)
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

/// Longest `data:` icon kept as a tab's favicon (it's sent with every state
/// update and saved in the session).
const MAX_DATA_FAVICON: usize = 8 * 1024;

/// A tab's favicon from its page's icon links (absolute hrefs, as
/// `native::page_icon_links` gives them), for a 16 px tile: an SVG, else the
/// smallest icon at least 32 px (sharp on Retina), else the largest; an app
/// icon only if there's nothing else. `None`: the page names none, so the
/// sidebar tries `/favicon.ico`.
pub fn favicon_from_links(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    // (rank, href): lower is better.
    let mut best: Option<(u32, String)> = None;
    let mut from = 0;
    while let Some(start) = lower[from..].find("<link").map(|i| from + i) {
        let end = lower[start..].find('>').map_or(lower.len(), |i| start + i);
        let tag = &html[start..end];
        from = end;
        let (Some(rel), Some(href)) = (attr(tag, "rel"), attr(tag, "href")) else {
            continue;
        };
        let href = href.replace("&amp;", "&");
        let usable = href.starts_with("https://")
            || href.starts_with("http://")
            || (href.starts_with("data:image/") && href.len() <= MAX_DATA_FAVICON);
        if !usable {
            continue;
        }
        let rel = rel.to_ascii_lowercase();
        let tokens: Vec<&str> = rel.split_whitespace().collect();
        let size = attr(tag, "sizes").and_then(|s| {
            s.split(['x', 'X'])
                .next()
                .and_then(|n| n.trim().parse::<u32>().ok())
        });
        let svg = href.split(['?', '#']).next().is_some_and(|p| p.ends_with(".svg"))
            || href.starts_with("data:image/svg");
        let rank = if tokens.contains(&"icon") {
            match (svg, size) {
                (true, _) => 0,
                // 32 px and up, smallest first.
                (false, Some(n)) if n >= 32 => n,
                // No size given: usually a 32 px favicon.
                (false, None) => 2_000,
                // Smaller than 32: larger first.
                (false, Some(n)) => 3_000 - n.min(31),
            }
        } else if tokens.iter().any(|t| t.starts_with("apple-touch-icon")) {
            5_000
        } else {
            continue;
        };
        if best.as_ref().is_none_or(|(r, _)| rank < *r) {
            best = Some((rank, href));
        }
    }
    best.map(|(_, href)| href)
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
    fn favicons_prefer_svg_then_the_smallest_sharp_icon() {
        let links = |tags: &[&str]| tags.concat();
        let touch = r#"<link rel="apple-touch-icon" href="https://a.com/touch.png" sizes="">"#;
        let ico = r#"<link rel="shortcut icon" href="https://a.com/favicon.ico" sizes="">"#;
        let i16 = r#"<link rel="icon" href="https://a.com/16.png" sizes="16x16">"#;
        let i32 = r#"<link rel="icon" href="https://a.com/32.png" sizes="32x32">"#;
        let i192 = r#"<link rel="icon" href="https://a.com/192.png" sizes="192x192">"#;
        let svg = r#"<link rel="icon" href="https://a.com/icon.svg?v=2" sizes="">"#;
        let mask = r#"<link rel="mask-icon" href="https://a.com/mask.svg" sizes="">"#;
        assert_eq!(favicon_from_links(&links(&[touch, i16, i192, i32])).as_deref(), Some("https://a.com/32.png"));
        assert_eq!(favicon_from_links(&links(&[i192, svg, i32])).as_deref(), Some("https://a.com/icon.svg?v=2"));
        assert_eq!(favicon_from_links(&links(&[i16, ico])).as_deref(), Some("https://a.com/favicon.ico"));
        assert_eq!(favicon_from_links(&links(&[touch, i16])).as_deref(), Some("https://a.com/16.png"));
        assert_eq!(favicon_from_links(&links(&[mask, touch])).as_deref(), Some("https://a.com/touch.png"));
        assert_eq!(favicon_from_links(mask), None);
        assert_eq!(favicon_from_links(""), None);
        let big = format!(r#"<link rel="icon" href="data:image/png;base64,{}" sizes="">"#, "A".repeat(9000));
        assert_eq!(favicon_from_links(&big), None, "too big to send with every update");
        let small = r#"<link rel="icon" href="data:image/svg+xml,%3Csvg/%3E" sizes="">"#;
        assert_eq!(favicon_from_links(small).as_deref(), Some("data:image/svg+xml,%3Csvg/%3E"));
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
