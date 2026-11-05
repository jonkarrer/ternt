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

use crate::browser::{self, NativeState};

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

/// Keep the view's width and left edge as the window resizes, following
/// its height (the peeking sidebar).
pub fn track_window_height(webview: &Webview) {
    let _ = webview.with_webview(|platform| unsafe {
        view(platform.inner()).setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewHeightSizable
                | NSAutoresizingMaskOptions::ViewMaxXMargin,
        );
    });
}

fn pin_to_superview(view: &NSView) {
    view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
}

/// How AppKit resizes a content view with the window until the next exact
/// layout: the whole content area tracks it, a group's pane scales with it.
pub fn set_pane(webview: &Webview, whole: bool) {
    let _ = webview.with_webview(move |platform| unsafe {
        let sizable = NSAutoresizingMaskOptions::ViewWidthSizable
            | NSAutoresizingMaskOptions::ViewHeightSizable;
        let mask = if whole {
            sizable
        } else {
            sizable
                | NSAutoresizingMaskOptions::ViewMinXMargin
                | NSAutoresizingMaskOptions::ViewMaxXMargin
                | NSAutoresizingMaskOptions::ViewMinYMargin
                | NSAutoresizingMaskOptions::ViewMaxYMargin
        };
        view(platform.inner()).setAutoresizingMask(mask);
    });
}

/// Put `webview` right below `other` (the terminal view under a group's
/// pages: it spans the terminals' panes, which can reach around a page).
pub fn place_below(webview: &Webview, other: &Webview) {
    let other = other.clone();
    let _ = webview.with_webview(move |platform| {
        let below = platform.inner() as usize;
        let _ = other.with_webview(move |other| unsafe {
            let below = view(below as *mut c_void);
            let other = view(other.inner());
            if let Some(superview) = other.superview() {
                superview.addSubview_positioned_relativeTo(below, NSWindowOrderingMode::Below, Some(other));
            }
        });
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

thread_local! {
    static DROP_HINT: RefCell<Option<Retained<NSBox>>> = const { RefCell::new(None) };
}

/// While a tab is dragged over the content area: a tinted box over where it
/// would land (x, y, width, height from the window's top left), above all
/// content views; `None` removes it.
pub fn set_drop_hint(ui: &Webview, frame: Option<(f64, f64, f64, f64)>, radius: f64) {
    let _ = ui.with_webview(move |platform| unsafe {
        let mtm = MainThreadMarker::new().expect("with_webview runs on the main thread");
        let Some((x, y, width, height)) = frame else {
            if let Some(hint) = DROP_HINT.take() {
                hint.removeFromSuperview();
            }
            return;
        };
        let Some(parent) = view(platform.inner()).superview() else {
            return;
        };
        let hint = DROP_HINT.with_borrow_mut(|slot| {
            slot.get_or_insert_with(|| {
                let hint = NSBox::new(mtm);
                hint.setBoxType(NSBoxType::Custom);
                hint.setTitlePosition(NSTitlePosition::NoTitle);
                hint.setBorderWidth(2.0);
                hint.setCornerRadius(radius);
                let accent = NSColor::controlAccentColor();
                hint.setBorderColor(&accent.colorWithAlphaComponent(0.9));
                hint.setFillColor(&accent.colorWithAlphaComponent(0.22));
                hint
            })
            .clone()
        });
        let y = if parent.isFlipped() { y } else { parent.frame().size.height - y - height };
        hint.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(width, height)));
        parent.addSubview_positioned_relativeTo(&hint, NSWindowOrderingMode::Above, None);
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

/// Move a page's Web Inspector into its own window once it shows (WebKit's
/// private `_WKInspector`, as the inspector's own "Detach" button does). It
/// opens asynchronously, so this checks for a moment off the main thread.
/// Detaching resizes the page to the whole window: `then` lays it out again.
pub fn detach_inspector(webview: &tauri::Webview, then: impl FnOnce() + Send + 'static) {
    let webview = webview.clone();
    std::thread::spawn(move || {
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let (tx, rx) = std::sync::mpsc::channel();
            let sent = webview.with_webview(move |p| unsafe {
                let view = &*(p.inner() as *const AnyObject);
                let inspector: *mut AnyObject = msg_send![view, _inspector];
                let done = inspector.as_ref().is_some_and(|inspector| {
                    let visible: bool = msg_send![inspector, isVisible];
                    if visible {
                        let _: () = msg_send![inspector, detach];
                    }
                    visible
                });
                let _ = tx.send(done);
            });
            match (sent, rx.recv()) {
                (Ok(()), Ok(true)) => return then(),
                (Ok(()), Ok(false)) => {}
                _ => return,
            }
        }
    });
}

/// Tell Finder a download finished: the Dock's Downloads stack bounces, as
/// it does for Safari.
pub fn download_finished(path: &std::path::Path) {
    let Some(path) = path.to_str() else {
        return;
    };
    unsafe {
        let center: *mut AnyObject = msg_send![
            objc2::class!(NSDistributedNotificationCenter),
            defaultCenter
        ];
        let name = NSString::from_str("com.apple.DownloadFileFinished");
        let object = NSString::from_str(path);
        let _: () = msg_send![center, postNotificationName: &*name, object: &*object];
    }
}

/// Set up a web page's view (tabs, popups, Little Ternt) right after
/// creating it, before its first page loads.
///
/// - Remove the scripts Tauri adds to every webview (its IPC bridge). Web
///   pages never talk to Ternt, and the bridge defines a locked `window.ipc`:
///   sites that declare their own global `ipc` (Google Docs, Sheets, Forms…)
///   fail to load.
/// - Show `alert()`, `confirm()` and `prompt()`. wry leaves them out, so
///   WebKit answered at once with nothing: a site asking for a password with
///   `prompt()` got `null` and its link did nothing.
/// - Tell pages their window's frame. Without it `outerWidth` and
///   `outerHeight` are 0, and Google Docs, which compares them with the inner
///   size to guess the zoom, drew its pages at half resolution (blurry).
pub fn prepare_page(webview: tauri::webview::PlatformWebview) {
    unsafe {
        let view = &*(webview.inner() as *const AnyObject);
        let config: *mut AnyObject = msg_send![view, configuration];
        let controller: *mut AnyObject = msg_send![&*config, userContentController];
        let _: () = msg_send![&*controller, removeAllUserScripts];

        let delegate: *mut AnyObject = msg_send![view, UIDelegate];
        if let Some(delegate) = delegate.as_ref() {
            add_js_dialogs(delegate.class());
            // WebKit notes which methods a delegate has when it's set:
            // set it again so it sees the added ones.
            let _: () = msg_send![view, setUIDelegate: delegate];
        }
    }
}

// --- JavaScript dialogs ------------------------------------------------------
//
// Added to wry's WKUIDelegate class at runtime (Tauri doesn't let us supply
// our own delegate). Each shows a sheet on the page's window, like Safari.

type JsAlert = unsafe extern "C-unwind" fn(
    &AnyObject,
    objc2::runtime::Sel,
    &AnyObject,
    &NSString,
    &AnyObject,
    &block2::Block<dyn Fn()>,
);
type JsConfirm = unsafe extern "C-unwind" fn(
    &AnyObject,
    objc2::runtime::Sel,
    &AnyObject,
    &NSString,
    &AnyObject,
    &block2::Block<dyn Fn(objc2::runtime::Bool)>,
);
type JsPrompt = unsafe extern "C-unwind" fn(
    &AnyObject,
    objc2::runtime::Sel,
    &AnyObject,
    &NSString,
    *mut NSString,
    &AnyObject,
    &block2::Block<dyn Fn(*mut NSString)>,
);

type WindowFrame = unsafe extern "C-unwind" fn(
    &AnyObject,
    objc2::runtime::Sel,
    &AnyObject,
    &block2::Block<dyn Fn(NSRect)>,
);

/// Add the dialog methods (and the window frame) to wry's delegate class,
/// once.
unsafe fn add_js_dialogs(class: &objc2::runtime::AnyClass) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        let class = class as *const _ as *mut objc2::runtime::AnyClass;
        let methods: [(&str, objc2::runtime::Imp, &std::ffi::CStr); 4] = [
            (
                "webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:",
                std::mem::transmute::<JsAlert, objc2::runtime::Imp>(js_alert),
                c"v@:@@@@?",
            ),
            (
                "webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:",
                std::mem::transmute::<JsConfirm, objc2::runtime::Imp>(js_confirm),
                c"v@:@@@@?",
            ),
            (
                "webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:",
                std::mem::transmute::<JsPrompt, objc2::runtime::Imp>(js_prompt),
                c"v@:@@@@@?",
            ),
            // WebKit's private WKUIDelegate method, which Safari implements.
            (
                "_webView:getWindowFrameWithCompletionHandler:",
                std::mem::transmute::<WindowFrame, objc2::runtime::Imp>(window_frame),
                c"v@:@@?",
            ),
        ];
        for (name, imp, types) in methods {
            objc2::ffi::class_addMethod(class, objc2::runtime::Sel::register(&std::ffi::CString::new(name).unwrap()), imp, types.as_ptr());
        }
    });
}

/// An alert for a page: titled with the site's host, `message` below,
/// OK (and Cancel when asked).
unsafe fn js_dialog_alert(
    frame: &AnyObject,
    message: &NSString,
    cancel: bool,
) -> Retained<AnyObject> {
    unsafe {
        let alert: Retained<AnyObject> = msg_send![objc2::class!(NSAlert), new];
        let request: *mut AnyObject = msg_send![frame, request];
        let url: *mut NSURL = msg_send![&*request, URL];
        let host = url
            .as_ref()
            .and_then(|url| url.host())
            .map(|host| host.to_string())
            .unwrap_or_default();
        let title = if host.is_empty() {
            "This page says".to_string()
        } else {
            format!("{host} says")
        };
        let _: () = msg_send![&*alert, setMessageText: &*NSString::from_str(&title)];
        let _: () = msg_send![&*alert, setInformativeText: message];
        let _: *mut AnyObject = msg_send![&*alert, addButtonWithTitle: &*NSString::from_str("OK")];
        if cancel {
            let _: *mut AnyObject =
                msg_send![&*alert, addButtonWithTitle: &*NSString::from_str("Cancel")];
        }
        alert
    }
}

/// Show `alert` as a sheet on `web_view`'s window (modal if it has none);
/// `done` gets whether OK was chosen.
unsafe fn js_dialog_show(web_view: &AnyObject, alert: &AnyObject, done: impl Fn(bool) + 'static) {
    const FIRST_BUTTON: isize = 1000; // NSAlertFirstButtonReturn
    unsafe {
        let window: *mut AnyObject = msg_send![web_view, window];
        match window.as_ref() {
            Some(window) => {
                let handler =
                    block2::RcBlock::new(move |response: isize| done(response == FIRST_BUTTON));
                let _: () = msg_send![alert, beginSheetModalForWindow: window, completionHandler: &*handler];
            }
            None => {
                let response: isize = msg_send![alert, runModal];
                done(response == FIRST_BUTTON);
            }
        }
    }
}

/// The "window" a page sees (`outerWidth`, `outerHeight`, `screenX`,
/// `screenY`): its own view's frame on screen, as if the browser had no
/// chrome. Google Docs scales its canvas by outer / inner width, so the
/// whole window (sidebar included, or a split's other side) over-draws it.
unsafe extern "C-unwind" fn window_frame(
    _this: &AnyObject,
    _cmd: objc2::runtime::Sel,
    web_view: &AnyObject,
    completion: &block2::Block<dyn Fn(NSRect)>,
) {
    unsafe {
        let bounds: NSRect = msg_send![web_view, bounds];
        let window: *mut AnyObject = msg_send![web_view, window];
        let frame = match window.as_ref() {
            Some(window) => {
                let null: *const AnyObject = ptr::null();
                let in_window: NSRect = msg_send![web_view, convertRect: bounds, toView: null];
                msg_send![window, convertRectToScreen: in_window]
            }
            None => bounds,
        };
        completion.call((frame,));
    }
}

unsafe extern "C-unwind" fn js_alert(
    _this: &AnyObject,
    _cmd: objc2::runtime::Sel,
    web_view: &AnyObject,
    message: &NSString,
    frame: &AnyObject,
    completion: &block2::Block<dyn Fn()>,
) {
    unsafe {
        let alert = js_dialog_alert(frame, message, false);
        let completion = completion.copy();
        js_dialog_show(web_view, &alert, move |_| completion.call(()));
    }
}

unsafe extern "C-unwind" fn js_confirm(
    _this: &AnyObject,
    _cmd: objc2::runtime::Sel,
    web_view: &AnyObject,
    message: &NSString,
    frame: &AnyObject,
    completion: &block2::Block<dyn Fn(objc2::runtime::Bool)>,
) {
    unsafe {
        let alert = js_dialog_alert(frame, message, true);
        let completion = completion.copy();
        js_dialog_show(web_view, &alert, move |ok| {
            completion.call((objc2::runtime::Bool::new(ok),))
        });
    }
}

unsafe extern "C-unwind" fn js_prompt(
    _this: &AnyObject,
    _cmd: objc2::runtime::Sel,
    web_view: &AnyObject,
    prompt: &NSString,
    default_text: *mut NSString,
    frame: &AnyObject,
    completion: &block2::Block<dyn Fn(*mut NSString)>,
) {
    unsafe {
        let alert = js_dialog_alert(frame, prompt, true);
        let field: objc2::rc::Allocated<AnyObject> = msg_send![objc2::class!(NSTextField), alloc];
        let field: Retained<AnyObject> = msg_send![field, initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(260.0, 24.0))];
        if let Some(text) = default_text.as_ref() {
            let _: () = msg_send![&*field, setStringValue: text];
        }
        let _: () = msg_send![&*alert, setAccessoryView: &*field];
        let alert_window: *mut AnyObject = msg_send![&*alert, window];
        let _: () = msg_send![&*alert_window, setInitialFirstResponder: &*field];
        let completion = completion.copy();
        js_dialog_show(web_view, &alert, move |ok| {
            if ok {
                let text: *mut NSString = msg_send![&*field, stringValue];
                completion.call((text,));
            } else {
                completion.call((ptr::null_mut(),));
            }
        });
    }
}

/// A page's icon links as `<link>` tags with absolute hrefs, read from the
/// live page (signed in, with links its scripts added). Runs in an isolated
/// script world, so the page's own scripts can't interfere.
pub async fn page_icon_links(webview: &Webview) -> Option<String> {
    const JS: &str = r#"[...document.querySelectorAll("link[rel][href]")]
        .map((l) => `<link rel="${l.rel}" href="${l.href.replaceAll('"', "%22")}" sizes="${l.sizes?.value ?? ""}">`)
        .join("")"#;
    let (tx, rx) = tokio::sync::oneshot::channel::<Option<String>>();
    let tx = std::sync::Mutex::new(Some(tx));
    webview
        .with_webview(move |platform| unsafe {
            let view = &*(platform.inner() as *const AnyObject);
            let world: *mut AnyObject =
                msg_send![objc2::class!(WKContentWorld), defaultClientWorld];
            let done =
                block2::RcBlock::new(move |result: *mut AnyObject, _error: *mut AnyObject| {
                    let text = result
                        .as_ref()
                        .filter(|r| msg_send![*r, isKindOfClass: objc2::class!(NSString)])
                        .map(|r| (*(r as *const AnyObject as *const NSString)).to_string());
                    if let Some(tx) = tx.lock().unwrap().take() {
                        let _ = tx.send(text);
                    }
                });
            let js = NSString::from_str(JS);
            let _: () = msg_send![
                view,
                evaluateJavaScript: &*js,
                inFrame: ptr::null::<AnyObject>(),
                inContentWorld: world,
                completionHandler: &*done
            ];
        })
        .ok()?;
    tokio::time::timeout(std::time::Duration::from_secs(2), rx)
        .await
        .ok()?
        .ok()?
}
