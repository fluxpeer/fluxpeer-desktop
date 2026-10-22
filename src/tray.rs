// SPDX-License-Identifier: AGPL-3.0-or-later
//! System tray / menu-bar item (macOS menu bar · Windows tray · Linux status icon)
//! via `tray-icon`. The menu is rebuilt on demand (`update_menu`) to reflect the
//! current networks + language: one checkable row per network (id `net:<iface>`,
//! checked = connected) then Connect all / Disconnect all / Quit. Menu clicks are
//! drained from the global receiver in an iced subscription (`Message::TrayPoll`).
//!
//! NOTE: macOS needs the NSStatusItem created on the main thread; we build it in
//! `main()` before iced's event loop and keep the handle alive for the program.

use tray_icon::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

use crate::i18n::{t, Lang};

/// Keep this alive for the whole program (dropping it removes the tray icon).
pub struct Tray {
    tray: TrayIcon,
    _keep: Vec<Box<dyn IsMenuItem>>, // menu items must outlive the menu
}

pub fn build() -> Option<Tray> {
    #[allow(unused_mut)]
    let mut builder = TrayIconBuilder::new().with_tooltip("Fluxpeer");
    if let Some(icon) = icon(false) { builder = builder.with_icon(icon); }
    // macOS menu-bar icons should be TEMPLATE images: the OS draws them crisp in the
    // bar's own colour (black on light, white on dark) from the alpha channel. The
    // thin colour mark was near-invisible; a template reads cleanly at 22pt.
    #[cfg(target_os = "macos")]
    {
        builder = builder.with_icon_as_template(true);
    }
    let tray = builder.build().ok()?;
    #[cfg(target_os = "macos")]
    tray.set_icon_as_template(true);
    let mut t = Tray { tray, _keep: Vec::new() };
    t.update_menu(&[], Lang::default());
    Some(t)
}

impl Tray {
    /// Rebuild the menu for the current networks + language.
    pub fn update_menu(&mut self, nets: &[(String, bool)], lang: Lang) {
        let menu = Menu::new();
        let mut keep: Vec<Box<dyn IsMenuItem>> = Vec::new();

        let show = MenuItem::with_id("show", t(lang, "show"), true, None);
        let _ = menu.append(&show);
        let _ = menu.append(&PredefinedMenuItem::separator());
        keep.push(Box::new(show));

        for (iface, up) in nets {
            let item = CheckMenuItem::with_id(format!("net:{iface}"), iface, true, *up, None);
            let _ = menu.append(&item);
            keep.push(Box::new(item));
        }
        if !nets.is_empty() {
            let _ = menu.append(&PredefinedMenuItem::separator());
        }
        let ca = MenuItem::with_id("connect_all", t(lang, "connect_all"), true, None);
        let da = MenuItem::with_id("disconnect_all", t(lang, "disconnect_all"), true, None);
        let quit = MenuItem::with_id("quit", t(lang, "quit"), true, None);
        let _ = menu.append(&ca);
        let _ = menu.append(&da);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&quit);
        keep.push(Box::new(ca));
        keep.push(Box::new(da));
        keep.push(Box::new(quit));

        self.tray.set_menu(Some(Box::new(menu)));
        self._keep = keep;

        // Icon reflects connection state: solid when any network is up, lighter otherwise.
        let any_up = nets.iter().any(|(_, up)| *up);
        let _ = self.tray.set_icon(icon(any_up));
        #[cfg(target_os = "macos")]
        self.tray.set_icon_as_template(true); // set_icon can clear the template flag
    }
}

/// A BOLD, full-opacity variant of the brand mark just for the menu bar. The nav
/// `brand/mark.svg` fades its strokes (opacity 0.55/0.4) and uses thin lines — fine
/// at 96px, but as a 22pt template glyph it reads as faint scattered dots. This one
/// is solid black (template → the OS recolours via alpha), thicker, with a bigger
/// hub + peer nodes so the octagon mesh stays legible at menu-bar size.
const TRAY_MARK: &[u8] = b"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 64 64'>\
<path d='M 32 9 L 48.3 15.7 L 55 32 L 48.3 48.3 L 32 55 L 15.7 48.3 L 9 32 L 15.7 15.7 Z' fill='none' stroke='#000' stroke-width='4' stroke-linejoin='round'/>\
<path d='M 32 9 L 32 55 M 48.3 15.7 L 15.7 48.3 M 55 32 L 9 32 M 48.3 48.3 L 15.7 15.7' stroke='#000' stroke-width='3' stroke-linecap='round'/>\
<circle cx='32' cy='9' r='4.5'/><circle cx='48.3' cy='15.7' r='4.5'/><circle cx='55' cy='32' r='4.5'/><circle cx='48.3' cy='48.3' r='4.5'/><circle cx='32' cy='55' r='4.5'/><circle cx='15.7' cy='48.3' r='4.5'/><circle cx='9' cy='32' r='4.5'/><circle cx='15.7' cy='15.7' r='4.5'/>\
<circle cx='32' cy='32' r='7'/></svg>";

/// Rasterize the bold mesh mark into an RGBA menu-bar icon (used as a macOS template
/// image). When `active` is false the alpha is reduced so the bar shows a lighter
/// "nothing connected" glyph.
fn icon(active: bool) -> Option<tray_icon::Icon> {
    let size = 44u32; // ~22pt menu bar @2x
    let mut rgba = crate::raster::render(TRAY_MARK, size)?;
    if !active {
        for px in rgba.as_chunks_mut::<4>().0 {
            px[3] = (px[3] as u16 * 110 / 255) as u8;
        }
    }
    tray_icon::Icon::from_rgba(rgba, size, size).ok()
}
