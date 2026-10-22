// SPDX-License-Identifier: AGPL-3.0-or-later
//! fluxpeer desktop client — a pure-Rust iced GUI over the local daemon control
//! API (`fluxpeer up`, 127.0.0.1:41999 + token). WireGuard-style layout.
//!
//! Run: `FLUXPEER_DAEMON_TOKEN=$(sudo cat /etc/fluxpeer/daemon.token) cargo run -p fluxpeer-desktop`

mod i18n;
mod tray;
mod raster;
mod quit;
use i18n::{t, Lang};

use iced::widget::{button, checkbox, column, container, image, pick_list, row, scrollable, text, text_editor, text_input, Space};
use iced::{Color, Element, Font, Length, Task, Theme};
use serde_json::{json, Value};

const DAEMON_ADDR: &str = "127.0.0.1:41999";

/// Which top-level pane the detail side shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Route {
    #[default]
    Networks,
    Diagnostics,
}

/// Connection mode — a friendly front for the transport-policy config flags
/// (force_relay / relay_anytls / relay_bond). Auto = the direct-first ladder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ConnMode {
    Auto,
    AntiCensor,
    ForceRelay,
    Bonded,
}
impl ConnMode {
    const ALL: [ConnMode; 4] = [ConnMode::Auto, ConnMode::AntiCensor, ConnMode::ForceRelay, ConnMode::Bonded];
    fn from_net(n: &Net) -> ConnMode {
        if !n.force_relay {
            ConnMode::Auto
        } else if n.relay_anytls {
            ConnMode::AntiCensor
        } else if n.relay_bond {
            ConnMode::Bonded
        } else {
            ConnMode::ForceRelay
        }
    }
    /// (force_relay, relay_anytls, relay_bond)
    fn flags(self) -> (bool, bool, bool) {
        match self {
            ConnMode::Auto => (false, false, false),
            ConnMode::AntiCensor => (true, true, false),
            ConnMode::ForceRelay => (true, false, false),
            ConnMode::Bonded => (true, false, true),
        }
    }
    fn key(self) -> &'static str {
        match self {
            ConnMode::Auto => "mode_auto",
            ConnMode::AntiCensor => "mode_anticensor",
            ConnMode::ForceRelay => "mode_forcerelay",
            ConnMode::Bonded => "mode_bonded",
        }
    }
}
/// pick_list item carrying its localized label (pick_list shows items via Display).
#[derive(Clone, PartialEq)]
struct ModeItem {
    mode: ConnMode,
    label: String,
}
impl std::fmt::Display for ModeItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label)
    }
}

/// Resolve the daemon control token: explicit env first, then the per-user and the
/// system config-dir token files (the latter is chowned to the user by Start daemon).
fn resolve_token() -> String {
    if let Ok(t) = std::env::var("FLUXPEER_DAEMON_TOKEN") {
        return t.trim().to_string();
    }
    for p in token_paths() {
        if let Ok(t) = std::fs::read_to_string(&p) {
            let t = t.trim().to_string();
            if !t.is_empty() {
                return t;
            }
        }
    }
    String::new()
}

/// Candidate `daemon.token` locations, most-specific first.
#[allow(clippy::vec_init_then_push)] // pushes are #[cfg]-gated, so vec![] doesn't fit
fn token_paths() -> Vec<std::path::PathBuf> {
    let mut v = Vec::new();
    #[cfg(unix)]
    v.push(std::path::PathBuf::from("/etc/fluxpeer/daemon.token"));
    #[cfg(windows)]
    if let Ok(pd) = std::env::var("PROGRAMDATA") {
        v.push(std::path::PathBuf::from(format!("{pd}\\fluxpeer\\daemon.token")));
    }
    v
}

/// macOS LaunchAgent plist path for "launch desktop at login".
#[cfg(target_os = "macos")]
fn login_plist() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(std::path::PathBuf::from(format!("{home}/Library/LaunchAgents/dev.fluxpeer.desktop.plist")))
}

// ---- #5 launch at login (macOS LaunchAgent) ----
#[cfg(target_os = "macos")]
fn login_at_login_enabled() -> bool {
    login_plist().map(|p| p.exists()).unwrap_or(false)
}
#[cfg(not(target_os = "macos"))]
fn login_at_login_enabled() -> bool {
    false
}

#[cfg(target_os = "macos")]
fn set_launch_at_login(on: bool) -> Result<(), String> {
    let plist = login_plist().ok_or("no HOME")?;
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let contents = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>dev.fluxpeer.desktop</string>
  <key>ProgramArguments</key><array><string>{}</string></array>
  <key>RunAtLoad</key><true/>
  <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
"#,
            exe.display()
        );
        if let Some(parent) = plist.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(&plist, contents).map_err(|e| format!("write plist: {e}"))?;
        let _ = std::process::Command::new("launchctl").args(["load", "-w"]).arg(&plist).status();
        Ok(())
    } else {
        let _ = std::process::Command::new("launchctl").args(["unload", "-w"]).arg(&plist).status();
        let _ = std::fs::remove_file(&plist);
        Ok(())
    }
}
#[cfg(not(target_os = "macos"))]
fn set_launch_at_login(_on: bool) -> Result<(), String> {
    Err("launch-at-login is macOS-only for now".into())
}

// ---- #1 daemon lifecycle: locate the daemon binary + start it with admin rights ----

/// Path to the `fluxpeer` daemon binary: a sibling of this desktop exe if present
/// (the packaged layout), else trust PATH.
fn daemon_bin() -> String {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let sib = dir.join("fluxpeer");
        if sib.exists() {
            return sib.to_string_lossy().to_string();
        }
    }
    "fluxpeer".to_string()
}

/// Start `fluxpeer up` with administrator rights (the TUN device needs root), then
/// hand the control token to the invoking user so this GUI can read it. macOS uses
/// the system auth dialog via osascript; other platforms instruct the user.
async fn start_daemon() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        let exe = daemon_bin();
        let user = std::env::var("USER").unwrap_or_default();
        // Background the daemon (detached fds so `do shell script` returns), wait for
        // it to write its token, then chown the token to the user (keeps it 0600).
        let inner = format!(
            "nohup '{exe}' up >/tmp/fluxpeer-daemon.log 2>&1 & sleep 1; chown {user} /etc/fluxpeer/daemon.token 2>/dev/null; true"
        );
        let script = format!(
            "do shell script \"{}\" with administrator privileges",
            inner.replace('\\', "\\\\").replace('"', "\\\"")
        );
        // osascript blocks on the system auth dialog → run it off the async executor.
        let out = tokio::task::spawn_blocking(move || std::process::Command::new("osascript").arg("-e").arg(&script).output())
            .await
            .map_err(|e| format!("join: {e}"))?
            .map_err(|e| format!("osascript: {e}"))?;
        if out.status.success() {
            Ok("daemon started".into())
        } else {
            let err = String::from_utf8_lossy(&out.stderr).to_string();
            if err.contains("-128") {
                Err("cancelled".into())
            } else {
                Err(format!("start failed: {}", err.trim()))
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Start the daemon manually:  sudo fluxpeer up".into())
    }
}

// ---- Design tokens: light + dark. The active mode lives in a thread-local set at
// the top of view() (iced style closures run on the same thread, same frame), so
// the getters below resolve to the right palette without threading state through.
thread_local! {
    static DARK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
fn is_dark() -> bool {
    DARK.with(|d| d.get())
}
fn set_dark(v: bool) {
    DARK.with(|d| d.set(v));
}
/// Pick the (r,g,b) for the current mode.
fn pick(light: (f32, f32, f32), dark: (f32, f32, f32)) -> Color {
    let (r, g, b) = if is_dark() { dark } else { light };
    Color::from_rgb(r, g, b)
}
fn sidebar_bg() -> Color {
    pick((0.965, 0.969, 0.976), (0.055, 0.063, 0.114)) // #f6f7f9 / #0e101d
}
fn card_bg() -> Color {
    pick((1.0, 1.0, 1.0), (0.086, 0.098, 0.157)) // #ffffff / #161928
}
fn border_c() -> Color {
    pick((0.902, 0.906, 0.925), (0.157, 0.184, 0.275)) // #e6e7ec / #282f46
}
fn text_c() -> Color {
    pick((0.106, 0.114, 0.141), (0.949, 0.961, 1.0)) // #1b1d24 / #f2f5ff
}
// Muted: AA-compliant on each canvas (~4.6:1 light, ~5:1 dark).
fn muted_c() -> Color {
    pick((0.420, 0.447, 0.502), (0.627, 0.671, 0.784)) // #6b7280 / #a0abc8
}
fn accent_c() -> Color {
    pick((0.388, 0.400, 0.945), (0.506, 0.549, 0.972)) // #6366f1 / #818cf8
}
fn accent_hover_c() -> Color {
    pick((0.329, 0.341, 0.878), (0.388, 0.400, 0.945)) // #5457e0 / #6366f1
}
fn accent_soft_c() -> Color {
    pick((0.933, 0.941, 0.996), (0.149, 0.176, 0.341)) // #eef0fe / #262d57
}
fn green_c() -> Color {
    pick((0.086, 0.639, 0.290), (0.204, 0.827, 0.600)) // #16a34a / #34d399
}
fn red_c() -> Color {
    pick((0.86, 0.20, 0.27), (0.973, 0.443, 0.443)) // #dc3445 / #f87171
}
fn subtle_c() -> Color {
    pick((0.941, 0.945, 0.957), (0.122, 0.141, 0.216) ) // #f0f1f4 / #1f2437
}

/// The window theme: a custom palette so the canvas behind the (transparent) detail
/// pane is our colour, not iced's stock light/dark grey.
fn theme_of(app: &App) -> Theme {
    if app.dark {
        Theme::custom(
            "fluxpeer-dark".to_string(),
            iced::theme::Palette {
                background: Color::from_rgb(0.043, 0.051, 0.090), // #0b0d17
                text: Color::from_rgb(0.949, 0.961, 1.0),
                primary: Color::from_rgb(0.506, 0.549, 0.972),
                success: Color::from_rgb(0.204, 0.827, 0.600),
                danger: Color::from_rgb(0.973, 0.443, 0.443),
            },
        )
    } else {
        Theme::custom(
            "fluxpeer-light".to_string(),
            iced::theme::Palette {
                background: Color::from_rgb(0.957, 0.961, 0.969), // #f4f5f7
                text: Color::from_rgb(0.106, 0.114, 0.141),
                primary: Color::from_rgb(0.388, 0.400, 0.945),
                success: Color::from_rgb(0.086, 0.639, 0.290),
                danger: Color::from_rgb(0.86, 0.20, 0.27),
            },
        )
    }
}

/// UI font with CJK coverage (iced's default font has no CJK glyphs, so CJK text
/// renders as mojibake). Uses the platform's system CJK family (also covers Latin).
fn ui_font() -> Font {
    #[cfg(target_os = "macos")]
    {
        Font::with_name("PingFang SC")
    }
    #[cfg(target_os = "windows")]
    {
        Font::with_name("Microsoft YaHei")
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Font::with_name("Noto Sans CJK SC")
    }
}

fn bold() -> Font {
    Font { weight: iced::font::Weight::Semibold, ..ui_font() }
}

/// Font for a rendered DATA VALUE. Monospace (Menlo/Consolas) is used for keys,
/// IPs, hashes and numbers — but `Font::MONOSPACE` has NO CJK glyphs, so any value
/// containing CJK (e.g. localized "never"/tunnel-mode text) renders as mojibake.
/// Fall back to the CJK-capable UI font for those. (Arrows are < U+2E80 and stay
/// monospace.)
fn vfont(s: &str) -> Font {
    if s.chars().any(|c| c >= '\u{2e80}') {
        ui_font()
    } else {
        Font::MONOSPACE
    }
}

// ---- iced style closures ----
fn card_style(_t: &Theme) -> iced::widget::container::Style {
    iced::widget::container::Style {
        background: Some(card_bg().into()),
        border: iced::Border { color: border_c(), width: 1.0, radius: 12.0.into() },
        ..Default::default()
    }
}
fn sidebar_style(_t: &Theme) -> iced::widget::container::Style {
    iced::widget::container::Style {
        background: Some(sidebar_bg().into()),
        border: iced::Border { color: border_c(), width: 1.0, radius: 0.0.into() },
        ..Default::default()
    }
}
fn primary_btn(_t: &Theme, s: iced::widget::button::Status) -> iced::widget::button::Style {
    let bg = if matches!(s, iced::widget::button::Status::Hovered | iced::widget::button::Status::Pressed) { accent_hover_c() } else { accent_c() };
    iced::widget::button::Style {
        background: Some(bg.into()),
        text_color: Color::WHITE,
        border: iced::Border { radius: 8.0.into(), ..Default::default() },
        ..Default::default()
    }
}
fn subtle_btn(_t: &Theme, s: iced::widget::button::Status) -> iced::widget::button::Style {
    let bg = if matches!(s, iced::widget::button::Status::Hovered) { border_c() } else { subtle_c() };
    iced::widget::button::Style {
        background: Some(bg.into()),
        text_color: text_c(),
        border: iced::Border { radius: 8.0.into(), ..Default::default() },
        ..Default::default()
    }
}
fn ghost_btn(_t: &Theme, s: iced::widget::button::Status) -> iced::widget::button::Style {
    let bg = if matches!(s, iced::widget::button::Status::Hovered) { Some(subtle_c().into()) } else { None };
    iced::widget::button::Style {
        background: bg,
        text_color: muted_c(),
        border: iced::Border { radius: 8.0.into(), ..Default::default() },
        ..Default::default()
    }
}
fn row_selected(_t: &Theme, _s: iced::widget::button::Status) -> iced::widget::button::Style {
    iced::widget::button::Style {
        background: Some(accent_soft_c().into()),
        text_color: accent_c(),
        border: iced::Border { radius: 8.0.into(), ..Default::default() },
        ..Default::default()
    }
}
fn row_plain(_t: &Theme, s: iced::widget::button::Status) -> iced::widget::button::Style {
    let bg = if matches!(s, iced::widget::button::Status::Hovered) { Some(subtle_c().into()) } else { None };
    iced::widget::button::Style {
        background: bg,
        text_color: text_c(),
        border: iced::Border { radius: 8.0.into(), ..Default::default() },
        ..Default::default()
    }
}
fn input_style(_t: &Theme, s: iced::widget::text_input::Status) -> iced::widget::text_input::Style {
    // Visible focus ring (keyboard a11y): indigo 1.5px border when focused.
    let border = match s {
        iced::widget::text_input::Status::Focused => iced::Border { color: accent_c(), width: 1.5, radius: 8.0.into() },
        _ => iced::Border { color: border_c(), width: 1.0, radius: 8.0.into() },
    };
    iced::widget::text_input::Style {
        background: card_bg().into(),
        border,
        icon: muted_c(),
        placeholder: muted_c(),
        value: text_c(),
        selection: accent_soft_c(),
    }
}
/// Config editor (text_editor): theme-aware so it doesn't show a stock white/grey
/// box that clashes with the dark canvas + cards.
fn editor_style(_t: &Theme, _s: iced::widget::text_editor::Status) -> iced::widget::text_editor::Style {
    iced::widget::text_editor::Style {
        background: card_bg().into(),
        border: iced::Border { color: border_c(), width: 1.0, radius: 10.0.into() },
        icon: muted_c(),
        placeholder: muted_c(),
        value: text_c(),
        selection: accent_soft_c(),
    }
}
/// Destructive action (Leave / confirm-leave): red text, tinted on hover.
fn danger_btn(_t: &Theme, s: iced::widget::button::Status) -> iced::widget::button::Style {
    let hover = matches!(s, iced::widget::button::Status::Hovered | iced::widget::button::Status::Pressed);
    let bg = if is_dark() {
        if hover { Color::from_rgb(0.27, 0.12, 0.15) } else { Color::from_rgb(0.20, 0.10, 0.12) }
    } else if hover {
        Color::from_rgb(0.99, 0.92, 0.93)
    } else {
        Color::from_rgb(0.99, 0.95, 0.96)
    };
    let edge = if is_dark() { Color::from_rgb(0.45, 0.22, 0.26) } else { Color::from_rgb(0.95, 0.80, 0.83) };
    iced::widget::button::Style {
        background: Some(bg.into()),
        text_color: red_c(),
        border: iced::Border { color: edge, width: 1.0, radius: 8.0.into() },
        ..Default::default()
    }
}
/// Soft amber card for confirmation / warning strips.
fn warn_style(_t: &Theme) -> iced::widget::container::Style {
    let (bg, edge) = if is_dark() {
        (Color::from_rgb(0.18, 0.145, 0.063), Color::from_rgb(0.42, 0.34, 0.16))
    } else {
        (Color::from_rgb(1.0, 0.976, 0.929), Color::from_rgb(0.96, 0.87, 0.70))
    };
    iced::widget::container::Style {
        background: Some(bg.into()),
        border: iced::Border { color: edge, width: 1.0, radius: 10.0.into() },
        ..Default::default()
    }
}

// The canonical fluxpeer mark (bagua1-mesh: octagon mesh + hub) — the SAME asset
// the.app icon is built from (packaging/macos/make-app.sh), so the nav, tray and
// app icon never drift.
const LOGO: &[u8] = include_bytes!("../assets/mark.svg");

// The tray lives on the main thread (macOS NSStatusItem requirement); iced runs
// update() on that same thread, so a thread-local lets update() rebuild the menu.
thread_local! {
    static TRAY: std::cell::RefCell<Option<tray::Tray>> = const { std::cell::RefCell::new(None) };
}

/// Rebuild the tray menu when the networks or language change (cheap signature guard).
fn sync_tray(app: &mut App) {
    let sig = format!(
        "{:?}|{}",
        app.lang,
        app.nets.iter().map(|n| format!("{}:{}", n.iface, n.up)).collect::<Vec<_>>().join(",")
    );
    if sig == app.tray_sig {
        return;
    }
    app.tray_sig = sig;
    let nets: Vec<(String, bool)> = app.nets.iter().map(|n| (n.iface.clone(), n.up)).collect();
    let lang = app.lang;
    TRAY.with(|c| {
        if let Some(tr) = c.borrow_mut().as_mut() {
            tr.update_menu(&nets, lang);
        }
    });
}

/// Rasterize the bagua mark to an RGBA image handle (the iced `svg` widget renders
/// the thin strokes/tiny dots near-invisibly at nav size). Cached in `App`.
fn logo_handle(size: u32) -> Option<image::Handle> {
    Some(image::Handle::from_rgba(size, size, raster::render(LOGO, size)?))
}

fn main() -> iced::Result {
    // Tray (menu-bar / system-tray). Kept alive for the whole program; if it can't
    // be created on this platform we just run without it.
    TRAY.with(|c| *c.borrow_mut() = tray::build());
    iced::application("Fluxpeer", update, view)
        .theme(theme_of)
        .default_font(ui_font())
        .window(iced::window::Settings {
            size: iced::Size::new(900.0, 600.0),
            exit_on_close_request: false, // closing hides to the tray; Quit is in the tray menu
            ..Default::default()
        })
        .subscription(|_| {
            iced::Subscription::batch([
                iced::time::every(std::time::Duration::from_secs(2)).map(|_| Message::Tick),
                iced::time::every(std::time::Duration::from_millis(300)).map(|_| Message::TrayPoll),
                iced::window::close_requests().map(Message::HideWindow),
            ])
        })
        .run_with(boot)
}

#[derive(Clone, Debug, Default)]
struct Net {
    iface: String,
    control: String,
    up: bool,
    exit_node: bool,
    use_exit: String,
    kill_switch: bool,
    last_error: Option<String>,
    force_relay: bool,
    relay_anytls: bool,
    relay_bond: bool,
}
#[derive(Clone, Debug, Default)]
struct Iface {
    last_error: Option<String>,
    address: String,
    public_key: String,
    listen_port: u64,
}
#[derive(Clone, Debug, Default)]
struct Peer {
    public_key: String,
    endpoint: String,
    allowed_ips: String,
    transport: String,
    last_handshake: u64,
    rx: u64,
    tx: u64,
}
#[derive(Clone, Debug, Default)]
struct Status {
    iface: Iface,
    peers: Vec<Peer>,
}
#[derive(Clone, Debug, Default)]
struct Snapshot {
    nets: Vec<Net>,
    status: Vec<(String, Status)>,
}

struct App {
    addr: String,
    token: String,
    lang: Lang,
    nets: Vec<Net>,
    status: std::collections::HashMap<String, Status>,
    selected: Option<String>,
    join_input: String,
    msg: String,
    action_error: Option<String>,
    prev: std::collections::HashMap<String, (u64, u64, std::time::Instant)>,
    rate: std::collections::HashMap<String, (f64, f64)>,
    s_mtu: String,
    s_dns: String,
    s_ep: String,
    s_exclude: String, // #3 split-exclude CIDRs (Settings input)
    editing: Option<String>, // iface whose raw config is open in the editor
    editor: text_editor::Content,
    importing: Option<ImportForm>, // a picked wg.conf awaiting control-server details
    tray_sig: String, // last tray-menu signature (nets + lang)
    logo: Option<image::Handle>,
    route: Route,                     // #6 which pane (Networks / Diagnostics)
    offline: bool,                    // #1 daemon unreachable (connection refused)
    starting: bool,                   // #1 a Start-daemon attempt is in flight
    confirm_leave: Option<String>,    // #2 iface awaiting a leave confirmation
    logs: String,                     // #6 tailed node log for the selected iface
    logs_iface: Option<String>,       // which iface app.logs belongs to
    launch_login: bool,               // #5 launch-at-login enabled (plist present)
    dark: bool,                       // dark theme on/off (system-detected at boot)
    quit: quit::QuitState,
    adding: bool,                     // the "add a network" pane is open
}

/// A parsed WireGuard config waiting on the admin's control-server + network so its
/// peers can be batch-registered (the GUI form behind the Import button).
#[derive(Clone, Debug)]
struct ImportForm {
    net_name: String,           // default network name (the [Interface] comment / file)
    peers: usize,               // peer count, for the summary line
    devices: Vec<Value>,        // already-built import device list (sent verbatim)
    server: String,             // control-server URL (input)
    password: String,           // admin password (input)
    network: String,            // target network id; blank = create a new one
}

#[derive(Debug, Clone)]
enum Message {
    Tick,
    TrayPoll,
    HideWindow(iced::window::Id),
    Loaded(Result<Snapshot, String>),
    Select(String),
    SetLang,
    ToggleTheme,
    Toggle(String, bool),
    JoinInput(String),
    Join,
    OpenAdd,
    CancelAdd,
    Import,
    Imported(Option<String>),
    SMtu(String),
    SDns(String),
    SEp(String),
    SaveSettings,
    OpenEdit(String),
    ConfigLoaded(Result<String, String>),
    EditorAction(text_editor::Action),
    SaveConfig,
    CancelEdit,
    ImpServer(String),
    ImpPassword(String),
    ImpNetwork(String),
    ImpSubmit,
    ImpCancel,
    ImpDone(Result<String, String>),
    Acted(Result<String, String>),
    DismissActionError,
    QuitFinished(Result<(), String>),
    QuitAnyway,
    QuitCancel,
    // #2 leave
    Leave(String),
    LeaveConfirm(String),
    LeaveCancel,
    // #3 split-exclude + exit toggle
    SExclude(String),
    ToggleExit(bool),
    ToggleUseExit(bool),
    ToggleKillSwitch(bool),
    SetConnMode(ConnMode),
    // #6 diagnostics / logs
    GoTo(Route),
    LoadLogs,
    LogsLoaded(Result<(String, String), String>),
    // #5 launch at login
    ToggleLaunchLogin(bool),
    // #1 daemon lifecycle
    StartDaemon,
    DaemonStarted(Result<String, String>),
}

/// Initial UI language: explicit `FLUXPEER_LANG` override, else the system locale
/// (`LANG`/`LC_ALL` starting `zh` selects Chinese), else English.
fn startup_lang() -> Lang {
    if let Ok(v) = std::env::var("FLUXPEER_LANG") {
        if v.starts_with("zh") {
            return Lang::Zh;
        }
        if v.starts_with("en") {
            return Lang::En;
        }
    }
    let loc = std::env::var("LANG").or_else(|_| std::env::var("LC_ALL")).unwrap_or_default();
    if loc.to_ascii_lowercase().starts_with("zh") {
        Lang::Zh
    } else {
        Lang::En
    }
}

/// Initial theme: `FLUXPEER_THEME=dark|light` override, else follow the OS (macOS
/// `AppleInterfaceStyle == Dark`), else light.
fn detect_dark() -> bool {
    if let Ok(v) = std::env::var("FLUXPEER_THEME") {
        if v.eq_ignore_ascii_case("dark") {
            return true;
        }
        if v.eq_ignore_ascii_case("light") {
            return false;
        }
    }
    #[cfg(target_os = "macos")]
    {
        return std::process::Command::new("defaults")
            .args(["read", "-g", "AppleInterfaceStyle"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().eq_ignore_ascii_case("Dark"))
            .unwrap_or(false);
    }
    #[allow(unreachable_code)]
    false
}

fn boot() -> (App, Task<Message>) {
    let addr = std::env::var("FLUXPEER_DAEMON_ADDR").unwrap_or_else(|_| DAEMON_ADDR.to_string());
    let token = resolve_token();
    let launch_login = login_at_login_enabled();
    let dark = detect_dark();
    let app = App {
        addr: addr.clone(),
        token: token.clone(),
        lang: startup_lang(),
        nets: Vec::new(),
        status: std::collections::HashMap::new(),
        selected: None,
        join_input: String::new(),
        msg: String::new(),
        action_error: None,
        prev: std::collections::HashMap::new(),
        rate: std::collections::HashMap::new(),
        s_mtu: String::new(),
        s_dns: String::new(),
        s_ep: String::new(),
        s_exclude: String::new(),
        editing: None,
        editor: text_editor::Content::new(),
        importing: None,
        tray_sig: String::new(),
        logo: logo_handle(96),
        route: if std::env::var("FLUXPEER_VIEW").is_ok_and(|v| v.starts_with("diag")) { Route::Diagnostics } else { Route::default() },
        offline: false,
        starting: false,
        confirm_leave: None,
        logs: String::new(),
        logs_iface: None,
        launch_login,
        dark,
        quit: quit::QuitState::default(),
        adding: std::env::var("FLUXPEER_VIEW").is_ok_and(|v| v.starts_with("add")),
    };
    (app, Task::perform(load(addr, token), Message::Loaded))
}

fn refresh(app: &App) -> Task<Message> {
    Task::perform(load(app.addr.clone(), app.token.clone()), Message::Loaded)
}

fn act(app: &App, body: Value) -> Task<Message> {
    Task::perform(daemon_action(app.addr.clone(), app.token.clone(), body), Message::Acted)
}

/// Reveal + focus the (possibly hidden) main window — used by the tray "Show" item.
fn show_window() -> Task<Message> {
    iced::window::get_latest().then(|maybe| match maybe {
        Some(id) => Task::batch([
            iced::window::change_mode(id, iced::window::Mode::Windowed),
            iced::window::gain_focus(id),
        ]),
        None => Task::none(),
    })
}

fn update(app: &mut App, message: Message) -> Task<Message> {
    match message {
        Message::Tick => {
            if app.editing.is_some() {
                Task::none()
            } else {
                refresh(app)
            }
        }
        Message::TrayPoll => {
            // Drain tray menu clicks (global receiver) and act.
            let mut out = Task::none();
            while let Ok(ev) = tray_icon::menu::MenuEvent::receiver().try_recv() {
                match ev.id.0.as_str() {
                    "quit" => {
                        if !app.quit.begin() { return Task::none(); }
                        return Task::perform(disconnect_for_quit(app.addr.clone(), app.token.clone()), Message::QuitFinished);
                    }
                    "connect_all" => {
                        let ts: Vec<_> = app.nets.iter().map(|n| act(app, json!({"cmd": "up", "iface": n.iface}))).collect();
                        out = Task::batch(ts);
                    }
                    "disconnect_all" => {
                        let ts: Vec<_> = app.nets.iter().map(|n| act(app, json!({"cmd": "down", "iface": n.iface}))).collect();
                        out = Task::batch(ts);
                    }
                    "show" => return show_window(),
                    id if id.starts_with("net:") => {
                        let iface = id.trim_start_matches("net:").to_string();
                        if let Some(n) = app.nets.iter().find(|x| x.iface == iface) {
                            let cmd = if n.up { "down" } else { "up" };
                            out = act(app, json!({"cmd": cmd, "iface": iface}));
                        }
                    }
                    _ => {}
                }
            }
            out
        }
        Message::HideWindow(id) => iced::window::change_mode(id, iced::window::Mode::Hidden),
        Message::Loaded(Ok(snap)) => {
            app.nets = snap.nets;
            app.status = snap.status.into_iter().collect();
            if app.selected.is_none() || !app.nets.iter().any(|n| Some(&n.iface) == app.selected.as_ref()) {
                app.selected = app.nets.first().map(|n| n.iface.clone());
            }
            let now = std::time::Instant::now();
            for (iface, st) in &app.status {
                let rx: u64 = st.peers.iter().map(|p| p.rx).sum();
                let tx: u64 = st.peers.iter().map(|p| p.tx).sum();
                if let Some((prx, ptx, pt)) = app.prev.get(iface) {
                    let dt = now.duration_since(*pt).as_secs_f64();
                    if dt > 0.2 {
                        app.rate.insert(iface.clone(), ((rx.saturating_sub(*prx) as f64) / dt, (tx.saturating_sub(*ptx) as f64) / dt));
                    }
                }
                app.prev.insert(iface.clone(), (rx, tx, now));
            }
            app.msg.clear();
            app.offline = false;
            app.starting = false;
            sync_tray(app);
            if app.route == Route::Diagnostics && app.selected.is_some() {
                load_logs(app)
            } else {
                Task::none()
            }
        }
        Message::Loaded(Err(e)) => {
            app.offline = daemon_unreachable(&e);
            app.msg = e;
            Task::none()
        }
        Message::Select(iface) => {
            app.selected = Some(iface);
            app.route = Route::Networks;
            app.s_mtu.clear();
            app.s_dns.clear();
            app.s_ep.clear();
            app.s_exclude.clear();
            app.confirm_leave = None;
            app.logs.clear();
            app.logs_iface = None;
            if app.route == Route::Diagnostics {
                load_logs(app)
            } else {
                Task::none()
            }
        }
        Message::SetLang => {
            app.lang = app.lang.toggle();
            sync_tray(app);
            Task::none()
        }
        Message::ToggleTheme => {
            app.dark = !app.dark;
            set_dark(app.dark);
            Task::none()
        }
        Message::Toggle(iface, up) => act(app, json!({"cmd": if up {"up"} else {"down"}, "iface": iface})),
        Message::JoinInput(s) => {
            app.join_input = s;
            Task::none()
        }
        Message::Join => {
            let tok = app.join_input.trim().to_string();
            if tok.is_empty() {
                app.msg = t(app.lang, "need_token");
                Task::none()
            } else {
                app.join_input.clear();
                app.adding = false;
                act(app, json!({"cmd": "join", "token": tok}))
            }
        }
        Message::OpenAdd => {
            app.adding = true;
            app.join_input.clear();
            app.msg.clear();
            Task::none()
        }
        Message::CancelAdd => {
            app.adding = false;
            app.join_input.clear();
            app.msg.clear();
            Task::none()
        }
        Message::Import => Task::perform(pick_file(), Message::Imported),
        Message::Imported(None) => Task::none(),
        Message::Imported(Some(content)) => {
            app.adding = false; // close the add pane; wg.conf opens its own form below
            let c = content.trim().to_string();
            if c.starts_with("fp://join/") {
                act(app, json!({"cmd": "join", "token": c}))
            } else if c.contains("[Interface]") {
                // A WireGuard config → open the control-side batch-import form.
                match fluxpeer_sdk::wgconf::parse(&c) {
                    Ok(conf) => {
                        let net_name = conf.interface.name.clone().unwrap_or_else(|| "imported".into());
                        match fluxpeer_sdk::build_import_devices(&conf, &net_name, false) {
                            Ok((devices, _warnings)) => {
                                // Prefill the control-server from a network we already know.
                                let server = app.nets.iter().map(|n| n.control.clone()).find(|s| !s.is_empty()).unwrap_or_default();
                                app.msg.clear();
                                app.importing = Some(ImportForm {
                                    net_name,
                                    peers: conf.peers.len(),
                                    devices,
                                    server,
                                    password: String::new(),
                                    network: String::new(),
                                });
                                Task::none()
                            }
                            Err(e) => {
                                app.msg = format!("wg.conf: {e}");
                                Task::none()
                            }
                        }
                    }
                    Err(e) => {
                        app.msg = format!("wg.conf: {e}");
                        Task::none()
                    }
                }
            } else if let Ok(cfg) = serde_json::from_str::<Value>(&c) {
                act(app, json!({"cmd": "import", "config": cfg}))
            } else {
                app.msg = "unrecognized file (need a fp://join token, a WireGuard .conf, or a node config JSON)".into();
                Task::none()
            }
        }
        Message::ImpServer(s) => {
            if let Some(f) = app.importing.as_mut() {
                f.server = s;
            }
            Task::none()
        }
        Message::ImpPassword(s) => {
            if let Some(f) = app.importing.as_mut() {
                f.password = s;
            }
            Task::none()
        }
        Message::ImpNetwork(s) => {
            if let Some(f) = app.importing.as_mut() {
                f.network = s;
            }
            Task::none()
        }
        Message::ImpCancel => {
            app.importing = None;
            Task::none()
        }
        Message::ImpSubmit => match app.importing.as_ref() {
            Some(f) if f.server.trim().is_empty() => {
                app.msg = t(app.lang, "server_required");
                Task::none()
            }
            Some(f) => Task::perform(
                ctl_import(f.server.clone(), f.password.clone(), f.network.clone(), f.net_name.clone(), f.devices.clone()),
                Message::ImpDone,
            ),
            None => Task::none(),
        },
        Message::ImpDone(Ok(m)) => {
            app.importing = None;
            app.msg = m;
            refresh(app)
        }
        Message::ImpDone(Err(e)) => {
            // keep the form open so the admin can fix the server/password and retry
            app.msg = e;
            Task::none()
        }
        Message::SMtu(s) => {
            app.s_mtu = s;
            Task::none()
        }
        Message::SDns(s) => {
            app.s_dns = s;
            Task::none()
        }
        Message::SEp(s) => {
            app.s_ep = s;
            Task::none()
        }
        Message::SaveSettings => {
            let Some(iface) = app.selected.clone() else { return Task::none() };
            let mut body = json!({"cmd": "settings", "iface": iface});
            if let Ok(m) = app.s_mtu.trim().parse::<i64>() {
                body["mtu"] = json!(m);
            }
            if !app.s_dns.trim().is_empty() {
                let dns: Vec<&str> = app.s_dns.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                body["dns"] = json!(dns);
            }
            if !app.s_ep.trim().is_empty() {
                body["endpoint"] = json!(app.s_ep.trim());
            }
            if !app.s_exclude.trim().is_empty() {
                let ex: Vec<&str> = app.s_exclude.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
                body["exclude_routes"] = json!(ex);
            }
            app.s_mtu.clear();
            app.s_dns.clear();
            app.s_ep.clear();
            app.s_exclude.clear();
            act(app, body)
        }
        Message::OpenEdit(iface) => {
            app.editing = Some(iface.clone());
            let (a, tk) = (app.addr.clone(), app.token.clone());
            Task::perform(
                async move {
                    let v = daemon_call(a, tk, json!({"cmd": "config", "iface": iface})).await?;
                    v.get("config")
                        .map(|c| serde_json::to_string_pretty(c).unwrap_or_default())
                        .ok_or_else(|| "no config".to_string())
                },
                Message::ConfigLoaded,
            )
        }
        Message::ConfigLoaded(Ok(text)) => {
            app.editor = text_editor::Content::with_text(&text);
            Task::none()
        }
        Message::ConfigLoaded(Err(e)) => {
            app.msg = e;
            app.editing = None;
            Task::none()
        }
        Message::EditorAction(a) => {
            app.editor.perform(a);
            Task::none()
        }
        Message::SaveConfig => {
            let Some(iface) = app.editing.clone() else { return Task::none() };
            match serde_json::from_str::<Value>(&app.editor.text()) {
                Ok(cfg) => {
                    app.editing = None;
                    act(app, json!({"cmd": "set-config", "iface": iface, "config": cfg}))
                }
                Err(e) => {
                    app.msg = format!("invalid JSON: {e}");
                    Task::none()
                }
            }
        }
        Message::CancelEdit => {
            app.editing = None;
            Task::none()
        }
        Message::QuitFinished(result) => {
            if app.quit.complete(result) { iced::exit() } else { show_window() }
        }
        Message::QuitAnyway => {
            if app.quit.can_force() { iced::exit() } else { Task::none() }
        }
        Message::QuitCancel => { app.quit = quit::QuitState::Idle; Task::none() }
        Message::Acted(Err(e)) => {
            app.action_error = Some(e);
            refresh(app)
        }
        Message::Acted(Ok(_)) => { app.action_error = None; refresh(app) },
        Message::DismissActionError => { app.action_error = None; Task::none() },

        // #2 leave network (two-step confirm)
        Message::Leave(iface) => {
            app.confirm_leave = Some(iface);
            Task::none()
        }
        Message::LeaveCancel => {
            app.confirm_leave = None;
            Task::none()
        }
        Message::LeaveConfirm(iface) => {
            app.confirm_leave = None;
            if app.selected.as_deref() == Some(iface.as_str()) {
                app.selected = None;
            }
            act(app, json!({"cmd": "leave", "iface": iface}))
        }

        // #3 split-exclude input + exit-node toggle
        Message::SExclude(s) => {
            app.s_exclude = s;
            Task::none()
        }
        Message::ToggleUseExit(on) => {
            let Some(settings) = use_exit_settings(app.selected.as_deref(), on) else { return Task::none() };
            act(app, settings)
        }
        Message::ToggleKillSwitch(on) => {
            let Some(settings) = kill_switch_settings(app.selected.as_deref(), on) else { return Task::none() };
            act(app, settings)
        }
        Message::ToggleExit(on) => {
            let Some(iface) = app.selected.clone() else { return Task::none() };
            act(app, json!({"cmd": "settings", "iface": iface, "exit_node": on}))
        }
        Message::SetConnMode(m) => {
            let Some(iface) = app.selected.clone() else { return Task::none() };
            let (fr, at, bd) = m.flags();
            act(app, json!({"cmd": "settings", "iface": iface, "force_relay": fr, "relay_anytls": at, "relay_bond": bd}))
        }

        // #6 diagnostics / logs
        Message::GoTo(r) => {
            app.route = r;
            if r == Route::Diagnostics && app.selected.is_some() {
                load_logs(app)
            } else {
                Task::none()
            }
        }
        Message::LoadLogs => load_logs(app),
        Message::LogsLoaded(Ok((iface, log))) => {
            app.logs_iface = Some(iface);
            app.logs = log;
            Task::none()
        }
        Message::LogsLoaded(Err(e)) => {
            app.msg = e;
            Task::none()
        }

        // #5 launch at login
        Message::ToggleLaunchLogin(on) => {
            match set_launch_at_login(on) {
                Ok(()) => app.launch_login = on,
                Err(e) => app.msg = e,
            }
            Task::none()
        }

        // #1 daemon lifecycle: start it (admin rights) then re-read the token + retry
        Message::StartDaemon => {
            app.starting = true;
            app.msg.clear();
            Task::perform(start_daemon(), Message::DaemonStarted)
        }
        Message::DaemonStarted(Ok(_)) => {
            app.token = resolve_token();
            app.msg.clear();
            refresh(app)
        }
        Message::DaemonStarted(Err(e)) => {
            app.starting = false;
            app.msg = e;
            Task::none()
        }
    }
}

/// Kick off a tail of the selected network's node log.
fn load_logs(app: &App) -> Task<Message> {
    let Some(iface) = app.selected.clone() else { return Task::none() };
    let (a, tk) = (app.addr.clone(), app.token.clone());
    Task::perform(
        async move {
            let v = daemon_call(a, tk, json!({"cmd": "logs", "iface": iface, "lines": 300})).await?;
            if let Some(e) = v.get("error").and_then(|x| x.as_str()) {
                return Err(e.to_string());
            }
            Ok((iface, v["log"].as_str().unwrap_or("").to_string()))
        },
        Message::LogsLoaded,
    )
}

/// True when a load error means the daemon itself is unreachable (so the GUI shows
/// the Start-daemon card rather than a raw error) — refused connection or no token.
fn daemon_unreachable(e: &str) -> bool {
    let e = e.to_ascii_lowercase();
    e.contains("refused") || e.contains("no daemon token") || e.contains("connection reset") || e.contains("os error 61")
}

// ---- view ----

fn view(app: &App) -> Element<'_, Message> {
    set_dark(app.dark); // style closures run later this frame on this thread
    if let quit::QuitState::Failed(error) = &app.quit {
        let dialog = container(column![
            text(format!("{}: {error}", t(app.lang, "disconnect_failed"))).size(18),
            row![
                button(text(t(app.lang, "quit_anyway"))).on_press(Message::QuitAnyway),
                button(text(t(app.lang, "cancel"))).on_press(Message::QuitCancel),
            ].spacing(12),
        ].spacing(20)).padding(24).max_width(620).style(card_style);
        return container(dialog).padding(24).center_x(Length::Fill).center_y(Length::Fill).into();
    }
    let content: Element<'_, Message> = if app.importing.is_some() {
        import_view(app)
    } else if app.adding {
        row![sidebar(app), add_view(app)].into()
    } else if app.editing.is_some() {
        editor_view(app)
    } else if app.route == Route::Diagnostics {
        row![sidebar(app), diagnostics_view(app)].into()
    } else {
        row![sidebar(app), detail(app)].into()
    };
    if let Some(error) = &app.action_error {
        let notice = container(row![
            text(error).size(13).color(text_c()).width(Length::Fill),
            button(text(t(app.lang, "dismiss_error"))).on_press(Message::DismissActionError).style(ghost_btn),
        ].spacing(12).align_y(iced::Alignment::Center)).padding(12).width(Length::Fill).style(warn_style);
        column![notice, content].into()
    } else { content }
}

/// A 1px horizontal rule that fills its slot (used in the "or" divider).
fn hr<'a>() -> Element<'a, Message> {
    container(Space::with_width(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &Theme| iced::widget::container::Style { background: Some(border_c().into()), ..Default::default() })
        .into()
}

/// The "add a network" pane: two clearly-separated, carded paths — paste an invite
/// token (the common case) OR import a file (auto-detects fp://join link /
/// WireGuard.conf / node config JSON). Replaces the cramped sidebar token field.
fn add_view(app: &App) -> Element<'_, Message> {
    let header = row![
        text(t(app.lang, "add_title")).size(20).font(bold()).color(text_c()),
        Space::with_width(Length::Fill),
        button(text(t(app.lang, "cancel")).size(13)).style(ghost_btn).padding([6, 12]).on_press(Message::CancelAdd),
    ]
    .align_y(iced::Alignment::Center);

    // path 1 — invite token (primary).
    let token_card = container(
        column![
            text(t(app.lang, "method_token")).size(13).font(bold()).color(text_c()),
            text(t(app.lang, "token_desc")).size(11).color(muted_c()),
            text_input(&t(app.lang, "paste_token"), &app.join_input)
                .on_input(Message::JoinInput)
                .on_submit(Message::Join)
                .size(13)
                .padding(10)
                .style(input_style),
            row![Space::with_width(Length::Fill), button(text(t(app.lang, "add_network")).size(13)).style(primary_btn).padding([9, 20]).on_press(Message::Join)],
        ]
        .spacing(12),
    )
    .padding(18)
    .width(Length::Fill)
    .style(card_style);

    let divider = row![hr(), text(t(app.lang, "or_sep")).size(11).color(muted_c()), hr()].spacing(14).align_y(iced::Alignment::Center);

    // path 2 — import a file.
    let file_card = container(
        column![
            text(t(app.lang, "method_file")).size(13).font(bold()).color(text_c()),
            text(t(app.lang, "file_supports")).size(11).color(muted_c()),
            row![button(text(t(app.lang, "choose_file")).size(13)).style(subtle_btn).padding([9, 20]).on_press(Message::Import), Space::with_width(Length::Fill)],
        ]
        .spacing(12),
    )
    .padding(18)
    .width(Length::Fill)
    .style(card_style);

    let mut col = column![header, text(t(app.lang, "add_intro")).size(13).color(muted_c()), token_card, divider, file_card]
        .spacing(18)
        .max_width(480);
    if !app.msg.is_empty() {
        col = col.push(text(app.msg.clone()).size(12).color(red_c()));
    }
    container(col).padding(32).center_x(Length::Fill).into()
}

/// The control-side wg.conf import form: shown after the user picks a WireGuard
/// `.conf` via the Import button. Collects the control-server + admin password +
/// target network, then batch-registers the parsed peers as devices.
fn import_view(app: &App) -> Element<'_, Message> {
    let Some(f) = app.importing.as_ref() else {
        return row![sidebar(app), detail(app)].into();
    };
    let summary = format!("{} · 1 {} + {} peer(s)", f.net_name, t(app.lang, "interface"), f.peers);
    let s_server = t(app.lang, "ctl_server_url");
    let s_pw = t(app.lang, "admin_password");
    let s_net = t(app.lang, "target_network");
    let mut col = column![
        text(t(app.lang, "import_wg_title")).size(18).font(bold()).color(text_c()),
        text(summary).size(13).color(muted_c()),
        text_input(&s_server, &f.server).on_input(Message::ImpServer).size(13).padding(8).style(input_style),
        text_input(&s_pw, &f.password).on_input(Message::ImpPassword).secure(true).size(13).padding(8).style(input_style),
        text_input(&s_net, &f.network).on_input(Message::ImpNetwork).size(13).padding(8).style(input_style),
        row![
            button(text(t(app.lang, "do_import")).size(13)).style(primary_btn).padding([8, 16]).on_press(Message::ImpSubmit),
            button(text(t(app.lang, "cancel")).size(13)).style(subtle_btn).padding([8, 16]).on_press(Message::ImpCancel),
        ]
        .spacing(8),
    ]
    .spacing(14);
    if !app.msg.is_empty() {
        col = col.push(text(app.msg.clone()).size(12).color(muted_c()));
    }
    container(col).padding(28).into()
}

fn editor_view(app: &App) -> Element<'_, Message> {
    let title = format!("{} · {}", t(app.lang, "edit"), app.editing.clone().unwrap_or_default());
    container(
        column![
            text(title).size(18).font(bold()).color(text_c()),
            text_editor(&app.editor)
                .on_action(Message::EditorAction)
                .height(Length::Fill)
                .font(Font::MONOSPACE)
                .size(13)
                .padding(12)
                .style(editor_style),
            row![
                button(text(t(app.lang, "save")).size(13)).style(primary_btn).padding([8, 16]).on_press(Message::SaveConfig),
                button(text(t(app.lang, "cancel")).size(13)).style(subtle_btn).padding([8, 16]).on_press(Message::CancelEdit),
            ]
            .spacing(8),
        ]
        .spacing(14),
    )
    .padding(28)
    .into()
}

fn sidebar(app: &App) -> Element<'_, Message> {
    let mut head = row![];
    if let Some(logo) = &app.logo { head = head.push(image(logo.clone()).width(30).height(30)); }
    let head = head.push(text("Fluxpeer").size(17).font(bold()).color(text_c()))
        .push(Space::with_width(Length::Fill))
        .push(button(text(app.lang.label()).size(11)).style(ghost_btn).padding([4, 8]).on_press(Message::SetLang))
    .spacing(6)
    .align_y(iced::Alignment::Center);

    let mut list = column![].spacing(3);
    for n in &app.nets {
        let selected = Some(&n.iface) == app.selected.as_ref();
        let dot = text(if n.up { "●" } else { "○" }).size(10).color(if n.up { green_c() } else { muted_c() });
        let btn = button(row![dot, text(n.iface.clone()).size(14)].spacing(9).align_y(iced::Alignment::Center))
            .width(Length::Fill)
            .padding([8, 11])
            .on_press(Message::Select(n.iface.clone()))
            .style(if selected { row_selected } else { row_plain });
        list = list.push(btn);
    }
    if app.nets.is_empty() {
        list = list.push(text(t(app.lang, "no_networks")).size(12).color(muted_c()));
    }

    // One clear entry — the token field + import paths live in the add pane now.
    let add = button(text(format!("＋  {}", t(app.lang, "add_network"))).size(13))
        .width(Length::Fill)
        .padding(9)
        .style(primary_btn)
        .on_press(Message::OpenAdd);

    // Footer collapses to ONE row: a live daemon-status pill that doubles as the
    // entry to Diagnostics (so the bottom isn't a wall of buttons). Launch-at-login
    // moved into the Diagnostics pane. Selecting a network returns to the detail.
    let (ddot, dlabel, dcolor) = if app.offline {
        ("○", t(app.lang, "daemon_offline"), red_c())
    } else {
        ("●", t(app.lang, "daemon_running"), green_c())
    };
    let status = button(
        row![
            text(ddot).size(10).color(dcolor),
            text(dlabel).size(11).color(muted_c()),
            Space::with_width(Length::Fill),
            text(format!("{} ›", t(app.lang, "diagnostics"))).size(11).color(muted_c()),
        ]
        .spacing(7)
        .align_y(iced::Alignment::Center),
    )
    .width(Length::Fill)
    .padding([7, 9])
    .style(if app.route == Route::Diagnostics { row_selected } else { row_plain })
    .on_press(Message::GoTo(Route::Diagnostics));

    container(column![head, vscroll(list), add, status].spacing(16).padding(16))
        .width(Length::Fixed(248.0))
        .height(Length::Fill)
        .style(sidebar_style)
        .into()
}

/// A vertical scrollable whose scrollbar has a margin, so it doesn't butt up against
/// the content or the window edge (the default scrollbar is flush — looks cramped).
/// Used for every scroll area so they're consistent.
fn vscroll<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    scrollable(content)
        .direction(iced::widget::scrollable::Direction::Vertical(
            iced::widget::scrollable::Scrollbar::new().width(6).scroller_width(6).margin(6),
        ))
        .height(Length::Fill)
        .into()
}

fn check_style(_t: &Theme, _s: iced::widget::checkbox::Status) -> iced::widget::checkbox::Style {
    iced::widget::checkbox::Style {
        background: card_bg().into(),
        icon_color: accent_c(),
        border: iced::Border { color: border_c(), width: 1.0, radius: 5.0.into() },
        text_color: Some(text_c()),
    }
}

fn detail(app: &App) -> Element<'_, Message> {
    if app.offline {
        return offline_card(app);
    }
    let Some(iface) = app.selected.as_ref() else {
        let m = if app.msg.is_empty() { t(app.lang, "select_network").to_string() } else { app.msg.clone() };
        return container(text(m).color(muted_c())).padding(30).center_x(Length::Fill).into();
    };
    let net = app.nets.iter().find(|n| &n.iface == iface).cloned().unwrap_or_default();
    let st = app.status.get(iface);

    let toggle = if net.up {
        button(text(t(app.lang, "deactivate")).size(13)).style(subtle_btn).padding([8, 16]).on_press(Message::Toggle(net.iface.clone(), false))
    } else {
        button(text(t(app.lang, "activate")).size(13)).style(primary_btn).padding([8, 16]).on_press(Message::Toggle(net.iface.clone(), true))
    };
    let header = row![
        column![
            text(net.iface.clone()).size(23).font(bold()).color(text_c()),
            text(if net.up { t(app.lang, "active") } else { t(app.lang, "inactive") }).size(12).color(if net.up { green_c() } else { muted_c() }),
        ]
        .spacing(3),
        Space::with_width(Length::Fill),
        button(text(t(app.lang, "leave")).size(13)).style(danger_btn).padding([8, 12]).on_press(Message::Leave(net.iface.clone())),
        button(text(t(app.lang, "edit")).size(13)).style(ghost_btn).padding([8, 12]).on_press(Message::OpenEdit(net.iface.clone())),
        toggle,
    ]
    .spacing(10)
    .align_y(iced::Alignment::Center);

    let mut body = column![header].spacing(16);

    // #2 leave confirmation strip
    if app.confirm_leave.as_deref() == Some(net.iface.as_str()) {
        body = body.push(
            container(column![
                text(t(app.lang, "leave_q")).size(12).color(text_c()),
                row![
                    button(text(t(app.lang, "confirm_leave")).size(12)).style(danger_btn).padding([7, 14]).on_press(Message::LeaveConfirm(net.iface.clone())),
                    button(text(t(app.lang, "cancel")).size(12)).style(subtle_btn).padding([7, 14]).on_press(Message::LeaveCancel),
                ]
                .spacing(8),
            ].spacing(10))
            .padding(14)
            .width(Length::Fill)
            .style(warn_style),
        );
    }

    if net.up {
        let total_rx: u64 = st.map(|s| s.peers.iter().map(|p| p.rx).sum()).unwrap_or(0);
        let total_tx: u64 = st.map(|s| s.peers.iter().map(|p| p.tx).sum()).unwrap_or(0);
        let (rrx, rtx) = app.rate.get(iface).copied().unwrap_or((0.0, 0.0));
        body = body.push(section(t(app.lang, "traffic"), vec![
            (t(app.lang, "total"), format!("↓ {}   ↑ {}", human(total_rx), human(total_tx))),
            (t(app.lang, "now"), format!("↓ {}/s   ↑ {}/s", human(rrx as u64), human(rtx as u64))),
        ]));
    }

    // #3 tunnel mode: full-tunnel iff some peer routes 0.0.0.0/0 to us.
    let full_tunnel = st.map(|s| s.peers.iter().any(|p| p.allowed_ips.split(',').any(|c| c.trim() == "0.0.0.0/0" || c.trim() == "::/0"))).unwrap_or(false);

    let mut irows = vec![(t(app.lang, "control_server"), net.control.clone())];
    if let Some(s) = st {
        if !s.iface.public_key.is_empty() {
            irows.push((t(app.lang, "public_key"), s.iface.public_key.clone()));
        }
        if !s.iface.address.is_empty() {
            irows.push((t(app.lang, "addresses"), s.iface.address.clone()));
        }
        if s.iface.listen_port != 0 {
            irows.push((t(app.lang, "listen_port"), s.iface.listen_port.to_string()));
        }
    }
    body = body.push(section(t(app.lang, "interface"), irows));

    if let Some(error) = st.and_then(|s| s.iface.last_error.as_ref()).or(net.last_error.as_ref()) {
        body = body.push(text(error.clone()).size(12).color(red_c()));
        if error.contains("kill_switch_dns_blocked:") {
            body = body.push(text(t(app.lang, "kill_switch_dns_blocked_hint")).size(12).color(red_c()));
        }
    }

    // Exit / routing card — kept next to the interface info, above the peer list.
    body = body.push(exit_card(app, &net, full_tunnel));

    if let Some(s) = st {
        for p in &s.peers {
            body = body.push(peer_card(app, p));
        }
    } else if net.up {
        body = body.push(text(t(app.lang, "connecting")).size(12).color(muted_c()));
    }

    let srow = |label: String, ph: &'static str, val: &str, on: fn(String) -> Message| {
        row![
            text(label).size(12).color(muted_c()).width(Length::Fixed(132.0)),
            text_input(ph, val).on_input(on).size(13).padding(6).style(input_style),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center)
    };
    // Connection mode — a friendly selector over the transport-policy flags. Applies
    // on selection (restarts the network), like the exit toggle.
    let cur_mode = ConnMode::from_net(&net);
    let mode_items: Vec<ModeItem> = ConnMode::ALL.iter().map(|&m| ModeItem { mode: m, label: t(app.lang, m.key()) }).collect();
    let mode_row = row![
        text(t(app.lang, "conn_mode")).size(12).color(muted_c()).width(Length::Fixed(132.0)),
        pick_list(mode_items, Some(ModeItem { mode: cur_mode, label: t(app.lang, cur_mode.key()) }), |it| Message::SetConnMode(it.mode)).text_size(13).padding(6),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center);

    body = body.push(
        container(column![
            text(t(app.lang, "settings").to_uppercase()).size(11).color(muted_c()).font(bold()),
            mode_row,
            text(t(app.lang, "conn_mode_hint")).size(11).color(muted_c()),
            srow(t(app.lang, "mtu"), "e.g. 1380", &app.s_mtu, Message::SMtu),
            srow(t(app.lang, "dns"), "1.1.1.1, 8.8.8.8", &app.s_dns, Message::SDns),
            srow(t(app.lang, "endpoint"), "203.0.113.5:51820", &app.s_ep, Message::SEp),
            srow(t(app.lang, "exclude_routes"), "10.0.0.0/8, 192.168.0.0/16", &app.s_exclude, Message::SExclude),
            text(t(app.lang, "split_hint")).size(11).color(muted_c()),
            button(text(t(app.lang, "save")).size(13)).style(primary_btn).padding([8, 16]).on_press(Message::SaveSettings),
        ].spacing(11)).padding(16).width(Length::Fill).style(card_style),
    );

    container(vscroll(body)).padding(28).width(Length::Fill).into()
}

/// The "exit node" card: explains + controls the two distinct exit concepts so the
/// "act as exit node" toggle stops being cryptic. fluxpeer is split-tunnel by
/// default (you choose what is routed via AllowedIPs); it never routes 0.0.0.0/0
/// by default and is not a public-exit VPN.
fn exit_card<'a>(app: &App, net: &Net, full_tunnel: bool) -> Element<'a, Message> {
    let mode = t(app.lang, if full_tunnel { "full_tunnel" } else { "split_tunnel" });
    let mut col = column![
        text(t(app.lang, "exit_card_title").to_uppercase()).size(11).color(muted_c()).font(bold()),
        // The live mode is separate from the explicit consumer preference.
        row![
            text(t(app.lang, "outbound_mode")).size(12).color(muted_c()).width(Length::Fixed(132.0)),
            text(mode).size(13).color(text_c()),
        ]
        .spacing(12),
    ]
    .spacing(11);

    col = col.push(checkbox(t(app.lang, "use_exit"), !matches!(net.use_exit.as_str(), "" | "off")).on_toggle(Message::ToggleUseExit).size(16).text_size(13).style(check_style));
    col = col.push(text(t(app.lang, "use_exit_desc")).size(11).color(muted_c()));
    col = col.push(checkbox(t(app.lang, "kill_switch"), net.kill_switch).on_toggle(Message::ToggleKillSwitch).size(16).text_size(13).style(check_style));
    col = col.push(text(t(app.lang, "kill_switch_desc")).size(11).color(muted_c()));

    // (2) the opt-in "be an exit for others" role — clearly scoped + explained.
    // Supported on Linux (iptables), Windows (WinNAT) and macOS (pf).
    col = col.push(checkbox(t(app.lang, "be_exit"), net.exit_node).on_toggle(Message::ToggleExit).size(16).text_size(13).style(check_style));
    col = col.push(text(t(app.lang, "be_exit_desc")).size(11).color(muted_c()));
    let _ = app;
    container(col).padding(16).width(Length::Fill).style(card_style).into()
}

/// One peer's card with a live online/offline status dot (#4): a peer is "online"
/// when its last handshake was within 180s (WireGuard's keepalive window).
fn peer_card<'a>(app: &App, p: &Peer) -> Element<'a, Message> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let online = p.last_handshake != 0 && now.saturating_sub(p.last_handshake) < 180;
    let (dot, label, color) = if online {
        ("●", t(app.lang, "online"), green_c())
    } else {
        ("○", t(app.lang, "offline"), muted_c())
    };
    let head = row![
        text(t(app.lang, "peer").to_uppercase()).size(11).color(muted_c()).font(bold()),
        Space::with_width(Length::Fill),
        text(dot).size(11).color(color),
        text(label).size(11).color(color),
    ]
    .spacing(7)
    .align_y(iced::Alignment::Center);
    let rows = vec![
        (t(app.lang, "public_key"), short(&p.public_key)),
        (t(app.lang, "endpoint"), if p.endpoint.is_empty() { "—".into() } else { format!("{}  ({})", p.endpoint, p.transport) }),
        (t(app.lang, "allowed_ips"), if p.allowed_ips.is_empty() { "—".into() } else { p.allowed_ips.clone() }),
        (t(app.lang, "latest_handshake"), ago(p.last_handshake, app.lang)),
        (t(app.lang, "transfer"), format!("↓ {}  ↑ {}", human(p.rx), human(p.tx))),
    ];
    let mut col = column![head].spacing(11);
    for (k, v) in rows {
        let f = vfont(&v);
        col = col.push(
            row![
                text(k).size(12).color(muted_c()).width(Length::Fixed(132.0)),
                text(v).size(13).font(f).color(text_c()),
            ]
            .spacing(12),
        );
    }
    container(col).padding(16).width(Length::Fill).style(card_style).into()
}

/// #1 — the daemon isn't reachable: a centered card explaining why + a Start button
/// (admin auth) so a fresh install can connect without dropping to a terminal.
fn offline_card(app: &App) -> Element<'_, Message> {
    let mut btn = button(text(if app.starting { t(app.lang, "starting") } else { t(app.lang, "start_daemon") }).size(13))
        .style(primary_btn)
        .padding([9, 18]);
    if !app.starting {
        btn = btn.on_press(Message::StartDaemon);
    }
    let mut col = column![
        text(t(app.lang, "daemon_offline_title")).size(20).font(bold()).color(text_c()),
        text(t(app.lang, "daemon_offline_body")).size(13).color(muted_c()),
        btn,
        text(format!("{}: sudo fluxpeer up", t(app.lang, "daemon"))).size(11).font(ui_font()).color(muted_c()),
    ]
    .spacing(16)
    .max_width(440);
    if !app.msg.is_empty() && app.msg != "cancelled" {
        col = col.push(text(app.msg.clone()).size(12).color(red_c()));
    }
    container(col).padding(40).center_x(Length::Fill).center_y(Length::Fill).into()
}

/// #6 — diagnostics pane: daemon reachability + the selected network's node log tail.
fn diagnostics_view(app: &App) -> Element<'_, Message> {
    let (ddot, dlabel, dcolor) = if app.offline {
        ("○", t(app.lang, "unreachable"), red_c())
    } else {
        ("●", t(app.lang, "reachable"), green_c())
    };
    let head = row![
        button(text(format!("‹ {}", t(app.lang, "networks_nav"))).size(12)).style(ghost_btn).padding([6, 10]).on_press(Message::GoTo(Route::Networks)),
        text(t(app.lang, "diagnostics")).size(22).font(bold()).color(text_c()),
        Space::with_width(Length::Fill),
        button(text(t(app.lang, "refresh_logs")).size(12)).style(subtle_btn).padding([7, 14]).on_press(Message::LoadLogs),
    ]
    .spacing(10)
    .align_y(iced::Alignment::Center);

    let info = section(t(app.lang, "daemon"), vec![
        ("Address".into(), app.addr.clone()),
        (t(app.lang, "status"), format!("{ddot} {dlabel}")),
        ("Token".into(), if app.token.is_empty() { "—".into() } else { format!("{}…", &app.token[..app.token.len().min(8)]) }),
        (t(app.lang, "networks_nav"), format!("{} ({} up)", app.nets.len(), app.nets.iter().filter(|n| n.up).count())),
    ]);
    // recolor the status line is overkill; the dot char conveys it.
    let _ = dcolor;

    // Preferences (moved out of the sidebar to declutter it).
    let prefs = container(column![
        text(t(app.lang, "preferences").to_uppercase()).size(11).color(muted_c()).font(bold()),
        checkbox(t(app.lang, "dark_theme"), app.dark).on_toggle(|_| Message::ToggleTheme).size(16).text_size(13).style(check_style),
        checkbox(t(app.lang, "launch_at_login"), app.launch_login).on_toggle(Message::ToggleLaunchLogin).size(16).text_size(13).style(check_style),
    ].spacing(12))
    .padding(16)
    .width(Length::Fill)
    .style(card_style);

    let log_title = match &app.logs_iface {
        Some(i) => format!("{} · {}", t(app.lang, "diagnostics"), i),
        None => t(app.lang, "select_for_logs"),
    };
    let log_body: Element<'_, Message> = if app.selected.is_none() {
        text(t(app.lang, "select_for_logs")).size(12).color(muted_c()).into()
    } else if app.logs.trim().is_empty() {
        text(t(app.lang, "no_logs")).size(12).color(muted_c()).into()
    } else {
        vscroll(text(app.logs.clone()).size(11).font(Font::MONOSPACE).color(text_c()))
    };
    let log_card = container(column![text(log_title).size(12).color(muted_c()).font(bold()), log_body].spacing(10))
        .padding(16)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(card_style);

    container(column![head, info, prefs, log_card].spacing(16))
        .padding(28)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn section(title: String, rows: Vec<(String, String)>) -> Element<'static, Message> {
    let mut col = column![text(title.to_uppercase()).size(11).color(muted_c()).font(bold())].spacing(11);
    for (k, v) in rows {
        let f = vfont(&v);
        col = col.push(
            row![
                text(k).size(12).color(muted_c()).width(Length::Fixed(132.0)),
                text(v).size(13).font(f).color(text_c()),
            ]
            .spacing(12),
        );
    }
    container(col).padding(16).width(Length::Fill).style(card_style).into()
}

fn short(key: &str) -> String {
    if key.len() > 20 {
        format!("{}…{}", &key[..10], &key[key.len() - 6..])
    } else {
        key.to_string()
    }
}

fn human(n: u64) -> String {
    let u = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut f = n as f64;
    let mut i = 0;
    while f >= 1024.0 && i < u.len() - 1 {
        f /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{f:.2} {}", u[i])
    }
}

fn ago(unix: u64, lang: Lang) -> String {
    if unix == 0 {
        return t(lang, "never").to_string();
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let d = now.saturating_sub(unix);
    let (en, zh) = if d < 60 {
        (format!("{d} seconds ago"), format!("{d} 秒前"))
    } else if d < 3600 {
        (format!("{} minutes ago", d / 60), format!("{} 分钟前", d / 60))
    } else {
        (format!("{} hours ago", d / 3600), format!("{} 小时前", d / 3600))
    };
    match lang {
        Lang::En => en,
        Lang::Zh => zh,
    }
}

// ---- daemon client + file import ----

async fn pick_file() -> Option<String> {
    let fh = rfd::AsyncFileDialog::new()
        .add_filter("fluxpeer config / token", &["json", "conf", "txt"])
        .pick_file()
        .await?;
    Some(String::from_utf8_lossy(&fh.read().await).to_string())
}

/// Batch-register a parsed wg.conf's devices at the control-server (admin import).
/// A blank `network` creates a new one named after the `[Interface]`.
async fn ctl_import(server: String, password: String, network: String, net_name: String, devices: Vec<Value>) -> Result<String, String> {
    let client = fluxpeer_sdk::Client::with_password(server.trim(), password.trim());
    let net_id = if network.trim().is_empty() {
        let net = client.create_network(net_name.trim()).await.map_err(|e| format!("create network: {e}"))?;
        net["id"].as_str().ok_or_else(|| "create network: missing id".to_string())?.to_string()
    } else {
        network.trim().to_string()
    };
    let res = client
        .import_devices(&net_id, &Value::Array(devices))
        .await
        .map_err(|e| format!("import: {e}"))?;
    let created = res["created"].as_array().map(|a| a.len()).unwrap_or(0);
    let skipped = res["skipped"].as_array().map(|a| a.len()).unwrap_or(0);
    Ok(format!("✓ {created} devices → {net_id}{}", if skipped > 0 { format!(", {skipped} skipped") } else { String::new() }))
}

async fn daemon_call(addr: String, token: String, mut body: Value) -> Result<Value, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    body["auth"] = json!(token);
    let mut stream = tokio::net::TcpStream::connect(&addr).await.map_err(|e| format!("connect {addr}: {e}"))?;
    stream.write_all((body.to_string() + "\n").as_bytes()).await.map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.map_err(|e| e.to_string())?;
    serde_json::from_slice(&buf).map_err(|e| format!("decode: {e}"))
}

async fn disconnect_for_quit(addr: String, token: String) -> Result<(), String> {
    let value = tokio::time::timeout(std::time::Duration::from_secs(30),
        daemon_call(addr, token, json!({"cmd":"shutdown"}))).await
        .map_err(|_| "daemon disconnection timed out".to_string())??;
    quit::check_ack(value)
}

async fn daemon_action(addr: String, token: String, body: Value) -> Result<String, String> {
    let v = daemon_call(addr, token, body).await?;
    match v.get("error").and_then(|x| x.as_str()) {
        Some(e) => Err(e.to_string()),
        None => Ok(v.to_string()),
    }
}

async fn load(addr: String, token: String) -> Result<Snapshot, String> {
    if token.is_empty() {
        return Err("no daemon token (set FLUXPEER_DAEMON_TOKEN)".into());
    }
    let nets_v = daemon_call(addr.clone(), token.clone(), json!({"cmd": "networks"})).await?;
    if let Some(e) = nets_v.get("error").and_then(|x| x.as_str()) {
        return Err(e.to_string());
    }
    let status_v = daemon_call(addr, token, json!({"cmd": "status"})).await.unwrap_or_else(|_| json!([]));
    let nets = nets_v
        .as_array()
        .map(|a| {
            a.iter()
                .map(|n| Net {
                    iface: n["iface"].as_str().unwrap_or("").to_string(),
                    control: n["control_server"].as_str().unwrap_or("").to_string(),
                    up: n["up"].as_bool().unwrap_or(false),
                    exit_node: n["exit_node"].as_bool().unwrap_or(false),
                    use_exit: n["use_exit"].as_str().unwrap_or("off").to_string(),
                    kill_switch: n["kill_switch"].as_bool().unwrap_or(false),
                    last_error: status_error(&n["last_error"]),
                    force_relay: n["force_relay"].as_bool().unwrap_or(false),
                    relay_anytls: n["relay_anytls"].as_bool().unwrap_or(false),
                    relay_bond: n["relay_bond"].as_bool().unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default();
    let mut status = Vec::new();
    if let Some(arr) = status_v.as_array() {
        for s in arr {
            let name = s["interface"]["name"].as_str().unwrap_or("").to_string();
            let iface = Iface {
                last_error: status_error(&s["interface"]["last_error"]),
                address: s["interface"]["address"].as_str().unwrap_or("").to_string(),
                public_key: s["interface"]["public_key"].as_str().unwrap_or("").to_string(),
                listen_port: s["interface"]["listen_port"].as_u64().unwrap_or(0),
            };
            let peers = s["peers"]
                .as_array()
                .map(|ps| {
                    ps.iter()
                        .map(|p| Peer {
                            public_key: p["public_key"].as_str().unwrap_or("").to_string(),
                            endpoint: p["endpoint"].as_str().unwrap_or("").to_string(),
                            allowed_ips: p["allowed_ips"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default(),
                            transport: p["transport"].as_str().unwrap_or("").to_string(),
                            last_handshake: p["last_handshake_unix"].as_u64().unwrap_or(0),
                            rx: p["rx_bytes"].as_u64().unwrap_or(0),
                            tx: p["tx_bytes"].as_u64().unwrap_or(0),
                        })
                        .collect()
                })
                .unwrap_or_default();
            status.push((name, Status { iface, peers }));
        }
    }
    Ok(Snapshot { nets, status })
}

// Startup failures are strings; a running engine reports structured ApplyError.
fn status_error(value: &Value) -> Option<String> {
    value.as_str().or_else(|| value.get("stderr").and_then(Value::as_str)).filter(|s| !s.is_empty()).map(String::from)
}

fn kill_switch_settings(iface: Option<&str>, enabled: bool) -> Option<Value> {
    Some(json!({"cmd":"settings", "iface":iface?, "kill_switch":enabled}))
}

/// Opting in selects an approved exit automatically; opting out restores split.
fn use_exit_settings(iface: Option<&str>, enabled: bool) -> Option<Value> {
    let iface = iface?;
    Some(json!({"cmd": "settings", "iface": iface, "use_exit": if enabled { "auto" } else { "off" }}))
}

#[cfg(test)]
mod exit_tests {
    use super::*;

    fn fixture_app() -> App {
        App { addr: String::new(), token: String::new(), lang: Lang::En, nets: vec![], status: Default::default(), selected: None,
            join_input: String::new(), msg: String::new(), action_error: None, prev: Default::default(), rate: Default::default(),
            s_mtu: String::new(), s_dns: String::new(), s_ep: String::new(), s_exclude: String::new(), editing: None,
            editor: text_editor::Content::new(), importing: None, tray_sig: String::new(), logo: None, route: Route::default(),
            offline: false, starting: false, confirm_leave: None, logs: String::new(), logs_iface: None, launch_login: false,
            dark: false, quit: quit::QuitState::default(), adding: false }
    }

    #[test]
    fn action_conflict_survives_status_refresh_until_dismissed_or_successful_retry() {
        let mut app = fixture_app();
        let message = "kill-switch active on fp0; disable it first";
        let _ = update(&mut app, Message::Acted(Err(message.into())));
        let _ = update(&mut app, Message::Loaded(Ok(Snapshot::default())));
        assert_eq!(app.action_error.as_deref(), Some(message));
        let _ = view(&app);
        let _ = update(&mut app, Message::DismissActionError);
        assert!(app.action_error.is_none());
        let _ = update(&mut app, Message::Acted(Err(message.into())));
        let _ = update(&mut app, Message::Acted(Ok("{}".into())));
        assert!(app.action_error.is_none());
    }

    #[test]
    fn kill_switch_conflict_reaches_desktop_action_error_unchanged() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
            let message = "kill-switch active on fp0; disable it first";
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap().to_string();
            let server = tokio::spawn(async move {
                let mut reader = BufReader::new(listener.accept().await.unwrap().0);
                let mut request = String::new(); reader.read_line(&mut request).await.unwrap();
                assert_eq!(serde_json::from_str::<Value>(&request).unwrap(), json!({"cmd":"up","iface":"fp1","auth":"test-token"}));
                let reply = json!({"status":409,"error":message});
                reader.into_inner().write_all(format!("{reply}\n").as_bytes()).await.unwrap();
            });
            assert_eq!(daemon_action(addr, "test-token".into(), json!({"cmd":"up","iface":"fp1"})).await.unwrap_err(), message);
            server.await.unwrap();
        });
    }

    #[test]
    fn blocked_carrier_dns_is_visible_for_startup_and_running_status() {
        let error = "kill_switch_dns_blocked: relay.test; connect an exit or disable kill-switch";
        assert_eq!(status_error(&json!(error)).as_deref(), Some(error));
        assert_eq!(status_error(&json!({"command":["kill-switch", "DNS"], "stderr":error})).as_deref(), Some(error));
        assert_eq!(status_error(&Value::Null), None);
        assert_eq!(status_error(&json!({})), None);
    }

    #[test]
    fn kill_switch_only_changes_the_selected_network_local_policy() {
        assert_eq!(kill_switch_settings(None, true), None);
        assert_eq!(kill_switch_settings(Some("fp0"), true), Some(json!({"cmd":"settings", "iface":"fp0", "kill_switch":true})));
        assert_eq!(kill_switch_settings(Some("fp1"), false), Some(json!({"cmd":"settings", "iface":"fp1", "kill_switch":false})));
    }

    #[test]
    fn consumer_toggle_without_selection_never_builds_a_request() {
        assert_eq!(use_exit_settings(None, true), None);
        assert_eq!(use_exit_settings(None, false), None);
    }

    #[test]
    fn consumer_toggle_only_sets_use_exit_for_selected_network() {
        assert_eq!(use_exit_settings(Some("fp0"), true), Some(json!({"cmd": "settings", "iface": "fp0", "use_exit": "auto"})));
        assert_eq!(use_exit_settings(Some("fp1"), false), Some(json!({"cmd": "settings", "iface": "fp1", "use_exit": "off"})));
    }
}

#[cfg(test)]
mod quit_rpc_tests {
    use super::*;
    #[test]
    fn quit_rpc_checks_token_and_does_not_accept_errors_or_missing_ack() {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
            for (reply, succeeds) in [(json!({"ok":true,"shutdown":true}), true),
                (json!({"error":"cleanup failed"}), false), (json!({"ok":true}), false)] {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap().to_string();
                let server = tokio::spawn(async move {
                    let mut reader = BufReader::new(listener.accept().await.unwrap().0);
                    let mut request = String::new(); reader.read_line(&mut request).await.unwrap();
                    assert_eq!(serde_json::from_str::<Value>(&request).unwrap(), json!({"cmd":"shutdown","auth":"test-token"}));
                    reader.into_inner().write_all(format!("{reply}\n").as_bytes()).await.unwrap();
                });
                assert_eq!(disconnect_for_quit(addr, "test-token".into()).await.is_ok(), succeeds);
                server.await.unwrap();
            }
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap().to_string(); drop(listener);
            assert!(disconnect_for_quit(addr, "test-token".into()).await.is_err());
        });
    }
}
