mod activity;
mod codex_credentials;
mod codex_usage_api;
mod credentials;
mod usage_api;

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
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
const ACTIVITY_INTERVAL_SECS: NSTimeInterval = 60.0;

use activity::ProviderActivity;
use codex_usage_api::CodexUsageSnapshot;
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

#[derive(Clone)]
enum ProviderState<T> {
    Ok(T),
    Unavailable,
    Error(String),
}

#[derive(Clone)]
struct MenuState {
    claude: ProviderState<UsageSnapshot>,
    codex: ProviderState<CodexUsageSnapshot>,
}

struct AppState {
    menu: Retained<NSMenu>,
    controller: Retained<RefreshController>,
    status_item: Retained<NSStatusItem>,
    current: MenuState,
    activity: ProviderActivity,
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

        #[unsafe(method(handleActivityTimer:))]
        fn handle_activity_timer(&self, _sender: *mut AnyObject) {
            refresh_activity();
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

    let mtm = MainThreadMarker::new().expect("must run on main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let status_bar = NSStatusBar::systemStatusBar();
    let status_item = status_bar.statusItemWithLength(NSVariableStatusItemLength);

    let menu = NSMenu::new(mtm);
    let controller = RefreshController::new(mtm);
    status_item.setMenu(Some(&menu));

    let initial = fetch_state();
    let activity = activity::detect();
    update_icon(&status_item, mtm, &initial, activity);
    populate_menu(&menu, mtm, &initial, activity, &controller);

    APP_STATE.with(|cell| {
        *cell.borrow_mut() = Some(AppState {
            menu: menu.clone(),
            controller: controller.clone(),
            status_item: status_item.clone(),
            current: initial,
            activity,
        });
    });

    unsafe {
        NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
            REFRESH_INTERVAL_SECS,
            &*controller,
            sel!(handleTimerRefresh:),
            None,
            true,
        );
        NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
            ACTIVITY_INTERVAL_SECS,
            &*controller,
            sel!(handleActivityTimer:),
            None,
            true,
        );
    }

    app.run();
}

fn fetch_state() -> MenuState {
    let (claude, codex) = std::thread::scope(|scope| {
        let claude = scope.spawn(fetch_claude_state);
        let codex = scope.spawn(fetch_codex_state);
        (
            claude.join().expect("Claude fetch thread panicked"),
            codex.join().expect("Codex fetch thread panicked"),
        )
    });
    MenuState { claude, codex }
}

fn fetch_claude_state() -> ProviderState<UsageSnapshot> {
    let credentials = match credentials::Credentials::load() {
        Ok(credentials) => {
            eprintln!(
                "ccbar: Claude credentials ok — source={:?} tier={:?} has_user_profile={}",
                credentials.source,
                credentials.rate_limit_tier,
                credentials.has_user_profile_scope(),
            );
            credentials
        }
        Err(credentials::CredentialsError::NotFound) => {
            eprintln!("ccbar: Claude credentials not found — hiding provider");
            return ProviderState::Unavailable;
        }
        Err(error) => {
            eprintln!("ccbar: Claude credentials error: {error}");
            return ProviderState::Error(format!("credentials: {error}"));
        }
    };

    match usage_api::fetch_usage(&credentials.access_token) {
        Ok(snap) => {
            eprintln!(
                "ccbar: Claude usage ok — session={:.0}% used, weekly={}, {}",
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
            ProviderState::Ok(snap)
        }
        Err(e) => {
            let mut chain = format!("{e}");
            let mut src: Option<&dyn std::error::Error> = std::error::Error::source(&e);
            while let Some(s) = src {
                chain.push_str(&format!(" <- {s}"));
                src = s.source();
            }
            eprintln!("ccbar: Claude usage error: {chain}");
            ProviderState::Error(format!("usage API: {e}"))
        }
    }
}

fn fetch_codex_state() -> ProviderState<CodexUsageSnapshot> {
    let credentials = match codex_credentials::CodexCredentials::load() {
        Ok(credentials) => {
            eprintln!(
                "ccbar: Codex credentials ok — source={:?} account_id={}",
                credentials.source,
                if credentials.account_id.is_some() {
                    "yes"
                } else {
                    "no"
                },
            );
            credentials
        }
        Err(
            codex_credentials::CodexCredentialsError::NotFound
            | codex_credentials::CodexCredentialsError::MissingOAuthToken,
        ) => {
            eprintln!("ccbar: Codex subscription credentials not found — hiding provider");
            return ProviderState::Unavailable;
        }
        Err(error) => {
            eprintln!("ccbar: Codex credentials error: {error}");
            return ProviderState::Error(format!("credentials: {error}"));
        }
    };

    match codex_usage_api::fetch_usage(&credentials.access_token, credentials.account_id.as_deref())
    {
        Ok(snapshot) => {
            let usage = snapshot
                .windows
                .iter()
                .map(|window| {
                    format!(
                        "{}={:.0}%",
                        window.label.to_lowercase(),
                        window.state.fraction_used * 100.0
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            eprintln!("ccbar: Codex usage ok — {usage}");
            ProviderState::Ok(snapshot)
        }
        Err(error) => {
            eprintln!("ccbar: Codex usage error: {error}");
            ProviderState::Error(format!("usage API: {error}"))
        }
    }
}

fn refresh_now(manual: bool) {
    static IN_FLIGHT: AtomicBool = AtomicBool::new(false);
    if IN_FLIGHT.swap(true, Ordering::AcqRel) {
        // Previous fetch still pending — skip overlap.
        return;
    }

    let ready = APP_STATE.with(|cell| cell.borrow().is_some());
    if !ready {
        IN_FLIGHT.store(false, Ordering::Release);
        return;
    }

    std::thread::spawn(move || {
        let state = fetch_state();
        dispatch::on_main(move || {
            apply_state(state, !manual);
            IN_FLIGHT.store(false, Ordering::Release);
        });
    });
}

fn refresh_activity() {
    let detected = activity::detect();
    let mut newly_active = false;
    APP_STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(state) = borrow.as_mut() else { return };
        if state.activity == detected {
            return;
        }

        newly_active = (!state.activity.claude && detected.claude)
            || (!state.activity.codex && detected.codex);
        state.activity = detected;
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        update_icon(&state.status_item, mtm, &state.current, detected);
        populate_menu(
            &state.menu,
            mtm,
            &state.current,
            detected,
            &state.controller,
        );
    });

    if newly_active {
        refresh_now(false);
    }
}

fn apply_state(mut new_state: MenuState, keep_previous_on_error: bool) {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("ccbar: apply_state called off main thread — skipping");
        return;
    };
    APP_STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(state) = borrow.as_mut() else { return };
        if keep_previous_on_error {
            if matches!(new_state.claude, ProviderState::Error(_))
                && matches!(state.current.claude, ProviderState::Ok(_))
            {
                eprintln!("ccbar: Claude timer refresh failed, keeping last state");
                new_state.claude = state.current.claude.clone();
            }
            if matches!(new_state.codex, ProviderState::Error(_))
                && matches!(state.current.codex, ProviderState::Ok(_))
            {
                eprintln!("ccbar: Codex timer refresh failed, keeping last state");
                new_state.codex = state.current.codex.clone();
            }
        }
        update_icon(&state.status_item, mtm, &new_state, state.activity);
        populate_menu(
            &state.menu,
            mtm,
            &new_state,
            state.activity,
            &state.controller,
        );
        state.current = new_state;
    });
}

fn populate_menu(
    menu: &NSMenu,
    mtm: MainThreadMarker,
    state: &MenuState,
    activity: ProviderActivity,
    controller: &RefreshController,
) {
    menu.removeAllItems();

    let mut provider_count = 0;
    if activity.claude {
        match &state.claude {
            ProviderState::Ok(snap) => {
                provider_count += 1;
                let local = snap.fetched_at.with_timezone(&chrono::Local);
                let updated = format!("Updated {}", local.format("%H:%M"));
                add_row(menu, mtm, &["Claude", &updated]);
                add_window_section(menu, mtm, "Session", Some(&snap.session));
                add_window_section(menu, mtm, "Weekly", snap.weekly.as_ref());

                if let Some(scoped) = &snap.scoped {
                    add_window_section(menu, mtm, &scoped.label, Some(&scoped.state));
                }
            }
            ProviderState::Error(msg) => {
                provider_count += 1;
                add_label(menu, mtm, "Claude  error");
                for chunk in wrap(msg, 60) {
                    add_label(menu, mtm, chunk);
                }
            }
            ProviderState::Unavailable => {}
        }
    }

    if activity.codex {
        match &state.codex {
            ProviderState::Ok(snapshot) => {
                if provider_count > 0 {
                    menu.addItem(&NSMenuItem::separatorItem(mtm));
                }
                provider_count += 1;
                let local = snapshot.fetched_at.with_timezone(&chrono::Local);
                let updated = format!("Updated {}", local.format("%H:%M"));
                let title = snapshot
                    .plan
                    .as_deref()
                    .map(|plan| format!("Codex · {}", plan.to_uppercase()))
                    .unwrap_or_else(|| "Codex".into());
                add_row(menu, mtm, &[&title, &updated]);

                for window in &snapshot.windows {
                    add_window_section(menu, mtm, &window.label, Some(&window.state));
                }
            }
            ProviderState::Error(msg) => {
                if provider_count > 0 {
                    menu.addItem(&NSMenuItem::separatorItem(mtm));
                }
                provider_count += 1;
                add_label(menu, mtm, "Codex  error");
                for chunk in wrap(msg, 60) {
                    add_label(menu, mtm, chunk);
                }
            }
            ProviderState::Unavailable => {}
        }
    }

    if provider_count == 0 {
        add_label(menu, mtm, "No active Claude or Codex session");
    }
    menu.addItem(&NSMenuItem::separatorItem(mtm));

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

fn add_window_section(menu: &NSMenu, mtm: MainThreadMarker, label: &str, w: Option<&WindowState>) {
    match w {
        Some(w) => {
            add_row(
                menu,
                mtm,
                &[label, &make_bar(w.fraction_used, 8), &window_summary(w)],
            );
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
        NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None)
    }
}

fn update_icon(
    status_item: &NSStatusItem,
    mtm: MainThreadMarker,
    state: &MenuState,
    activity: ProviderActivity,
) {
    let Some(button) = status_item.button(mtm) else {
        return;
    };
    let mut meters = Vec::with_capacity(4);
    if activity.claude {
        if let ProviderState::Ok(snapshot) = &state.claude {
            meters.push(snapshot.session.fraction_used);
            meters.push(
                snapshot
                    .weekly
                    .as_ref()
                    .map(|window| window.fraction_used)
                    .unwrap_or(0.0),
            );
        }
    }
    if activity.codex {
        if let ProviderState::Ok(snapshot) = &state.codex {
            meters.extend(
                snapshot
                    .windows
                    .iter()
                    .take(2)
                    .map(|window| window.state.fraction_used),
            );
        }
    }

    if meters.is_empty() {
        let symbol = if activity.claude || activity.codex {
            "exclamationmark.triangle.fill"
        } else {
            "pause.circle.fill"
        };
        if let Some(img) = load_symbol(symbol) {
            button.setImage(Some(&img));
            button.setTitle(&NSString::from_str(""));
        }
        return;
    }

    let img = render_meter_icon(&meters);
    button.setImage(Some(&img));
    button.setTitle(&NSString::from_str(""));
}

/// Packs one active provider into one pair of meters and both providers into four.
fn render_meter_icon(meters: &[f64]) -> Retained<NSImage> {
    let size = NSSize::new(22.0, 16.0);
    let image = unsafe { NSImage::initWithSize(NSImage::alloc(), size) };

    unsafe { image.lockFocus() };

    let margin_x: f64 = 2.0;
    let bar_width: f64 = size.width - margin_x * 2.0;
    let bar_height: f64 = 2.0;
    let positions: &[f64] = match meters.len() {
        1 => &[7.0],
        2 => &[9.5, 4.0],
        3 => &[11.0, 6.5, 2.0],
        _ => &[12.0, 9.0, 5.0, 2.0],
    };

    // Tracks (subtle outline so empty bars are still visible on busy wallpapers).
    let track_color = unsafe { NSColor::labelColor().colorWithAlphaComponent(0.25) };
    unsafe { track_color.set() };
    for &y in positions {
        let track = NSRect::new(
            NSPoint::new(margin_x, y),
            NSSize::new(bar_width, bar_height),
        );
        unsafe { NSBezierPath::fillRect(track) };
    }

    // Filled portions.
    unsafe { NSColor::labelColor().set() };
    for (&used, &y) in meters.iter().zip(positions) {
        let fill = NSRect::new(
            NSPoint::new(margin_x, y),
            NSSize::new(bar_width * used.clamp(0.0, 1.0), bar_height),
        );
        unsafe { NSBezierPath::fillRect(fill) };
    }

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
    const ROW_WIDTH: f64 = 320.0;
    const ROW_HEIGHT: f64 = 26.0;
    const H_INSET: f64 = 14.0; // matches native menu item horizontal padding
    const LEFT_COL_WIDTH: f64 = 110.0;
    const BAR_COL_WIDTH: f64 = 90.0;
    const RIGHT_COL_WIDTH: f64 = 90.0; // wide enough for "XX%  Xd XXh"

    let n = cols.len();
    let views: Vec<Retained<NSView>> = cols
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
            unsafe { label.setTextColor(Some(&NSColor::secondaryLabelColor())) };
            if n >= 3 && i == 0 {
                let anchor = unsafe { label.widthAnchor() };
                let constraint = unsafe { anchor.constraintEqualToConstant(LEFT_COL_WIDTH) };
                unsafe { constraint.setActive(true) };
            } else if n >= 3 && i == 1 {
                unsafe { label.setAlignment(NSTextAlignment::Center) };
                let anchor = unsafe { label.widthAnchor() };
                let constraint = unsafe { anchor.constraintEqualToConstant(BAR_COL_WIDTH) };
                unsafe { constraint.setActive(true) };
            }
            if i + 1 == n && n >= 2 {
                unsafe { label.setAlignment(NSTextAlignment::Right) };
                let anchor = unsafe { label.widthAnchor() };
                let constraint = unsafe { anchor.constraintEqualToConstant(RIGHT_COL_WIDTH) };
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
