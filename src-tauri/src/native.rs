//! macOS-only tweaks applied directly to the underlying `WKWebView`s.

use std::{cell::RefCell, collections::HashMap, ffi::c_void, ptr};

use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject},
};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBox, NSBoxType, NSColor, NSEvent, NSResponder, NSTitlePosition,
    NSView, NSWindowOrderingMode,
};
use objc2_foundation::{
    NSKeyValueObservingOptions, NSObjectNSKeyValueObserverRegistration, NSPoint, NSRect, NSSize,
    NSString, NSURL,
};
use tauri::{AppHandle, Manager, Webview};

use crate::browser::{self, NativeState, Pane};

/// Pin the view's margins to its superview so AppKit resizes it in the same
/// frame as the window. This avoids an event-loop/IPC hop per frame during
/// live resize, which is what makes multi-webview layouts lag.
pub fn track_window_size(webview: &Webview) {
    let _ = webview.with_webview(|platform| unsafe {
        pin_to_superview(view(platform.inner()));
    });
}

/// Content webviews: track window size and round the corners to sit inside
/// the sidebar chrome. `swipe` enables trackpad swipe back/forward (pages).
pub fn style_content(webview: &Webview, corner_radius: f64, swipe: bool) {
    let _ = webview.with_webview(move |platform| unsafe {
        let view = view(platform.inner());
        pin_to_superview(view);
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            layer.setCornerRadius(corner_radius);
            layer.setMasksToBounds(true);
        }
        let _: () = msg_send![view, setAllowsBackForwardNavigationGestures: swipe];
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

/// How a content view resizes with the window: the full content area pins
/// all four edges; a split pane also lets its outer margin grow, so the two
/// panes share a resize roughly evenly until `browser` lays them out exactly.
pub fn set_pane(webview: &Webview, pane: Pane) {
    let _ = webview.with_webview(move |platform| unsafe {
        let sizable = NSAutoresizingMaskOptions::ViewWidthSizable
            | NSAutoresizingMaskOptions::ViewHeightSizable;
        let mask = match pane {
            Pane::Full => sizable,
            Pane::Left => sizable | NSAutoresizingMaskOptions::ViewMaxXMargin,
            Pane::Right => sizable | NSAutoresizingMaskOptions::ViewMinXMargin,
        };
        view(platform.inner()).setAutoresizingMask(mask);
    });
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
    #[name = "TerntTabObserver"]
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
        let observer: Retained<TabObserver> = msg_send![
            super(TabObserver::alloc(mtm).set_ivars(ObserverIvars { tab, app })),
            init
        ];
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

// --- Focus between split panes ---------------------------------------------
//
// The window's first responder is key-value observable: when it moves into
// a content view (a click into the other pane of a split), tell `browser`
// which tab that view shows.

struct FocusIvars {
    app: AppHandle,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and we don't impl Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TerntFocusObserver"]
    #[ivars = FocusIvars]
    struct FocusObserver;

    impl FocusObserver {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe(
            &self,
            _key_path: Option<&NSString>,
            object: Option<&AnyObject>,
            _change: Option<&AnyObject>,
            _context: *mut c_void,
        ) {
            let Some(window) = object else { return };
            let responder: *mut AnyObject = unsafe { msg_send![window, firstResponder] };
            let Some(responder) = (unsafe { responder.as_ref() }) else { return };
            let is_view: bool = unsafe { msg_send![responder, isKindOfClass: objc2::class!(NSView)] };
            if !is_view {
                return;
            }
            // Walk up from the focused view (WebKit focuses an inner view) to
            // a content webview.
            let mut current: Option<Retained<NSView>> =
                Some(unsafe { &*(responder as *const AnyObject as *const NSView) }.retain());
            while let Some(view) = current {
                let ptr = Retained::as_ptr(&view) as usize;
                if TERM_VIEW.get() == Some(ptr) {
                    browser::on_view_focused(&self.ivars().app, None, true);
                    return;
                }
                let tab = OBSERVERS.with_borrow(|map| {
                    map.iter().find(|(_, (_, v))| Retained::as_ptr(v) as usize == ptr).map(|(tab, _)| *tab)
                });
                if let Some(tab) = tab {
                    browser::on_view_focused(&self.ivars().app, Some(tab), false);
                    return;
                }
                current = unsafe { view.superview() };
            }
        }
    }
);

thread_local! {
    static FOCUS_OBSERVER: RefCell<Option<Retained<FocusObserver>>> = const { RefCell::new(None) };
    /// The shared terminal webview's view, once created.
    static TERM_VIEW: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Start reporting focus moves between content views in `window`.
pub fn watch_focus(window: &tauri::Window, app: AppHandle) {
    let Ok(ns_window) = window.ns_window() else {
        return;
    };
    let ns_window = ns_window as usize; // raw pointers aren't Send
    let _ = window.run_on_main_thread(move || unsafe {
        let mtm = MainThreadMarker::new().expect("run_on_main_thread runs on the main thread");
        let window = &*(ns_window as *const NSObject);
        let observer: Retained<FocusObserver> = msg_send![
            super(FocusObserver::alloc(mtm).set_ivars(FocusIvars { app })),
            init
        ];
        window.addObserver_forKeyPath_options_context(
            &observer,
            &NSString::from_str("firstResponder"),
            NSKeyValueObservingOptions::empty(),
            ptr::null_mut(),
        );
        FOCUS_OBSERVER.set(Some(observer));
    });
}

/// Remember the shared terminal view, so focus in it maps to its tab.
pub fn register_terminal_view(webview: &Webview) {
    let _ = webview.with_webview(|platform| {
        TERM_VIEW.set(Some(platform.inner() as usize));
    });
}

/// Hide or show the close/minimize/zoom buttons (hidden while the sidebar is
/// collapsed, where they'd overlap the page).
pub fn set_traffic_lights_hidden(window: &tauri::Window, hidden: bool) {
    let Ok(ns_window) = window.ns_window() else {
        return;
    };
    let ns_window = ns_window as usize; // raw pointers aren't Send
    let _ = window.run_on_main_thread(move || unsafe {
        let ns_window = &*(ns_window as *const AnyObject);
        // NSWindowCloseButton, NSWindowMiniaturizeButton, NSWindowZoomButton
        for kind in 0usize..3 {
            let button: *mut AnyObject = msg_send![ns_window, standardWindowButton: kind];
            if let Some(button) = button.as_ref() {
                let _: () = msg_send![button, setHidden: hidden];
            }
        }
    });
}

pub fn copy_to_clipboard(app: &AppHandle, text: String) {
    let _ = app.run_on_main_thread(move || unsafe {
        let pasteboard: *mut AnyObject = msg_send![objc2::class!(NSPasteboard), generalPasteboard];
        let Some(pasteboard) = pasteboard.as_ref() else {
            return;
        };
        let _: isize = msg_send![pasteboard, clearContents];
        let text = NSString::from_str(&text);
        let kind = NSString::from_str("public.utf8-plain-text");
        let _: bool = msg_send![pasteboard, setString: &*text, forType: &*kind];
    });
}

// --- Explore mode dimming --------------------------------------------------
//
// One translucent view over the content area while exploring tabs (⌘F). It
// sits above every content webview, so pages get no input; a click on it
// ends explore mode. AppKit resizes it with the window like the webviews.

define_class!(
    // SAFETY: NSBox has no subclassing requirements and we don't impl Drop.
    #[unsafe(super(NSBox, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TerntDimView"]
    #[ivars = AppHandle]
    struct DimView;

    impl DimView {
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            let app = self.ivars().clone();
            std::thread::spawn(move || browser::end_explore(&app, browser::ExploreEnd::Commit));
        }
    }
);

thread_local! {
    static DIM: RefCell<Option<Retained<DimView>>> = const { RefCell::new(None) };
}

/// Show the dim over `frame` (x, y, width, height in the content view's
/// coordinates) above all content views, or remove it with `None`. Called
/// again after a view is added, to stay on top.
pub fn set_dim(ui: &Webview, frame: Option<(f64, f64, f64, f64)>, radius: f64) {
    let app = ui.app_handle().clone();
    let _ = ui.with_webview(move |platform| unsafe {
        let mtm = MainThreadMarker::new().expect("with_webview runs on the main thread");
        let Some((x, y, width, height)) = frame else {
            if let Some(dim) = DIM.take() {
                dim.removeFromSuperview();
            }
            return;
        };
        let Some(parent) = view(platform.inner()).superview() else {
            return;
        };
        let dim = DIM.with_borrow_mut(|slot| {
            slot.get_or_insert_with(|| {
                let dim: Retained<DimView> =
                    msg_send![super(DimView::alloc(mtm).set_ivars(app)), init];
                dim.setBoxType(NSBoxType::Custom);
                dim.setTitlePosition(NSTitlePosition::NoTitle);
                dim.setBorderWidth(0.0);
                dim.setCornerRadius(radius);
                dim.setFillColor(&NSColor::colorWithWhite_alpha(0.0, 0.32));
                dim.setContentViewMargins(NSSize::new(0.0, 0.0));
                pin_to_superview(&dim);
                dim
            })
            .clone()
        });
        dim.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(width, height)));
        parent.addSubview_positioned_relativeTo(&dim, NSWindowOrderingMode::Above, None);
    });
}

// --- Default browser ----------------------------------------------------------
//
// Launch Services' per-scheme handlers. Setting one makes macOS ask the user
// to confirm. (Deprecated in favor of an NSWorkspace call that takes a block;
// these still work and need no extra crates.)

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    /// Returns a +1 CFString (a bundle id), or null.
    fn LSCopyDefaultHandlerForURLScheme(scheme: *const NSString) -> *mut NSString;
    fn LSSetDefaultHandlerForURLScheme(scheme: *const NSString, bundle_id: *const NSString) -> i32;
}

/// Whether `bundle_id` handles http links.
pub fn is_default_browser(bundle_id: &str) -> bool {
    let scheme = NSString::from_str("http");
    let handler = unsafe { LSCopyDefaultHandlerForURLScheme(&*scheme) };
    // Copy rule: we own the result; Retained releases it.
    let Some(handler) = (unsafe { Retained::from_raw(handler) }) else {
        return false;
    };
    handler.to_string().eq_ignore_ascii_case(bundle_id)
}

/// Make `bundle_id` the handler for http and https links.
pub fn make_default_browser(bundle_id: &str) {
    let id = NSString::from_str(bundle_id);
    for scheme in ["http", "https"] {
        let scheme = NSString::from_str(scheme);
        let status = unsafe { LSSetDefaultHandlerForURLScheme(&*scheme, &*id) };
        if status != 0 {
            eprintln!("failed to become the default browser for {scheme}: {status}");
        }
    }
}
