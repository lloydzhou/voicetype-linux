//! Tray icon (tray-icon + libappindicator/gtk3) with language selection,
//! LLM-refinement submenu, settings entry and Quit.
//! The icon is generated programmatically into an RGBA buffer — no assets.

use crate::config::{language_index, SharedConfig, LANGUAGES};
use gtk::glib;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

/// Actions forwarded from the tray thread to the UI.
#[derive(Debug, Clone)]
pub enum TrayAction {
    Language(usize),
    LlmToggled(bool),
    OpenSettings,
    Quit,
}

pub struct Tray {
    _menu: Menu,
    _tray: TrayIcon,
}

/// 32x32 RGBA icon: dark rounded disc + five white waveform bars.
fn make_icon() -> Icon {
    const S: usize = 32;
    let mut rgba = vec![0u8; S * S * 4];

    let cx = (S as f32 - 1.0) / 2.0;
    let cy = cx;
    let radius = 14.5;

    let bars: [(f32, f32); 5] = [ // (center_x, height)
        (9.0, 8.0),
        (12.5, 14.0),
        (16.0, 18.0),
        (19.5, 13.0),
        (23.0, 9.0),
    ];

    for y in 0..S {
        for x in 0..S {
            let (fx, fy) = (x as f32, y as f32);
            let i = (y * S + x) * 4;
            let dist = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();

            let (mut r, mut g, mut b, mut a) = (0u8, 0u8, 0u8, 0u8);
            if dist <= radius {
                // dark disc with soft 1px edge
                let edge = ((radius - dist).min(1.0)).max(0.0);
                r = 31;
                g = 31;
                b = 36;
                a = (edge * 235.0) as u8;

                for (bx, bh) in bars {
                    let half_h = bh / 2.0;
                    if (fx - bx).abs() <= 1.5 && (fy - cy).abs() <= half_h {
                        r = 242;
                        g = 247;
                        b = 255;
                        a = 255;
                    }
                }
            }
            rgba[i] = r;
            rgba[i + 1] = g;
            rgba[i + 2] = b;
            rgba[i + 3] = a;
        }
    }

    Icon::from_rgba(rgba, S as u32, S as u32).expect("valid icon rgba")
}

/// Build the tray. Must be called from the GTK main thread after gtk::init.
/// Menu events are forwarded to `tx` from a small forwarding thread.
pub fn setup(config: SharedConfig, tx: glib::Sender<TrayAction>) -> Result<Tray, String> {
    let (lang_idx, llm_enabled) = {
        let c = config.lock().map_err(|_| "config mutex poisoned")?;
        (language_index(&c.language), c.llm.enabled)
    };

    let menu = Menu::new();

    // ---- language submenu (radio-like CheckMenuItems) ----
    let lang_menu = Submenu::new("语言 / Language", true);
    let mut lang_items: Vec<CheckMenuItem> = Vec::new();
    for (i, (_, name, _code)) in LANGUAGES.iter().enumerate() {
        let item = CheckMenuItem::new(*name, true, i == lang_idx, None);
        lang_menu.append(&item);
        lang_items.push(item);
    }
    menu.append(&lang_menu);

    // ---- LLM refinement submenu ----
    let llm_menu = Submenu::new("LLM 润色", true);
    let llm_toggle = CheckMenuItem::new("启用", true, llm_enabled, None);
    let settings_item = MenuItem::new("Settings…", true, None);
    llm_menu.append(&llm_toggle);
    llm_menu.append(&PredefinedMenuItem::separator());
    llm_menu.append(&settings_item);
    menu.append(&llm_menu);

    menu.append(&PredefinedMenuItem::separator());
    let quit_item = MenuItem::new("退出 Quit", true, None);
    menu.append(&quit_item);

    // muda menus on the GTK backend need a gtk accelerator group context;
    // tray-icon handles this internally once gtk is initialized.

    let icon = make_icon();
    let tray = TrayIconBuilder::new()
        .with_id("voicetype")
        .with_menu(Box::new(menu.clone()))
        .with_tooltip(format!(
            "voicetype — 按住 {} 说话",
            config.lock().map(|c| crate::hotkey::display_combos(&c.hotkeys))
                .unwrap_or_else(|_| "右 Ctrl+右 Shift".to_string())
        ))
        .with_icon(icon)
        .build()
        .map_err(|e| format!("创建托盘失败: {e}"))?;

    // Build id -> action map after all items exist.
    let mut id_actions: std::collections::HashMap<String, TrayAction> =
        std::collections::HashMap::new();
    for (i, item) in lang_items.iter().enumerate() {
        id_actions.insert(item.id().0.clone(), TrayAction::Language(i));
    }
    id_actions.insert(llm_toggle.id().0.clone(), TrayAction::LlmToggled(true)); // resolved below
    id_actions.insert(settings_item.id().0.clone(), TrayAction::OpenSettings);
    id_actions.insert(quit_item.id().0.clone(), TrayAction::Quit);

    let llm_toggle_id = llm_toggle.id().0.clone();
    let lang_items_for_events: Vec<(String, usize, CheckMenuItem)> = lang_items
        .iter()
        .enumerate()
        .map(|(i, it)| (it.id().0.clone(), i, it.clone()))
        .collect();

    // Forwarding thread: muda's global event channel only forwards the raw
    // menu-item id (menu items are !Send, so no item state is touched here).
    // All lookups, checkbox updates and config writes happen on the main
    // thread in the attach closure below.
    let (ev_tx, ev_rx) = glib::MainContext::channel::<String>(glib::Priority::DEFAULT);
    {
        let ev_tx = ev_tx;
        std::thread::Builder::new()
            .name("tray-events".into())
            .spawn(move || {
                let recv = MenuEvent::receiver();
                loop {
                    let ev = match recv.recv() {
                        Ok(e) => e,
                        Err(_) => return,
                    };
                    if ev_tx.send(ev.id.0.clone()).is_err() {
                        return; // UI gone
                    }
                }
            })
            .map_err(|e| format!("spawn tray thread: {e}"))?;
    }

    {
        let config = config;
        ev_rx.attach(None, move |id| {
            if id == llm_toggle_id {
                let checked = llm_toggle.is_checked();
                if let Ok(mut c) = config.lock() {
                    c.llm.enabled = checked;
                    if let Err(e) = c.save() {
                        log::error!("save config: {e}");
                    }
                }
                let _ = tx.send(TrayAction::LlmToggled(checked));
                return glib::ControlFlow::Continue;
            }
            if let Some((_, idx, self_item)) =
                lang_items_for_events.iter().find(|(iid, _, _)| *iid == id)
            {
                let idx = *idx;
                // radio behaviour: check selected, uncheck others
                for (_, j, it) in &lang_items_for_events {
                    it.set_checked(*j == idx);
                }
                if let Ok(mut c) = config.lock() {
                    c.language = LANGUAGES[idx].0.to_string();
                    if let Err(e) = c.save() {
                        log::error!("save config: {e}");
                    }
                }
                let _ = self_item.is_checked();
                let _ = tx.send(TrayAction::Language(idx));
                return glib::ControlFlow::Continue;
            }
            if let Some(action) = id_actions.get(&id) {
                let _ = tx.send(action.clone());
            }
            glib::ControlFlow::Continue
        });
    }

    Ok(Tray {
        _menu: menu,
        _tray: tray,
    })
}
