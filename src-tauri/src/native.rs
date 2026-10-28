//! macOS-only tweaks applied directly to the underlying `WKWebView`s.

use objc2_app_kit::{NSAutoresizingMaskOptions, NSView};
use tauri::Webview;

/// Pin the view's margins to its superview so AppKit resizes it in the same
/// frame as the window. This avoids an event-loop/IPC hop per frame during
/// live resize, which is what makes multi-webview layouts lag.
pub fn track_window_size(webview: &Webview) {
    let _ = webview.with_webview(|platform| unsafe {
        let view = &*(platform.inner() as *const NSView);
        pin_to_superview(view);
    });
}

/// Page webviews: track window size, round the corners to sit inside the
/// sidebar chrome, and enable trackpad swipe back/forward.
pub fn style_content(webview: &Webview, corner_radius: f64) {
    let _ = webview.with_webview(move |platform| unsafe {
        let view = &*(platform.inner() as *const NSView);
        pin_to_superview(view);
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            layer.setCornerRadius(corner_radius);
            layer.setMasksToBounds(true);
        }
        let _: () = objc2::msg_send![view, setAllowsBackForwardNavigationGestures: true];
    });
}

fn pin_to_superview(view: &NSView) {
    view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
}
