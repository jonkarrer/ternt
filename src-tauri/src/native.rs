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

// --- Mouse moves ---------------------------------------------------------------
//
// AppKit sends mouse moves to the first responder wherever the mouse is, and
// to each view's tracking area over its whole visible rect, which ignores
// sibling views on top. With the UI webview spanning the window under the
// pages (or a hidden terminal view still holding the keyboard), two webviews
// handled each move over a link or text and fought over the cursor: arrow,
// pointer, arrow. Each webview now only takes moves over the part of it that
// shows. WebKit's tracking areas belong to a `WKMouseTrackingObserver`, not
// the view, so both get the guard.

type MouseEvent = unsafe extern "C-unwind" fn(&AnyObject, objc2::runtime::Sel, &AnyObject);

/// The originals: the view's mouseMoved:, the observer's mouseMoved:,
/// mouseEntered: and mouseExited:.
static VIEW_MOVED: std::sync::OnceLock<MouseEvent> = std::sync::OnceLock::new();
static OBSERVER_MOVED: std::sync::OnceLock<MouseEvent> = std::sync::OnceLock::new();
static OBSERVER_ENTERED: std::sync::OnceLock<MouseEvent> = std::sync::OnceLock::new();
static OBSERVER_EXITED: std::sync::OnceLock<MouseEvent> = std::sync::OnceLock::new();
/// Observers whose view the mouse left for a view on top (told with a
/// mouseExited:, so hover doesn't stick on what it last passed over).
static COVERED: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());

/// Install the guards (once: wry's webview class is shared by every
/// webview, WebKit's observer class by every page).
pub fn guard_mouse_moves(webview: &Webview) {
    let _ = webview.with_webview(|platform| unsafe {
        if VIEW_MOVED.get().is_some() {
            return;
        }
        let view = &*(platform.inner() as *const AnyObject);
        // The view's own class, not a KVO subclass of it (which only the
        // observed views would have).
        let class: *const objc2::runtime::AnyClass = msg_send![view, class];
        replace(class, objc2::sel!(mouseMoved:), view_moved, &VIEW_MOVED);
        if let Some(observer) = objc2::runtime::AnyClass::get(c"WKMouseTrackingObserver") {
            replace(observer, objc2::sel!(mouseMoved:), observer_moved, &OBSERVER_MOVED);
            replace(observer, objc2::sel!(mouseEntered:), observer_entered, &OBSERVER_ENTERED);
            replace(observer, objc2::sel!(mouseExited:), observer_exited, &OBSERVER_EXITED);
        }
    });
}

/// Put `imp` in place of `class`'s method `sel` (its own or inherited),
/// keeping the original in `original`.
unsafe fn replace(
    class: *const objc2::runtime::AnyClass,
    sel: objc2::runtime::Sel,
    imp: MouseEvent,
    original: &std::sync::OnceLock<MouseEvent>,
) {
    unsafe {
        let method = objc2::ffi::class_getInstanceMethod(class, sel);
        if method.is_null() {
            return;
        }
        let Some(old) = objc2::ffi::method_getImplementation(method) else {
            return;
        };
        let _ = original.set(std::mem::transmute::<objc2::runtime::Imp, MouseEvent>(old));
        let imp = std::mem::transmute::<MouseEvent, objc2::runtime::Imp>(imp);
        let class = class as *mut objc2::runtime::AnyClass;
        if !objc2::ffi::class_addMethod(class, sel, imp, c"v@:@".as_ptr()).as_bool() {
            // The class has its own: replace it (the original runs after).
            objc2::ffi::method_setImplementation(method, imp);
        }
    }
}

// --- WebKit's Screen Time observer ------------------------------------------
//
// Freeing a WKWebView stops its Screen Time observer, which sometimes raises
// ("Cannot remove an observer <WKScreenTimeConfigurationObserver> for the key
// path configuration.enforcesChildRestrictions …", a WebKit bug). It raises
// inside wry's dealloc, through Rust frames, so closing a tab aborted the
// app. The observer now stops inside `caught`.

type Stop = unsafe extern "C-unwind" fn(&AnyObject, objc2::runtime::Sel);
static STOP_OBSERVING: std::sync::OnceLock<Stop> = std::sync::OnceLock::new();

/// Install the guard (once; call before any page view is freed).
pub fn guard_screen_time() {
    unsafe {
        let Some(class) = objc2::runtime::AnyClass::get(c"WKScreenTimeConfigurationObserver") else {
            return;
        };
        let sel = objc2::sel!(stopObserving);
        let method = objc2::ffi::class_getInstanceMethod(class, sel);
        if method.is_null() || STOP_OBSERVING.get().is_some() {
            return;
        }
        let Some(old) = objc2::ffi::method_getImplementation(method) else {
            return;
        };
        let _ = STOP_OBSERVING.set(std::mem::transmute::<objc2::runtime::Imp, Stop>(old));
        let imp = std::mem::transmute::<Stop, objc2::runtime::Imp>(stop_observing);
        objc2::ffi::method_setImplementation(method, imp);
    }
}

/// Whether the guard is in place (for tests).
#[cfg(debug_assertions)]
pub fn screen_time_guarded() -> bool {
    STOP_OBSERVING.get().is_some()
}

unsafe extern "C-unwind" fn stop_observing(this: &AnyObject, cmd: objc2::runtime::Sel) {
    caught(|| unsafe {
        if let Some(original) = STOP_OBSERVING.get() {
            original(this, cmd);
        }
    });
}

/// Run a callback from AppKit or WebKit, stopping any Objective-C exception
/// it raises there: unwinding through these Rust frames would abort the app
/// (release builds abort on unwind; 0.16.0 crashed this way), where AppKit
/// would have logged it and carried on. Every `extern "C-unwind"` method we
/// add or swap in runs its body in here.
fn caught(handler: impl FnOnce()) {
    if let Err(exception) = objc2::exception::catch(std::panic::AssertUnwindSafe(handler)) {
        RAISED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        eprintln!("native callback raised: {exception:?}");
    }
}

/// `caught` for a body with a result: `fallback` if it raised.
fn caught_or<T>(fallback: T, body: impl FnOnce() -> T) -> T {
    let mut result = None;
    caught(|| result = Some(body()));
    result.unwrap_or(fallback)
}

/// How many exceptions `caught` stopped (for tests).
pub static RAISED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

unsafe extern "C-unwind" fn view_moved(this: &AnyObject, cmd: objc2::runtime::Sel, event: &AnyObject) {
    caught(|| unsafe {
        if shows_at(this, event)
            && let Some(original) = VIEW_MOVED.get()
        {
            original(this, cmd, event);
        }
    });
}

unsafe extern "C-unwind" fn observer_moved(this: &AnyObject, cmd: objc2::runtime::Sel, event: &AnyObject) {
    caught(|| unsafe { observer_event(this, cmd, event, &OBSERVER_MOVED) });
}

unsafe extern "C-unwind" fn observer_entered(this: &AnyObject, cmd: objc2::runtime::Sel, event: &AnyObject) {
    caught(|| unsafe { observer_event(this, cmd, event, &OBSERVER_ENTERED) });
}

unsafe extern "C-unwind" fn observer_exited(this: &AnyObject, cmd: objc2::runtime::Sel, event: &AnyObject) {
    caught(|| unsafe {
        let me = this as *const AnyObject as usize;
        COVERED.lock().unwrap().retain(|&o| o != me);
        if let Some(original) = OBSERVER_EXITED.get() {
            original(this, cmd, event);
        }
    });
}

/// A move or enter for an observer's view: passed on only where the view
/// shows. The first one dropped tells the view the mouse left.
unsafe fn observer_event(
    this: &AnyObject,
    cmd: objc2::runtime::Sel,
    event: &AnyObject,
    original: &std::sync::OnceLock<MouseEvent>,
) {
    unsafe {
        let me = this as *const AnyObject as usize;
        if observer_shows_at(this, event) {
            COVERED.lock().unwrap().retain(|&o| o != me);
            if let Some(original) = original.get() {
                original(this, cmd, event);
            }
            return;
        }
        let newly_covered = {
            let mut covered = COVERED.lock().unwrap();
            let newly = !covered.contains(&me);
            if newly {
                covered.push(me);
            }
            newly
        };
        // A mouse-exited event: WebKit asks it for its tracking area, which
        // raises for any other kind.
        if newly_covered
            && let Some(exited) = OBSERVER_EXITED.get()
            && let Some(exit) = exit_event(event)
        {
            exited(this, objc2::sel!(mouseExited:), &exit);
        }
    }
}

/// A mouse-exited event where and when `event` happened.
unsafe fn exit_event(event: &AnyObject) -> Option<Retained<AnyObject>> {
    unsafe {
        let location: NSPoint = msg_send![event, locationInWindow];
        let flags: usize = msg_send![event, modifierFlags];
        let timestamp: f64 = msg_send![event, timestamp];
        let window: isize = msg_send![event, windowNumber];
        let exit: *mut AnyObject = msg_send![
            objc2::class!(NSEvent),
            enterExitEventWithType: 9usize, // NSEventTypeMouseExited
            location: location,
            modifierFlags: flags,
            timestamp: timestamp,
            windowNumber: window,
            context: ptr::null::<AnyObject>(),
            eventNumber: 0isize,
            trackingNumber: 0isize,
            userData: ptr::null_mut::<std::ffi::c_void>()
        ];
        Retained::retain(exit)
    }
}

/// Whether the view on top where `event` happened is (in) the view that
/// `observer` tracks the mouse for.
pub unsafe fn observer_shows_at(observer: &AnyObject, event: &AnyObject) -> bool {
    unsafe {
        let Some(mut view) = hit_view(event) else {
            return true;
        };
        loop {
            let areas: *mut AnyObject = msg_send![&*view, trackingAreas];
            let count: usize = msg_send![&*areas, count];
            for i in 0..count {
                let area: *mut AnyObject = msg_send![&*areas, objectAtIndex: i];
                let owner: *mut AnyObject = msg_send![&*area, owner];
                if std::ptr::eq(owner, observer) {
                    return true;
                }
            }
            let parent: *mut AnyObject = msg_send![&*view, superview];
            match Retained::retain(parent) {
                Some(parent) => view = parent,
                None => return false,
            }
        }
    }
}

/// The view on top where `event` happened, if it's in a window.
unsafe fn hit_view(event: &AnyObject) -> Option<Retained<AnyObject>> {
    unsafe {
        let window: *mut AnyObject = msg_send![event, window];
        let content: *mut AnyObject = msg_send![window.as_ref()?, contentView];
        let content = content.as_ref()?;
        let location: NSPoint = msg_send![event, locationInWindow];
        // hitTest: takes a point in the superview's coordinates.
        let parent: *mut AnyObject = msg_send![content, superview];
        let null: *const AnyObject = ptr::null();
        let point: NSPoint = match parent.as_ref() {
            Some(parent) => msg_send![parent, convertPoint: location, fromView: null],
            None => location,
        };
        let hit: *mut AnyObject = msg_send![content, hitTest: point];
        Retained::retain(hit)
    }
}

/// Whether `view` is (in) the view on top where `event` happened.
pub unsafe fn shows_at(view: &AnyObject, event: &AnyObject) -> bool {
    unsafe {
        match hit_view(event) {
            Some(hit) => msg_send![&*hit, isDescendantOf: view],
            None => {
                let window: *mut AnyObject = msg_send![event, window];
                window.is_null()
            }
        }
    }
}

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
/// Whether a page is playing audio or capturing the camera or microphone.
pub fn page_in_use(webview: &Webview) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    let sent = webview.with_webview(move |p| {
        let mut busy = false;
        caught(|| unsafe {
            let view = &*(p.inner() as *const AnyObject);
            let has = |sel| -> bool { msg_send![view, respondsToSelector: sel] };
            if has(objc2::sel!(_isPlayingAudio)) {
                let playing: bool = msg_send![view, _isPlayingAudio];
                busy |= playing;
            }
            // WKMediaCaptureState: 0 none, 1 active, 2 muted.
            if has(objc2::sel!(cameraCaptureState)) {
                let state: isize = msg_send![view, cameraCaptureState];
                busy |= state != 0;
            }
            if has(objc2::sel!(microphoneCaptureState)) {
                let state: isize = msg_send![view, microphoneCaptureState];
                busy |= state != 0;
            }
        });
        let _ = tx.send(busy);
    });
    // Unknown counts as in use: better to keep a page than lose a call.
    sent.is_err() || rx.recv().unwrap_or(true)
}

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

/// Whether a key press (not a click) chose the menu item being handled, while
/// a terminal has the keyboard (the window's first responder is in the
/// shared terminal view). Main thread only: called from menu events.
pub fn key_pressed_in_terminal(window: &tauri::Window) -> bool {
    let Ok(ns_window) = window.ns_window() else {
        return false;
    };
    unsafe {
        let app: *mut AnyObject = msg_send![objc2::class!(NSApplication), sharedApplication];
        let event: *mut AnyObject = msg_send![app, currentEvent];
        let Some(event) = event.as_ref() else {
            return false;
        };
        let kind: usize = msg_send![event, type];
        // NSEventTypeKeyDown
        if kind != 10 {
            return false;
        }
        let window = &*(ns_window as *const AnyObject);
        let responder: *mut AnyObject = msg_send![window, firstResponder];
        match (responder.as_ref(), TERM_VIEW.get()) {
            (Some(responder), Some(term)) => {
                let is_view: bool = msg_send![responder, isKindOfClass: objc2::class!(NSView)];
                is_view && msg_send![responder, isDescendantOf: &*(term as *const AnyObject)]
            }
            _ => false,
        }
    }
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
// One translucent view over the content area while exploring tabs (⌃F). It
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

/// Make Ternt the handler for http and https links; macOS confirms with the
/// user. Points Launch Services at the running bundle, not whichever copy of
/// Ternt it finds first (old builds, a mounted DMG). Unbundled dev runs fall
/// back to the bundle id.
pub fn make_default_browser(bundle_id: &str) {
    let bundle: *mut AnyObject = unsafe { msg_send![objc2::class!(NSBundle), mainBundle] };
    let url: *mut NSURL = unsafe { msg_send![bundle, bundleURL] };
    let bundled = unsafe { url.as_ref() }
        .and_then(|url| url.path())
        .is_some_and(|path| path.to_string().ends_with(".app"));
    if !bundled {
        let id = NSString::from_str(bundle_id);
        for scheme in ["http", "https"] {
            let scheme = NSString::from_str(scheme);
            let status = unsafe { LSSetDefaultHandlerForURLScheme(&*scheme, &*id) };
            if status != 0 {
                eprintln!("failed to become the default browser for {scheme}: {status}");
            }
        }
        return;
    }
    // One confirmation covers http, https and web pages.
    let workspace: *mut AnyObject =
        unsafe { msg_send![objc2::class!(NSWorkspace), sharedWorkspace] };
    let scheme = NSString::from_str("http");
    let done = block2::RcBlock::new(|err: *mut AnyObject| {
        if !err.is_null() {
            eprintln!("failed to become the default browser");
        }
    });
    let _: () = unsafe {
        msg_send![workspace, setDefaultApplicationAtURL: url, toOpenURLsWithScheme: &*scheme, completionHandler: &*done]
    };
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
                // Private API: make sure this WebKit still has it.
                let has = |o: &AnyObject, sel| -> bool { msg_send![o, respondsToSelector: sel] };
                if !has(view, objc2::sel!(_inspector)) {
                    let _ = tx.send(true);
                    return;
                }
                let inspector: *mut AnyObject = msg_send![view, _inspector];
                let done = inspector.as_ref().is_some_and(|inspector| {
                    let visible: bool = msg_send![inspector, isVisible];
                    if visible && has(inspector, objc2::sel!(detach)) {
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

// --- Downloads -----------------------------------------------------------------
//
// wry tells Tauri only a download's URL, when it starts and when it ends. Its
// download delegate methods now also hand Ternt the `WKDownload` itself (see
// `with_download`), for its progress, to cancel it, and to tell two
// downloads of the same URL apart. And a response the server marks
// `Content-Disposition: attachment` downloads, as in Safari: wry downloaded
// only types WebKit can't show, so an attached PDF or text file opened in
// the tab instead.

type DecideDestination = unsafe extern "C-unwind" fn(
    &AnyObject,
    objc2::runtime::Sel,
    &AnyObject,
    &AnyObject,
    &NSString,
    &block2::Block<dyn Fn(*const NSURL)>,
);
type DownloadDone = unsafe extern "C-unwind" fn(&AnyObject, objc2::runtime::Sel, &AnyObject);
type DownloadFailed = unsafe extern "C-unwind" fn(
    &AnyObject,
    objc2::runtime::Sel,
    &AnyObject,
    &AnyObject,
    *mut AnyObject,
);
type ResponsePolicy = unsafe extern "C-unwind" fn(
    &AnyObject,
    objc2::runtime::Sel,
    &AnyObject,
    &AnyObject,
    &block2::Block<dyn Fn(isize)>,
);

type SetDelegate = unsafe extern "C-unwind" fn(&AnyObject, objc2::runtime::Sel, *mut AnyObject);

static SET_DELEGATE: std::sync::OnceLock<SetDelegate> = std::sync::OnceLock::new();
static DECIDE_DESTINATION: std::sync::OnceLock<DecideDestination> = std::sync::OnceLock::new();
static DOWNLOAD_DONE: std::sync::OnceLock<DownloadDone> = std::sync::OnceLock::new();
static DOWNLOAD_FAILED: std::sync::OnceLock<DownloadFailed> = std::sync::OnceLock::new();
static RESPONSE_POLICY: std::sync::OnceLock<ResponsePolicy> = std::sync::OnceLock::new();

/// `WKNavigationResponsePolicyDownload`.
const RESPONSE_DOWNLOAD: isize = 2;

thread_local! {
    static CURRENT_DOWNLOAD: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
}

/// The download (a `WKDownload`) whose delegate method is running (wry's
/// download handlers run inside it), if any.
pub fn current_download() -> Option<Retained<AnyObject>> {
    CURRENT_DOWNLOAD.with(|d| d.borrow().clone())
}

/// Whether `webview` never showed a page: a link opened in a new tab (or
/// window) that turned out to be a file. On the main thread. (Ask the view,
/// not the download: a download in a view another page opened names the
/// opener's view as its own.)
pub fn shows_nothing(webview: &Webview) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = webview.with_webview(move |p| {
        let empty = caught_or(false, || unsafe {
            let view = &*(p.inner() as *const AnyObject);
            let list: *mut AnyObject = msg_send![view, backForwardList];
            let item: *mut AnyObject = match list.as_ref() {
                Some(list) => msg_send![list, currentItem],
                None => ptr::null_mut(),
            };
            item.is_null()
        });
        let _ = tx.send(empty);
    });
    rx.try_recv().unwrap_or(false)
}

/// Bytes received so far and the size (-1 when the server didn't say).
pub fn download_progress(download: &AnyObject) -> (i64, i64) {
    caught_or((0, -1), || unsafe {
        let progress: *mut AnyObject = msg_send![download, progress];
        match progress.as_ref() {
            Some(p) => (msg_send![p, completedUnitCount], msg_send![p, totalUnitCount]),
            None => (0, -1),
        }
    })
}

pub fn cancel_download(download: &AnyObject) {
    caught(|| unsafe {
        let none: Option<&block2::Block<dyn Fn(*mut AnyObject)>> = None;
        let _: () = msg_send![download, cancel: none];
    });
}

/// Hook wry's navigation and download delegates (once; call after the first
/// page view is made, when wry has defined its navigation delegate class).
pub fn guard_downloads(view: &AnyObject) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        let delegate: *mut AnyObject = msg_send![view, navigationDelegate];
        if let Some(delegate) = delegate.as_ref()
            && let Some(old) = swizzle(
                delegate.class(),
                objc2::sel!(webView:decidePolicyForNavigationResponse:decisionHandler:),
                std::mem::transmute::<ResponsePolicy, objc2::runtime::Imp>(response_policy),
            )
        {
            let _ = RESPONSE_POLICY.set(std::mem::transmute::<objc2::runtime::Imp, ResponsePolicy>(old));
        }
        // wry's download delegate class has a generated name: hook it when
        // wry first hands a download its delegate.
        let Some(class) = objc2::runtime::AnyClass::get(c"WKDownload") else {
            return;
        };
        if let Some(old) = swizzle(
            class,
            objc2::sel!(setDelegate:),
            std::mem::transmute::<SetDelegate, objc2::runtime::Imp>(set_delegate),
        ) {
            let _ = SET_DELEGATE.set(std::mem::transmute::<objc2::runtime::Imp, SetDelegate>(old));
        }
    });
}

unsafe extern "C-unwind" fn set_delegate(this: &AnyObject, cmd: objc2::runtime::Sel, delegate: *mut AnyObject) {
    caught(|| unsafe {
        if let Some(original) = SET_DELEGATE.get() {
            original(this, cmd, delegate);
        }
        if let Some(delegate) = delegate.as_ref() {
            guard_download_delegate(delegate.class());
        }
    });
}

/// Hook wry's download delegate class (once).
unsafe fn guard_download_delegate(class: &objc2::runtime::AnyClass) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        if let Some(old) = swizzle(
            class,
            objc2::sel!(download:decideDestinationUsingResponse:suggestedFilename:completionHandler:),
            std::mem::transmute::<DecideDestination, objc2::runtime::Imp>(decide_destination),
        ) {
            let _ = DECIDE_DESTINATION.set(std::mem::transmute::<objc2::runtime::Imp, DecideDestination>(old));
        }
        if let Some(old) = swizzle(
            class,
            objc2::sel!(downloadDidFinish:),
            std::mem::transmute::<DownloadDone, objc2::runtime::Imp>(download_done),
        ) {
            let _ = DOWNLOAD_DONE.set(std::mem::transmute::<objc2::runtime::Imp, DownloadDone>(old));
        }
        if let Some(old) = swizzle(
            class,
            objc2::sel!(download:didFailWithError:resumeData:),
            std::mem::transmute::<DownloadFailed, objc2::runtime::Imp>(download_failed),
        ) {
            let _ = DOWNLOAD_FAILED.set(std::mem::transmute::<objc2::runtime::Imp, DownloadFailed>(old));
        }
    });
}

/// Replace `class`'s method for `sel` with `imp`, returning the original.
unsafe fn swizzle(
    class: &objc2::runtime::AnyClass,
    sel: objc2::runtime::Sel,
    imp: objc2::runtime::Imp,
) -> Option<objc2::runtime::Imp> {
    unsafe {
        let method = objc2::ffi::class_getInstanceMethod(class, sel);
        if method.is_null() {
            return None;
        }
        let old = objc2::ffi::method_getImplementation(method)?;
        objc2::ffi::method_setImplementation(method, imp);
        Some(old)
    }
}

/// Run `body` with `download` as the current download.
fn with_download(download: &AnyObject, body: impl FnOnce()) {
    CURRENT_DOWNLOAD.with(|d| *d.borrow_mut() = Some(download.retain()));
    caught(body);
    CURRENT_DOWNLOAD.with(|d| *d.borrow_mut() = None);
}

unsafe extern "C-unwind" fn decide_destination(
    this: &AnyObject,
    cmd: objc2::runtime::Sel,
    download: &AnyObject,
    response: &AnyObject,
    filename: &NSString,
    handler: &block2::Block<dyn Fn(*const NSURL)>,
) {
    with_download(download, || unsafe {
        if let Some(original) = DECIDE_DESTINATION.get() {
            original(this, cmd, download, response, filename, handler);
        }
    });
}

unsafe extern "C-unwind" fn download_done(this: &AnyObject, cmd: objc2::runtime::Sel, download: &AnyObject) {
    with_download(download, || unsafe {
        if let Some(original) = DOWNLOAD_DONE.get() {
            original(this, cmd, download);
        }
    });
}

unsafe extern "C-unwind" fn download_failed(
    this: &AnyObject,
    cmd: objc2::runtime::Sel,
    download: &AnyObject,
    error: &AnyObject,
    resume_data: *mut AnyObject,
) {
    with_download(download, || unsafe {
        if let Some(original) = DOWNLOAD_FAILED.get() {
            original(this, cmd, download, error, resume_data);
        }
    });
}

unsafe extern "C-unwind" fn response_policy(
    this: &AnyObject,
    cmd: objc2::runtime::Sel,
    web_view: &AnyObject,
    response: &AnyObject,
    handler: &block2::Block<dyn Fn(isize)>,
) {
    let attachment = caught_or(false, || unsafe {
        let response: *mut AnyObject = msg_send![response, response];
        let Some(response) = response.as_ref() else {
            return false;
        };
        let is_http: bool = msg_send![response, isKindOfClass: objc2::class!(NSHTTPURLResponse)];
        if !is_http {
            return false;
        }
        let name = NSString::from_str("Content-Disposition");
        let value: *mut NSString = msg_send![response, valueForHTTPHeaderField: &*name];
        value.as_ref().is_some_and(|v| is_attachment(&v.to_string()))
    });
    if attachment {
        handler.call((RESPONSE_DOWNLOAD,));
        return;
    }
    caught(|| unsafe {
        if let Some(original) = RESPONSE_POLICY.get() {
            original(this, cmd, web_view, response, handler);
        }
    });
}

/// Whether a `Content-Disposition` header says to save the file.
fn is_attachment(header: &str) -> bool {
    header
        .split(';')
        .next()
        .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("attachment"))
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
pub fn prepare_page(
    app: &AppHandle,
    label: &str,
) -> impl FnOnce(tauri::webview::PlatformWebview) + Send + 'static {
    let _ = APP.set(app.clone());
    let label = label.to_string();
    move |webview| prepare_page_view(webview, label)
}

/// The app, for delegate methods that call back into it.
static APP: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();
/// Page views' labels by their `WKWebView`, for `webViewDidClose:`.
static PAGE_LABELS: std::sync::Mutex<Vec<(usize, String)>> = std::sync::Mutex::new(Vec::new());

fn prepare_page_view(webview: tauri::webview::PlatformWebview, label: String) {
    {
        let mut labels = PAGE_LABELS.lock().unwrap();
        // A freed view's address can come back for a new one.
        labels.retain(|(view, l)| *view != webview.inner() as usize && *l != label);
        labels.push((webview.inner() as usize, label));
    }
    unsafe {
        let view = &*(webview.inner() as *const AnyObject);
        let config: *mut AnyObject = msg_send![view, configuration];
        let controller: *mut AnyObject = msg_send![&*config, userContentController];
        let _: () = msg_send![&*controller, removeAllUserScripts];

        let delegate: *mut AnyObject = msg_send![view, UIDelegate];
        if let Some(delegate) = delegate.as_ref() {
            add_js_dialogs(delegate.class());
            guard_new_windows(delegate.class());
            guard_media(delegate.class());
            // WebKit notes which methods a delegate has when it's set:
            // set it again so it sees the added ones.
            let _: () = msg_send![view, setUIDelegate: delegate];
        }
        guard_downloads(view);
        guard_link_clicks(view);
    }
}

/// The label of page view `view` (a `WKWebView`), if it's one of ours.
fn page_label(view: &AnyObject) -> Option<String> {
    let view = view as *const AnyObject as usize;
    PAGE_LABELS
        .lock()
        .unwrap()
        .iter()
        .find(|(v, _)| *v == view)
        .map(|(_, label)| label.clone())
}

/// The URL a navigation action asks for (empty for `window.open()`).
unsafe fn action_url(action: &AnyObject) -> Option<String> {
    unsafe {
        let request: *mut AnyObject = msg_send![action, request];
        let url: *mut NSURL = msg_send![request.as_ref()?, URL];
        Some(url.as_ref()?.absoluteString()?.to_string())
    }
}

// --- Camera, microphone and screen ------------------------------------------
//
// wry grants every site the camera and microphone without asking. Instead a
// site asks once, in a sheet like Safari's, and is remembered if allowed
// (Settings → General lists them). Screen sharing goes to macOS's picker,
// which is the asking.

type MediaPermission = unsafe extern "C-unwind" fn(
    &AnyObject,
    objc2::runtime::Sel,
    &AnyObject,
    &AnyObject,
    &AnyObject,
    isize,
    &block2::Block<dyn Fn(isize)>,
);

type DisplayPermission = unsafe extern "C-unwind" fn(
    &AnyObject,
    objc2::runtime::Sel,
    &AnyObject,
    &AnyObject,
    &AnyObject,
    objc2::runtime::Bool,
    &block2::Block<dyn Fn(isize)>,
);

/// `WKPermissionDecision`.
const PERMISSION_GRANT: isize = 1;
const PERMISSION_DENY: isize = 2;
/// `WKMediaCaptureType`.
const CAPTURE_CAMERA: isize = 0;
const CAPTURE_MICROPHONE: isize = 1;
/// `_WKDisplayCapturePermissionDecisionScreenPrompt`: macOS's picker.
const DISPLAY_PROMPT: isize = 1;

/// Hook wry's UI delegate class's capture permissions (once).
unsafe fn guard_media(class: &objc2::runtime::AnyClass) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        swizzle(
            class,
            objc2::sel!(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:),
            std::mem::transmute::<MediaPermission, objc2::runtime::Imp>(media_permission),
        );
        // WebKit's private method (Safari's): without it, getDisplayMedia()
        // is refused.
        objc2::ffi::class_addMethod(
            class as *const _ as *mut objc2::runtime::AnyClass,
            objc2::sel!(_webView:requestDisplayCapturePermissionForOrigin:initiatedByFrame:withSystemAudio:decisionHandler:),
            std::mem::transmute::<DisplayPermission, objc2::runtime::Imp>(display_permission),
            c"v@:@@@B@?".as_ptr(),
        );
    });
}

/// `https://meet.google.com` (with a port when it isn't the default).
unsafe fn origin_string(origin: &AnyObject) -> (String, String) {
    unsafe {
        let protocol: *mut NSString = msg_send![origin, protocol];
        let host: *mut NSString = msg_send![origin, host];
        let port: isize = msg_send![origin, port];
        let protocol = protocol.as_ref().map(|p| p.to_string()).unwrap_or_default();
        let host = host.as_ref().map(|h| h.to_string()).unwrap_or_default();
        let origin = match port {
            0 => format!("{protocol}://{host}"),
            port => format!("{protocol}://{host}:{port}"),
        };
        (origin, host)
    }
}

unsafe extern "C-unwind" fn media_permission(
    _this: &AnyObject,
    _cmd: objc2::runtime::Sel,
    web_view: &AnyObject,
    origin: &AnyObject,
    _frame: &AnyObject,
    kind: isize,
    decision: &block2::Block<dyn Fn(isize)>,
) {
    let asked = caught_or(false, || unsafe {
        let Some(app) = APP.get() else {
            return false;
        };
        let (origin, host) = origin_string(origin);
        if crate::prefs::media_allowed(app, &origin) {
            decision.call((PERMISSION_GRANT,));
            return true;
        }
        let what = match kind {
            CAPTURE_CAMERA => "camera",
            CAPTURE_MICROPHONE => "microphone",
            _ => "camera and microphone",
        };
        let alert: Retained<AnyObject> = msg_send![objc2::class!(NSAlert), new];
        let title = format!("Allow {host} to use your {what}?");
        let _: () = msg_send![&*alert, setMessageText: &*NSString::from_str(&title)];
        let info = "Ternt remembers your answer for this site. Settings → General lists the sites you've allowed.";
        let _: () = msg_send![&*alert, setInformativeText: &*NSString::from_str(info)];
        let _: *mut AnyObject = msg_send![&*alert, addButtonWithTitle: &*NSString::from_str("Allow")];
        let _: *mut AnyObject = msg_send![&*alert, addButtonWithTitle: &*NSString::from_str("Don\u{2019}t Allow")];
        let decision = decision.copy();
        let app = app.clone();
        js_dialog_show(web_view, &alert, move |allow| {
            if allow {
                crate::prefs::allow_media(&app, &origin);
            }
            decision.call((if allow { PERMISSION_GRANT } else { PERMISSION_DENY },));
        });
        true
    });
    if !asked {
        decision.call((PERMISSION_DENY,));
    }
}

unsafe extern "C-unwind" fn display_permission(
    _this: &AnyObject,
    _cmd: objc2::runtime::Sel,
    _web_view: &AnyObject,
    _origin: &AnyObject,
    _frame: &AnyObject,
    _system_audio: objc2::runtime::Bool,
    decision: &block2::Block<dyn Fn(isize)>,
) {
    decision.call((DISPLAY_PROMPT,));
}

// --- New windows and modified clicks -----------------------------------------
//
// A page opening a window without a size (`target=_blank`, `window.open()`)
// gets a tab whose view WebKit loads the request in: a real window the page
// can point somewhere after opening it blank, post a form into, or reach
// through `window.opener`. Tauri's new-window handler can only return a
// whole window, so wry's delegate method is hooked here. Sized popups
// (sign-in) still go to the handler (`browser::on_new_window`).

type CreateWebView = unsafe extern "C-unwind" fn(
    &AnyObject,
    objc2::runtime::Sel,
    &AnyObject,
    &AnyObject,
    &AnyObject,
    &AnyObject,
) -> *mut AnyObject;

static CREATE_WEB_VIEW: std::sync::OnceLock<CreateWebView> = std::sync::OnceLock::new();

type ActionPolicy = unsafe extern "C-unwind" fn(
    &AnyObject,
    objc2::runtime::Sel,
    &AnyObject,
    &AnyObject,
    &block2::Block<dyn Fn(isize)>,
);

static ACTION_POLICY: std::sync::OnceLock<ActionPolicy> = std::sync::OnceLock::new();

/// Hook wry's UI delegate class's `createWebView…` (once).
unsafe fn guard_new_windows(class: &objc2::runtime::AnyClass) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        if let Some(old) = swizzle(
            class,
            objc2::sel!(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:),
            std::mem::transmute::<CreateWebView, objc2::runtime::Imp>(create_web_view),
        ) {
            let _ = CREATE_WEB_VIEW.set(std::mem::transmute::<objc2::runtime::Imp, CreateWebView>(old));
        }
    });
}

unsafe extern "C-unwind" fn create_web_view(
    this: &AnyObject,
    cmd: objc2::runtime::Sel,
    web_view: &AnyObject,
    configuration: &AnyObject,
    action: &AnyObject,
    features: &AnyObject,
) -> *mut AnyObject {
    let adopted = caught_or(None, || unsafe {
        let width: *mut AnyObject = msg_send![features, width];
        let height: *mut AnyObject = msg_send![features, height];
        if !width.is_null() && !height.is_null() {
            return None;
        }
        let opener = page_label(web_view)?.strip_prefix("tab-")?.parse().ok()?;
        let url = action_url(action).filter(|u| !u.is_empty());
        let url = tauri::Url::parse(url.as_deref().unwrap_or("about:blank")).ok()?;
        let configuration = Retained::retain(
            configuration as *const AnyObject as *mut objc2_web_kit::WKWebViewConfiguration,
        )?;
        let view = browser::adopt_new_window(APP.get()?, opener, url, configuration)?;
        let (tx, rx) = std::sync::mpsc::channel();
        // On the main thread, this runs at once.
        view.with_webview(move |p| {
            let _ = tx.send(p.inner() as usize);
        })
        .ok()?;
        rx.try_recv().ok()
    });
    if let Some(view) = adopted {
        return view as *mut AnyObject;
    }
    caught_or(ptr::null_mut(), || unsafe {
        match CREATE_WEB_VIEW.get() {
            Some(original) => original(this, cmd, web_view, configuration, action, features),
            None => ptr::null_mut(),
        }
    })
}

/// Hook wry's navigation delegate's link decisions (once): ⌘-click and
/// middle-click open a link in a new tab, as in every browser; WebKit alone
/// follows it in place.
unsafe fn guard_link_clicks(view: &AnyObject) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        let delegate: *mut AnyObject = msg_send![view, navigationDelegate];
        if let Some(delegate) = delegate.as_ref()
            && let Some(old) = swizzle(
                delegate.class(),
                objc2::sel!(webView:decidePolicyForNavigationAction:decisionHandler:),
                std::mem::transmute::<ActionPolicy, objc2::runtime::Imp>(action_policy),
            )
        {
            let _ = ACTION_POLICY.set(std::mem::transmute::<objc2::runtime::Imp, ActionPolicy>(old));
        }
    });
}

/// `WKNavigationTypeLinkActivated`, `WKNavigationTypeFormSubmitted`.
const LINK_ACTIVATED: isize = 0;
const FORM_SUBMITTED: isize = 1;
/// `WKNavigationActionPolicyCancel`.
const ACTION_CANCEL: isize = 0;
/// `WKNavigationAction.buttonNumber` for the middle button (a mask).
const MIDDLE_BUTTON: isize = 1 << 2;

unsafe extern "C-unwind" fn action_policy(
    this: &AnyObject,
    cmd: objc2::runtime::Sel,
    web_view: &AnyObject,
    action: &AnyObject,
    handler: &block2::Block<dyn Fn(isize)>,
) {
    // The link, and whether to show its tab (⇧).
    let new_tab = caught_or(None, || unsafe {
        let kind: isize = msg_send![action, navigationType];
        let flags: usize = msg_send![action, modifierFlags];
        let button: isize = msg_send![action, buttonNumber];
        let download: bool = msg_send![action, shouldPerformDownload];
        let command = flags & (1 << 20) != 0;
        if kind != LINK_ACTIVATED || download || !(command || button == MIDDLE_BUTTON) {
            return None;
        }
        let url = tauri::Url::parse(&action_url(action)?).ok()?;
        Some((url, flags & (1 << 17) != 0))
    });
    // A link or form in another view that targets this one by name.
    caught(|| unsafe {
        let kind: isize = msg_send![action, navigationType];
        let source: *mut AnyObject = msg_send![action, sourceFrame];
        let source_view: *mut AnyObject = match source.as_ref() {
            Some(frame) => msg_send![frame, webView],
            None => ptr::null_mut(),
        };
        if matches!(kind, LINK_ACTIVATED | FORM_SUBMITTED)
            && new_tab.is_none()
            && !source_view.is_null()
            && !ptr::eq(source_view, web_view)
            && let (Some(app), Some(label)) = (APP.get(), page_label(web_view))
        {
            browser::on_link_into(app, &label);
        }
    });
    if let Some((url, front)) = new_tab
        && let Some(app) = APP.get()
        && browser::open_link_in_new_tab(app, page_label(web_view).as_deref(), url, front)
    {
        handler.call((ACTION_CANCEL,));
        return;
    }
    caught(|| unsafe {
        if let Some(original) = ACTION_POLICY.get() {
            original(this, cmd, web_view, action, handler);
        }
    });
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
        let methods: [(&str, objc2::runtime::Imp, &std::ffi::CStr); 5] = [
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
            // `window.close()`: wry ignores it, leaving sign-in popups open.
            (
                "webViewDidClose:",
                std::mem::transmute::<DidClose, objc2::runtime::Imp>(did_close),
                c"v@:@",
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
    caught(|| unsafe {
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
    });
}

type DidClose = unsafe extern "C-unwind" fn(&AnyObject, objc2::runtime::Sel, &AnyObject);

/// A page called `window.close()` (and WebKit allowed it: the page was
/// opened by script).
unsafe extern "C-unwind" fn did_close(
    _this: &AnyObject,
    _cmd: objc2::runtime::Sel,
    web_view: &AnyObject,
) {
    caught(|| {
        if let (Some(app), Some(label)) = (APP.get(), page_label(web_view)) {
            let app = app.clone();
            // Closing views blocks on the main thread.
            std::thread::spawn(move || browser::on_page_closed(&app, &label));
        }
    });
}

unsafe extern "C-unwind" fn js_alert(
    _this: &AnyObject,
    _cmd: objc2::runtime::Sel,
    web_view: &AnyObject,
    message: &NSString,
    frame: &AnyObject,
    completion: &block2::Block<dyn Fn()>,
) {
    caught(|| unsafe {
        let alert = js_dialog_alert(frame, message, false);
        let completion = completion.copy();
        js_dialog_show(web_view, &alert, move |_| completion.call(()));
    });
}

unsafe extern "C-unwind" fn js_confirm(
    _this: &AnyObject,
    _cmd: objc2::runtime::Sel,
    web_view: &AnyObject,
    message: &NSString,
    frame: &AnyObject,
    completion: &block2::Block<dyn Fn(objc2::runtime::Bool)>,
) {
    caught(|| unsafe {
        let alert = js_dialog_alert(frame, message, true);
        let completion = completion.copy();
        js_dialog_show(web_view, &alert, move |ok| {
            completion.call((objc2::runtime::Bool::new(ok),))
        });
    });
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
    caught(|| unsafe {
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
    });
}

/// A page's icon links as `<link>` tags with absolute hrefs (`"`, `<` and `>`
/// escaped: an inline SVG's data URL holds them), read from the
/// live page (signed in, with links its scripts added). Only links for the
/// current appearance: GitHub names a light icon for dark mode, invisible
/// on a light sidebar. Runs in an isolated script world, so the page's own
/// scripts can't interfere.
pub async fn page_icon_links(webview: &Webview) -> Option<String> {
    const JS: &str = r#"[...document.querySelectorAll("link[rel][href]")]
        .filter((l) => !l.media || matchMedia(l.media).matches)
        .map((l) => `<link rel="${l.rel}" href="${l.href.replaceAll('"', "%22").replaceAll("<", "%3C").replaceAll(">", "%3E")}" sizes="${l.sizes?.value ?? ""}">`)
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

#[cfg(test)]
mod tests {
    #[test]
    fn attachments_are_saved_inline_ones_shown() {
        assert!(super::is_attachment("attachment"));
        assert!(super::is_attachment(r#"Attachment; filename="a.pdf""#));
        assert!(!super::is_attachment(r#"inline; filename="attachment.pdf""#));
        assert!(!super::is_attachment(""));
    }
}
