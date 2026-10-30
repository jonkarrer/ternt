//! The welcome page: `ternt://welcome`, a plain web tab served from a custom
//! URI scheme, so it needs no IPC and shows in every webview. Opened on the
//! very first launch (no session file yet) and from Help → Welcome to Ternt.

use tauri::{AppHandle, UriSchemeContext, UriSchemeResponder, http::Request};

pub const SCHEME: &str = "ternt";
pub const URL: &str = "ternt://welcome/";

const PAGE: &str = include_str!("../welcome.html");

/// A URL this scheme serves.
pub fn is_welcome(url: &str) -> bool {
    url.trim_end_matches('/') == URL.trim_end_matches('/')
}

pub fn respond(_ctx: UriSchemeContext<'_, tauri::Wry>, request: Request<Vec<u8>>, responder: UriSchemeResponder) {
    let (status, body, kind) = if is_welcome(&request.uri().to_string()) {
        (200, PAGE.as_bytes().to_vec(), "text/html; charset=utf-8")
    } else {
        (404, b"not found".to_vec(), "text/plain")
    };
    let response = tauri::http::Response::builder()
        .status(status)
        .header("Content-Type", kind)
        .body(body)
        .expect("static response");
    responder.respond(response);
}

/// Open the welcome page in a new tab. Must not be called from the main
/// thread (see `browser::open_tab`).
pub fn open(app: &AppHandle) -> tauri::Result<()> {
    crate::browser::open_tab(app, URL.parse().expect("constant url")).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_its_url_with_or_without_slash() {
        assert!(is_welcome("ternt://welcome"));
        assert!(is_welcome("ternt://welcome/"));
        assert!(!is_welcome("ternt://other"));
        assert!(!is_welcome("https://welcome/"));
    }

    #[test]
    fn page_is_self_contained_html() {
        assert!(PAGE.starts_with("<!doctype html>"));
        assert!(PAGE.contains("<title>Welcome to Ternt</title>"));
        assert!(!PAGE.contains("<script"), "no scripts: tab webviews have no IPC");
        assert!(!PAGE.contains("src=\"http"), "no remote assets: the page must work offline");
    }
}
