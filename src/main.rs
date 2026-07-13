mod credentials;
mod usage_api;

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBezierPath, NSColor, NSImage, NSMenu,
    NSMenuItem, NSStackView, NSStackViewDistribution, NSStatusBar, NSStatusItem, NSTextAlignment,
    NSTextField, NSUserInterfaceLayoutOrientation, NSVariableStatusItemLength, NSView, NSWorkspace,
};
use objc2_foundation::{
    NSArray, NSEdgeInsets, NSObject, NSPoint, NSRect, NSSize, NSString, NSTimeInterval, NSTimer,
    NSURL,
};

const REPO_URL: &str = "https://github.com/GuinsooRocky/ccbar";
const REFRESH_INTERVAL_SECS: NSTimeInterval = 300.0;

use usage_api::{UsageSnapshot, WindowState};

// Minimal FFI to libdispatch so a worker thread can hop UI work back to the
// main run loop after a blocking HTTP fetch — keeps the menu bar responsive
// even when the network stalls for the full 30 s timeout.
mod dispatch {
    use std::ffi::c_void;

    unsafe extern "C" {
        static _dispatch_main_q: c_void;
        fn dispatch_async_f(
            queue: *const c_void,
            context: *mut c_void,
            work: extern "C" fn(*mut c_void),
        );
    }

    pub fn on_main<F: FnOnce() + Send + 'static>(f: F) {
        let boxed: Box<Box<dyn FnOnce() + Send>> = Box::new(Box::new(f));
        let ctx = Box::into_raw(boxed) as *mut c_void;
        unsafe { dispatch_async_f(&_dispatch_main_q, ctx, run_boxed) };
    }

    extern "C" fn run_boxed(ctx: *mut c_void) {
        let boxed: Box<Box<dyn FnOnce() + Send>> =
            unsafe { Box::from_raw(ctx as *mut Box<dyn FnOnce() + Send>) };
        boxed();
    }
}

enum MenuState {
    Ok(UsageSnapshot),
    Error(String),
}

struct AppState {
    access_token: String,
    menu: Retained<NSMenu>,
    controller: Retained<RefreshController>,
    status_item: Retained<NSStatusItem>,
}

thread_local! {
    static APP_STATE: RefCell<Option<AppState>> = const { RefCell::new(None) };
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CCBarRefreshController"]
    struct RefreshController;

    impl RefreshController {
        #[unsafe(method(handleRefresh:))]
        fn handle_refresh(&self, _sender: *mut AnyObject) {
            refresh_now(true);
            // Reopen the menu on the next run-loop pass so the user can watch
            // the data update in place after a manual ⌘R press.
            dispatch::on_main(|| {
                APP_STATE.with(|cell| {
                    let borrow = cell.borrow();
                    let Some(state) = borrow.as_ref() else { return };
                    let Some(mtm) = MainThreadMarker::new() else { return };
                    let Some(btn) = state.status_item.button(mtm) else { return };
                    unsafe { let _: () = msg_send![&*btn, performClick: std::ptr::null_mut::<AnyObject>()]; }
                });
            });
        }

        // Used by NSTimer — refreshes data silently without reopening the menu.
        #[unsafe(method(handleTimerRefresh:))]
        fn handle_timer_refresh(&self, _sender: *mut AnyObject) {
            refresh_now(false);
        }

        #[unsafe(method(openRepo:))]
        fn open_repo(&self, _sender: *mut AnyObject) {
            open_url(REPO_URL);
        }

        // Indirection around `terminate:` — macOS auto-assigns an ⊠ icon
        // to menu items wired directly to that selector.
        #[unsafe(method(handleQuit:))]
        fn handle_quit(&self, _sender: *mut AnyObject) {
            if let Some(mtm) = MainThreadMarker::new() {
                unsafe { NSApplication::sharedApplication(mtm).terminate(None) };
            }
        }
    }
);

impl RefreshController {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

fn main() {
    // Gatekeeper copies unsigned + quarantined apps into a read-only ephemeral
    // /private/var/folders/.../AppTranslocation/ path when launched from
    // outside /Applications; repeated security checks in that sandbox pin a
    // CPU core at 100%. Bail out with a dialog instead of hanging.
    if let Ok(exe) = std::env::current_exe() {
        if exe.to_string_lossy().contains("/AppTranslocation/") {
            let script = r#"display dialog "ccbar 正在 macOS AppTranslocation 沙盒中运行，会持续占用 100% CPU。

ccbar is running inside macOS AppTranslocation sandbox, which pins CPU at 100%.

修复 / Fix:
1. 把 ccbar.app 拖到 /Applications  (move to /Applications)
2. 在终端执行 (run in Terminal):
   xattr -rd com.apple.quarantine /Applications/ccbar.app
3. 从启动台重新打开  (relaunch from Launchpad)" with title "ccbar" with icon stop buttons {"OK"} default button 1"#;
            let _ = std::process::Command::new("osascript")
                .args(["-e", script])
                .status();
            eprintln!("ccbar: refusing to run from AppTranslocation sandbox ({exe:?})");
            std::process::exit(1);
        }
    }

    let creds = match credentials::Credentials::load() {
        Ok(c) => {
            let tail: String = c
                .access_token
                .chars()
                .rev()
                .take(6)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            eprintln!(
                "ccbar: credentials ok — source={:?} token=...{} tier={:?} has_user_profile={}",
                c.source,
                tail,
                c.rate_limit_tier,
                c.has_user_profile_scope(),
            );
            c
        }
        Err(e) => {
            eprintln!("ccbar: credentials error: {e}");
            // Still boot so the user sees an error menu rather than silent exit.
            run_with_error(format!("credentials: {e}"));
            return;
        }
    };

    let mtm = MainThreadMarker::new().expect("must run on main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let status_bar = NSStatusBar::systemStatusBar();
    let status_item = status_bar.statusItemWithLength(NSVariableStatusItemLength);

    let menu = NSMenu::new(mtm);
    let controller = RefreshController::new(mtm);
    status_item.setMenu(Some(&menu));

    APP_STATE.with(|cell| {
        *cell.borrow_mut() = Some(AppState {
            access_token: creds.access_token.clone(),
            menu: menu.clone(),
            controller: controller.clone(),
            status_item: status_item.clone(),
        });
    });

    let initial = fetch_state(&creds.access_token);
    update_icon(&status_item, mtm, &initial);
    populate_menu(&menu, mtm, &initial, &controller);

    unsafe {
        NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
            REFRESH_INTERVAL_SECS,
            &*controller,
            sel!(handleTimerRefresh:),
            None,
            true,
        );
    }

    app.run();
}

fn run_with_error(msg: String) {
    let mtm = MainThreadMarker::new().expect("must run on main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let status_bar = NSStatusBar::systemStatusBar();
    let status_item = status_bar.statusItemWithLength(NSVariableStatusItemLength);

    let menu = NSMenu::new(mtm);
    let controller = RefreshController::new(mtm);
    let state = MenuState::Error(msg);
    update_icon(&status_item, mtm, &state);
    populate_menu(&menu, mtm, &state, &controller);
    status_item.setMenu(Some(&menu));

    let _keep = status_item;
    app.run();
}

fn fetch_state(token: &str) -> MenuState {
    match usage_api::fetch_usage(token) {
        Ok(snap) => {
            eprintln!(
                "ccbar: usage ok — session={:.0}% used, weekly={}, {}",
                snap.session.fraction_used * 100.0,
                snap.weekly
                    .as_ref()
                    .map(|w| format!("{:.0}%", w.fraction_used * 100.0))
                    .unwrap_or_else(|| "n/a".into()),
                snap.scoped
                    .as_ref()
                    .map(|s| format!(
                        "{}={:.0}%",
                        s.label.to_lowercase(),
                        s.state.fraction_used * 100.0
                    ))
                    .unwrap_or_else(|| "scoped=n/a".into()),
            );
            MenuState::Ok(snap)
        }
        Err(e) => {
            let mut chain = format!("{e}");
            let mut src: Option<&dyn std::error::Error> = std::error::Error::source(&e);
            while let Some(s) = src {
                chain.push_str(&format!(" <- {s}"));
                src = s.source();
            }
            eprintln!("ccbar: usage error: {chain}");
            MenuState::Error(format!("usage API: {e}"))
        }
    }
}

fn refresh_now(manual: bool) {
    static IN_FLIGHT: AtomicBool = AtomicBool::new(false);
    if IN_FLIGHT.swap(true, Ordering::AcqRel) {
        // Previous fetch still pending — skip overlap.
        return;
    }

    let token = APP_STATE.with(|cell| cell.borrow().as_ref().map(|s| s.access_token.clone()));
    let Some(token) = token else {
        IN_FLIGHT.store(false, Ordering::Release);
        return;
    };

    std::thread::spawn(move || {
        let mut state = fetch_state(&token);
        // On 401 the OAuth token may have rotated — reload once from keychain.
        if matches!(state, MenuState::Error(ref m) if m.contains("token expired")) {
            if let Ok(fresh) = credentials::Credentials::load() {
                state = fetch_state(&fresh.access_token);
                let new_token = fresh.access_token.clone();
                dispatch::on_main(move || {
                    APP_STATE.with(|cell| {
                        if let Some(s) = cell.borrow_mut().as_mut() {
                            s.access_token = new_token;
                        }
                    });
                });
            }
        }
        dispatch::on_main(move || {
            // Timer-triggered failures are silent — keep last good data.
            if !manual && matches!(state, MenuState::Error(_)) {
                eprintln!("ccbar: timer refresh failed, keeping last state");
            } else {
                apply_state(state);
            }
            IN_FLIGHT.store(false, Ordering::Release);
        });
    });
}

fn apply_state(new_state: MenuState) {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("ccbar: apply_state called off main thread — skipping");
        return;
    };
    APP_STATE.with(|cell| {
        let borrow = cell.borrow();
        let Some(state) = &*borrow else { return };
        update_icon(&state.status_item, mtm, &new_state);
        populate_menu(&state.menu, mtm, &new_state, &state.controller);
    });
}

fn populate_menu(
    menu: &NSMenu,
    mtm: MainThreadMarker,
    state: &MenuState,
    controller: &RefreshController,
) {
    menu.removeAllItems();

    match state {
        MenuState::Ok(snap) => {
            let local = snap.fetched_at.with_timezone(&chrono::Local);
            let updated = format!("Updated {}", local.format("%H:%M"));
            add_row(menu, mtm, &["Claude", &updated]);
            menu.addItem(&NSMenuItem::separatorItem(mtm));

            add_window_section(menu, mtm, "Session", Some(&snap.session));
            menu.addItem(&NSMenuItem::separatorItem(mtm));

            add_window_section(menu, mtm, "Weekly", snap.weekly.as_ref());
            menu.addItem(&NSMenuItem::separatorItem(mtm));

            if let Some(scoped) = &snap.scoped {
                add_window_section(menu, mtm, &scoped.label, Some(&scoped.state));
                menu.addItem(&NSMenuItem::separatorItem(mtm));
            }
        }
        MenuState::Error(msg) => {
            add_label(menu, mtm, "Claude  error");
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            for chunk in wrap(msg, 60) {
                add_label(menu, mtm, chunk);
            }
            menu.addItem(&NSMenuItem::separatorItem(mtm));
        }
    }

    let refresh = NSMenuItem::new(mtm);
    refresh.setTitle(&NSString::from_str("Refresh"));
    refresh.setKeyEquivalent(&NSString::from_str("r"));
    unsafe {
        refresh.setTarget(Some(controller));
        refresh.setAction(Some(sel!(handleRefresh:)));
    }
    menu.addItem(&refresh);

    let about = NSMenuItem::new(mtm);
    about.setTitle(&NSString::from_str("Open on GitHub"));
    unsafe {
        about.setTarget(Some(controller));
        about.setAction(Some(sel!(openRepo:)));
    }
    menu.addItem(&about);

    let quit = NSMenuItem::new(mtm);
    quit.setTitle(&NSString::from_str("Quit"));
    quit.setKeyEquivalent(&NSString::from_str("q"));
    unsafe {
        quit.setTarget(Some(controller));
        quit.setAction(Some(sel!(handleQuit:)));
    }
    menu.addItem(&quit);
}

fn window_summary(w: &WindowState) -> String {
    let left = w.percent_left();
    let reset = w.reset_label();
    if reset.is_empty() {
        format!("{left}%")
    } else {
        format!("{left}%  {reset}")
    }
}

fn add_window_section(
    menu: &NSMenu,
    mtm: MainThreadMarker,
    label: &str,
    w: Option<&WindowState>,
) {
    match w {
        Some(w) => {
            add_row(menu, mtm, &[label, &make_bar(w.fraction_used, 8), &window_summary(w)]);
        }
        None => {
            add_row(menu, mtm, &[label, "no data"]);
        }
    }
}

fn make_bar(fraction_used: f64, width: usize) -> String {
    let filled = (fraction_used.clamp(0.0, 1.0) * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);
    "█".repeat(filled) + &"░".repeat(empty)
}

fn load_symbol(name: &str) -> Option<Retained<NSImage>> {
    unsafe {
        NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(name),
            None,
        )
    }
}

fn update_icon(status_item: &NSStatusItem, mtm: MainThreadMarker, state: &MenuState) {
    let Some(button) = status_item.button(mtm) else { return };
    match state {
        MenuState::Ok(snap) => {
            let session = snap.session.fraction_used.clamp(0.0, 1.0);
            let weekly = snap
                .weekly
                .as_ref()
                .map(|w| w.fraction_used.clamp(0.0, 1.0))
                .unwrap_or(0.0);
            let img = render_meter_icon(session, weekly);
            button.setImage(Some(&img));
            button.setTitle(&NSString::from_str(""));
        }
        MenuState::Error(_) => {
            if let Some(img) = load_symbol("exclamationmark.triangle.fill") {
                button.setImage(Some(&img));
                button.setTitle(&NSString::from_str(""));
            }
        }
    }
}

/// Two stacked horizontal meters: thicker top = session (5h), thinner bottom = weekly (7d).
/// Rendered as a template image so macOS auto-inverts for dark/light menu bar.
fn render_meter_icon(session_used: f64, weekly_used: f64) -> Retained<NSImage> {
    let size = NSSize::new(22.0, 14.0);
    let image = unsafe { NSImage::initWithSize(NSImage::alloc(), size) };

    unsafe { image.lockFocus() };

    let margin_x: f64 = 2.0;
    let bar_width: f64 = size.width - margin_x * 2.0;
    let top_y: f64 = 8.0;
    let top_h: f64 = 3.5;
    let bot_y: f64 = 3.5;
    let bot_h: f64 = 1.5;

    // Tracks (subtle outline so empty bars are still visible on busy wallpapers).
    let track_color = unsafe { NSColor::labelColor().colorWithAlphaComponent(0.25) };
    unsafe { track_color.set() };
    let top_track = NSRect::new(NSPoint::new(margin_x, top_y), NSSize::new(bar_width, top_h));
    let bot_track = NSRect::new(NSPoint::new(margin_x, bot_y), NSSize::new(bar_width, bot_h));
    unsafe { NSBezierPath::fillRect(top_track) };
    unsafe { NSBezierPath::fillRect(bot_track) };

    // Filled portions.
    unsafe { NSColor::labelColor().set() };
    let top_fill = NSRect::new(
        NSPoint::new(margin_x, top_y),
        NSSize::new(bar_width * session_used, top_h),
    );
    let bot_fill = NSRect::new(
        NSPoint::new(margin_x, bot_y),
        NSSize::new(bar_width * weekly_used, bot_h),
    );
    unsafe { NSBezierPath::fillRect(top_fill) };
    unsafe { NSBezierPath::fillRect(bot_fill) };

    unsafe { image.unlockFocus() };
    image.setTemplate(true);
    image
}

fn open_url(s: &str) {
    let Some(url) = (unsafe { NSURL::URLWithString(&NSString::from_str(s)) }) else {
        eprintln!("ccbar: invalid URL: {s}");
        return;
    };
    let workspace = unsafe { NSWorkspace::sharedWorkspace() };
    unsafe { workspace.openURL(&url) };
}

fn add_label(menu: &NSMenu, mtm: MainThreadMarker, text: impl AsRef<str>) {
    let item = NSMenuItem::new(mtm);
    item.setTitle(&NSString::from_str(text.as_ref()));
    item.setEnabled(false);
    menu.addItem(&item);
}

/// Horizontal row of N secondary-color labels laid out via NSStackView with
/// `.equalSpacing` — first hugs left, last hugs right, the rest spaced evenly.
/// The last column is pinned to a fixed width + right-aligned so variable
/// summary strings (e.g. "100%" vs "76% 3h 35m") don't shift the bar column.
fn add_row(menu: &NSMenu, mtm: MainThreadMarker, cols: &[&str]) {
    const ROW_WIDTH: f64 = 280.0;
    const ROW_HEIGHT: f64 = 26.0;
    const H_INSET: f64 = 14.0; // matches native menu item horizontal padding
    const RIGHT_COL_WIDTH: f64 = 90.0; // wide enough for "XX%  Xd XXh"

    let n = cols.len();
    let views: Vec<Retained<NSView>> = cols
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
            unsafe { label.setTextColor(Some(&NSColor::secondaryLabelColor())) };
            if i + 1 == n && n >= 2 {
                unsafe { label.setAlignment(NSTextAlignment::Right) };
                let anchor = unsafe { label.widthAnchor() };
                let constraint =
                    unsafe { anchor.constraintEqualToConstant(RIGHT_COL_WIDTH) };
                unsafe { constraint.setActive(true) };
            }
            // NSTextField -> NSControl -> NSView.
            label.into_super().into_super()
        })
        .collect();
    let views: Retained<NSArray<NSView>> = NSArray::from_retained_slice(&views);

    let stack = unsafe { NSStackView::stackViewWithViews(&views, mtm) };
    unsafe {
        stack.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
        stack.setDistribution(NSStackViewDistribution::EqualSpacing);
        stack.setEdgeInsets(NSEdgeInsets {
            top: 4.0,
            left: H_INSET,
            bottom: 4.0,
            right: H_INSET,
        });
        stack.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(ROW_WIDTH, ROW_HEIGHT),
        ));
    }

    let item = NSMenuItem::new(mtm);
    item.setView(Some(&stack));
    item.setEnabled(false);
    menu.addItem(&item);
}

fn wrap(s: &str, width: usize) -> Vec<String> {
    let mut out = vec![];
    let mut line = String::new();
    for word in s.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}
