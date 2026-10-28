//! macOS-only tweaks applied directly to the underlying `WKWebView`s.

use std::{cell::RefCell, collections::HashMap, ffi::c_void, ptr};

use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject},
};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSView, NSWindowOrderingMode};
use objc2_foundation::{NSKeyValueObservingOptions, NSObjectNSKeyValueObserverRegistration, NSString, NSURL};
use tauri::{AppHandle, Webview};

use crate::browser::{self, NativeState};

/// Pin the view's margins to its superview so AppKit resizes it in the same
/// frame as the window. This avoids an event-loop/IPC hop per frame during
/// live resize, which is what makes multi-webview layouts lag.
pub fn track_window_size(webview: &Webview) {
    let _ = webview.with_webview(|platform| unsafe {
        pin_to_superview(view(platform.inner()));
    });
}

/// Page webviews: track window size, round the corners to sit inside the
/// sidebar chrome, and enable trackpad swipe back/forward.
pub fn style_content(webview: &Webview, corner_radius: f64) {
    let _ = webview.with_webview(move |platform| unsafe {
        let view = view(platform.inner());
        pin_to_superview(view);
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            layer.setCornerRadius(corner_radius);
            layer.setMasksToBounds(true);
        }
        let _: () = msg_send![view, setAllowsBackForwardNavigationGestures: true];
    });
}

/// Raise the view above its siblings (later-created tab webviews would
/// otherwise sit on top of it).
pub fn bring_to_front(webview: &Webview) {
    let _ = webview.with_webview(|platform| unsafe {
        let view = view(platform.inner());
        if let Some(superview) = view.superview() {
            superview.addSubview_positioned_relativeTo(view, NSWindowOrderingMode::Above, None);
        }
    });
}

pub fn go_back(webview: &Webview) {
    let _ = webview.with_webview(|platform| unsafe {
        let _: *mut AnyObject = msg_send![view(platform.inner()), goBack];
    });
}

pub fn go_forward(webview: &Webview) {
    let _ = webview.with_webview(|platform| unsafe {
        let _: *mut AnyObject = msg_send![view(platform.inner()), goForward];
    });
}

unsafe fn view<'a>(ptr: *mut c_void) -> &'a NSView {
    unsafe { &*(ptr as *const NSView) }
}

fn pin_to_superview(view: &NSView) {
    view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
}

// --- Navigation state via key-value observing ---------------------------
//
// WKWebView exposes URL/title/loading/back-forward state as KVO properties.
// Observing them is exact (it includes pushState route changes in SPAs) and
// costs nothing while a page is idle.

const OBSERVED_KEYS: [&str; 5] = ["URL", "title", "loading", "canGoBack", "canGoForward"];

struct ObserverIvars {
    tab: u32,
    app: AppHandle,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and we don't impl Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MooveTabObserver"]
    #[ivars = ObserverIvars]
    struct TabObserver;

    impl TabObserver {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe(
            &self,
            _key_path: Option<&NSString>,
            object: Option<&AnyObject>,
            _change: Option<&AnyObject>,
            _context: *mut c_void,
        ) {
            let Some(webview) = object else { return };
            let state = unsafe { read_state(webview) };
            let ivars = self.ivars();
            browser::on_native_change(&ivars.app, ivars.tab, state);
        }
    }
);

unsafe fn read_state(webview: &AnyObject) -> NativeState {
    unsafe {
        let url: Option<Retained<NSURL>> = msg_send![webview, URL];
        let title: Option<Retained<NSString>> = msg_send![webview, title];
        NativeState {
            url: url.and_then(|u| u.absoluteString()).map(|s| s.to_string()),
            title: title.map(|s| s.to_string()).unwrap_or_default(),
            loading: msg_send![webview, isLoading],
            can_go_back: msg_send![webview, canGoBack],
            can_go_forward: msg_send![webview, canGoForward],
        }
    }
}

/// An observer and the view it observes, kept so it can be removed on close.
type Observation = (Retained<TabObserver>, Retained<NSView>);

thread_local! {
    // Main-thread only, like the observers themselves. Keyed by tab id.
    static OBSERVERS: RefCell<HashMap<u32, Observation>> = RefCell::new(HashMap::new());
}

pub fn observe(webview: &Webview, tab: u32, app: AppHandle) {
    let _ = webview.with_webview(move |platform| unsafe {
        let mtm = MainThreadMarker::new().expect("with_webview runs on the main thread");
        let view = view(platform.inner());
        let observer: Retained<TabObserver> =
            msg_send![super(TabObserver::alloc(mtm).set_ivars(ObserverIvars { tab, app })), init];
        for key in OBSERVED_KEYS {
            view.addObserver_forKeyPath_options_context(
                &observer,
                &NSString::from_str(key),
                NSKeyValueObservingOptions::empty(),
                ptr::null_mut(),
            );
        }
        OBSERVERS.with_borrow_mut(|map| map.insert(tab, (observer, view.retain())));
    });
}

/// Must run before the webview is closed.
pub fn unobserve(webview: &Webview, tab: u32) {
    let _ = webview.with_webview(move |_| unsafe {
        let Some((observer, view)) = OBSERVERS.with_borrow_mut(|map| map.remove(&tab)) else {
            return;
        };
        for key in OBSERVED_KEYS {
            view.removeObserver_forKeyPath(&observer, &NSString::from_str(key));
        }
    });
}
