//! System tray icon: minimize-to-tray, quick menu, state-colored icon.

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

const ICON_BYTES: &[u8] = include_bytes!("../../../assets/icon.png");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    Normal,
    Warning,
    Error,
}

pub enum TrayCommand {
    Open,
    ApplyDefaultProfile,
    Exit,
}

pub struct AppTray {
    tray: TrayIcon,
    open_item: MenuItem,
    apply_item: MenuItem,
    exit_item: MenuItem,
    open_id: String,
    apply_id: String,
    exit_id: String,
    icons: [Icon; 3],
    state: TrayState,
}

impl AppTray {
    /// Build the tray icon. Must run on the same thread as the (already-started)
    /// event loop - call this from inside the eframe app-creation closure.
    pub fn new() -> Result<Self, String> {
        let icons = build_state_icons()?;

        let open = MenuItem::with_id("open", t!("tray.open"), true, None);
        let apply = MenuItem::with_id("apply-default", t!("tray.apply_default"), true, None);
        let exit = MenuItem::with_id("exit", t!("tray.exit"), true, None);
        let menu = Menu::new();
        menu.append_items(&[&open, &apply, &exit])
            .map_err(|e| format!("tray menu: {e}"))?;

        let tray = TrayIconBuilder::new()
            .with_tooltip("fancontrol-rs")
            .with_icon(icons[0].clone())
            .with_menu(Box::new(menu))
            .build()
            .map_err(|e| format!("tray icon: {e}"))?;

        Ok(Self {
            tray,
            open_id: open.id().0.clone(),
            apply_id: apply.id().0.clone(),
            exit_id: exit.id().0.clone(),
            open_item: open,
            apply_item: apply,
            exit_item: exit,
            icons,
            state: TrayState::Normal,
        })
    }

    /// Re-apply translated text to the tray menu/tooltip after a language change.
    pub fn retranslate(&self) {
        self.open_item.set_text(t!("tray.open"));
        self.apply_item.set_text(t!("tray.apply_default"));
        self.exit_item.set_text(t!("tray.exit"));
    }

    /// Swap the tray icon image when the state actually changes (avoid redundant OS calls).
    pub fn set_state(&mut self, state: TrayState) {
        if state == self.state {
            return;
        }
        let icon = self.icons[state as usize].clone();
        if self.tray.set_icon(Some(icon)).is_ok() {
            self.state = state;
        }
    }

    /// Drain pending tray/menu events. Call once per UI frame.
    pub fn poll_commands(&self) -> Vec<TrayCommand> {
        let mut out = Vec::new();
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id.0 == self.open_id {
                out.push(TrayCommand::Open);
            } else if event.id.0 == self.apply_id {
                out.push(TrayCommand::ApplyDefaultProfile);
            } else if event.id.0 == self.exit_id {
                out.push(TrayCommand::Exit);
            }
        }
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if matches!(event, TrayIconEvent::DoubleClick { .. }) {
                out.push(TrayCommand::Open);
            }
        }
        out
    }
}

/// Build normal/warning/error icon variants by tinting the base app icon.
fn build_state_icons() -> Result<[Icon; 3], String> {
    let base = eframe::icon_data::from_png_bytes(ICON_BYTES).map_err(|e| e.to_string())?;
    let normal = Icon::from_rgba(base.rgba.clone(), base.width, base.height)
        .map_err(|e| format!("normal icon: {e}"))?;
    let warning = Icon::from_rgba(tint(&base.rgba, 255, 200, 0), base.width, base.height)
        .map_err(|e| format!("warning icon: {e}"))?;
    let error = Icon::from_rgba(tint(&base.rgba, 220, 40, 40), base.width, base.height)
        .map_err(|e| format!("error icon: {e}"))?;
    Ok([normal, warning, error])
}

/// Blend every opaque-ish pixel toward `(r, g, b)`, keeping alpha untouched.
fn tint(rgba: &[u8], r: u8, g: u8, b: u8) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for px in out.chunks_exact_mut(4) {
        if px[3] == 0 {
            continue;
        }
        px[0] = ((u16::from(px[0]) + u16::from(r) * 2) / 3) as u8;
        px[1] = ((u16::from(px[1]) + u16::from(g) * 2) / 3) as u8;
        px[2] = ((u16::from(px[2]) + u16::from(b) * 2) / 3) as u8;
    }
    out
}
