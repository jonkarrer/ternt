//! The welcome page: `ternt://welcome`, a plain web tab served from a custom
//! URI scheme, so it needs no IPC and shows in every webview. Opened on the
//! very first launch (no session file yet) and from Help → Welcome to Ternt.

use std::sync::LazyLock;

use tauri::{AppHandle, Manager, UriSchemeContext, UriSchemeResponder, http::Request};

pub const SCHEME: &str = "ternt";
pub const URL: &str = "ternt://welcome/";
/// Ternt's license, and the notices of the software it includes (Help menu, the setup
/// window, Settings → Agent).
pub const LICENSE_URL: &str = "ternt://license/";
pub const THIRD_PARTY_URL: &str = "ternt://third-party/";

const PAGE: &str = include_str!("../welcome.html");
const LICENSE: &str = include_str!("../../LICENSE");
/// Written by `scripts/licenses.ts` (`just licenses`).
const THIRD_PARTY: &str = include_str!("../third-party-licenses.txt");

static LICENSE_PAGE: LazyLock<String> = LazyLock::new(|| {
    text_page(
        "Ternt License",
        "Ternt is also built with software from others: <a href=\"ternt://third-party/\">Third-Party Licenses</a>.",
        LICENSE,
    )
});
static THIRD_PARTY_PAGE: LazyLock<String> = LazyLock::new(|| {
    text_page(
        "Third-Party Licenses",
        "Ternt's own terms: <a href=\"ternt://license/\">Ternt License</a>.",
        THIRD_PARTY,
    )
});

/// A page showing `text` as written. `note` is our own HTML; `text` is escaped.
fn text_page(title: &str, note: &str, text: &str) -> String {
    let text = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>{title}</title>\
         <meta name=\"color-scheme\" content=\"light dark\"><style>\
         body{{margin:0;font:14px/1.5 -apple-system,sans-serif}}\
         main{{max-width:84ch;margin:0 auto;padding:40px 24px 64px}}\
         h1{{font-size:24px;margin:0 0 6px}}p{{margin:0 0 24px;opacity:.7}}\
         pre{{font:12.5px/1.5 ui-monospace,Menlo,monospace;white-space:pre-wrap;overflow-wrap:anywhere;margin:0}}\
         </style></head><body><main><h1>{title}</h1><p>{note}</p><pre>{text}</pre></main></body></html>\n"
    )
}

fn same(url: &str, page: &str) -> bool {
    url.trim_end_matches('/') == page.trim_end_matches('/')
}

/// A URL this scheme serves.
pub fn is_welcome(url: &str) -> bool {
    same(url, URL)
}

/// The page at `url`, if it is one of ours.
fn page(url: &str) -> Option<&'static str> {
    if is_welcome(url) {
        Some(PAGE)
    } else if same(url, LICENSE_URL) {
        Some(&LICENSE_PAGE)
    } else if same(url, THIRD_PARTY_URL) {
        Some(&THIRD_PARTY_PAGE)
    } else {
        None
    }
}

pub fn respond(
    _ctx: UriSchemeContext<'_, tauri::Wry>,
    request: Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let (status, body, kind) = match page(&request.uri().to_string()) {
        Some(page) => (200, page.as_bytes().to_vec(), "text/html; charset=utf-8"),
        None => (404, b"not found".to_vec(), "text/plain"),
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
    open_page(app, URL)
}

/// Open one of this scheme's pages in a new tab. Not from the main thread.
pub fn open_page(app: &AppHandle, url: &str) -> tauri::Result<()> {
    crate::browser::open_tab(app, url.parse().expect("constant url")).map(|_| ())
}

/// Show the license (or, with `third_party`, the notices of the software Ternt includes)
/// in a tab, and bring the main window forward: the setup and Settings windows ask for it,
/// and the tab opens behind them.
#[tauri::command]
pub async fn open_license(app: AppHandle, third_party: bool) -> Result<(), String> {
    let url = if third_party {
        THIRD_PARTY_URL
    } else {
        LICENSE_URL
    };
    open_page(&app, url).map_err(|e| e.to_string())?;
    // Focusing a window activates the app; test runs stay in the background (see `lib.rs`).
    let testing = cfg!(debug_assertions) && std::env::var_os("TERNT_TEST").is_some();
    if !testing && let Some(window) = app.get_window(crate::browser::MAIN_WINDOW) {
        window.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
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
    fn the_license_pages_show_their_text_escaped() {
        let license = page("ternt://license").unwrap();
        assert!(license.contains("Ternt Source-Available License"));
        assert!(license.contains("WITHOUT WARRANTY OF ANY KIND"));
        let third = page(THIRD_PARTY_URL).unwrap();
        assert!(third.contains("Part 1: packages") && third.contains("tauri "));
        // Texts hold <year> and <name of author>: shown, never markup. No scripts either.
        assert!(!THIRD_PARTY.contains("&lt;") && third.contains("&lt;"));
        let body = &third[third.find("<pre>").unwrap()..third.rfind("</pre>").unwrap()];
        assert!(!body[5..].contains('<'));
        assert!(page("ternt://other").is_none());
    }

    #[test]
    fn page_is_self_contained_html() {
        assert!(PAGE.starts_with("<!doctype html>"));
        assert!(PAGE.contains("<title>Welcome to Ternt</title>"));
        assert!(
            !PAGE.contains("<script"),
            "no scripts: tab webviews have no IPC"
        );
        assert!(
            !PAGE.contains("src=\"http"),
            "no remote assets: the page must work offline"
        );
    }
}
