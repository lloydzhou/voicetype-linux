//! GTK3 UI: the recording capsule HUD and the LLM settings window.

use crate::config::{LlmConfig, SharedConfig};
use crate::llm;
use gtk::prelude::*;
use gtk::{cairo, glib, pango};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

// ---- capsule geometry / style constants ----
const CAPSULE_HEIGHT: f64 = 56.0;
const CORNER_RADIUS: f64 = 28.0;
const BARS_AREA_W: f64 = 44.0;
const BARS_AREA_H: f64 = 32.0;
const BAR_W: f64 = 5.0;
const BAR_GAP: f64 = 4.0;
const BAR_WEIGHTS: [f64; 5] = [0.5, 0.8, 1.0, 0.75, 0.55];
const PAD_LEFT: f64 = 18.0;
const PAD_RIGHT: f64 = 22.0;
const LABEL_MIN_W: f64 = 160.0;
const LABEL_MAX_W: f64 = 560.0;
const MARGIN_BOTTOM: f64 = 80.0;

const BG: (f64, f64, f64, f64) = (0.12, 0.12, 0.14, 0.92);
const FG: (f64, f64, f64) = (0.95, 0.97, 1.0);

#[derive(PartialEq, Clone, Copy)]
enum Mode {
    Hidden,
    Entering,
    Shown,
    Exiting,
}

struct Anim {
    mode: Mode,
    entry_t: f64,
    exit_t: f64,
    cur_w: f64,
    target_w: f64,
    level: f64,
    bars: [f64; 5],
    jitter_seed: u64,
    text: String,
    refining: bool,
}

impl Anim {
    fn new() -> Self {
        Self {
            mode: Mode::Hidden,
            entry_t: 0.0,
            exit_t: 0.0,
            cur_w: min_width(),
            target_w: min_width(),
            level: 0.0,
            bars: [0.05; 5],
            jitter_seed: 0x9E37_79B9_7F4A_7C15,
            text: String::new(),
            refining: false,
        }
    }

    /// Deterministic cheap PRNG in [-0.5, 0.5] for per-bar jitter.
    fn next_rand(&mut self) -> f64 {
        self.jitter_seed = self
            .jitter_seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.jitter_seed >> 33) as f64 / (u32::MAX as f64)) - 0.5
    }

    /// Per-tick animation. Returns true when a redraw is needed.
    fn tick(&mut self, dt: f64) -> bool {
        let mut dirty = false;

        match self.mode {
            Mode::Entering => {
                self.entry_t = (self.entry_t + dt / 0.35).min(1.0); // 0.35s spring
                dirty = true;
                if self.entry_t >= 1.0 {
                    self.mode = Mode::Shown;
                }
            }
            Mode::Exiting => {
                self.exit_t = (self.exit_t + dt / 0.22).min(1.0); // 0.22s exit
                dirty = true;
                if self.exit_t >= 1.0 {
                    self.mode = Mode::Hidden;
                }
            }
            _ => {}
        }

        // Width easing (0.25 s feel) via exponential approach.
        if (self.cur_w - self.target_w).abs() > 0.5 {
            let k = 1.0 - (-dt / 0.085).exp(); // tau ≈ 85ms → settles in ~0.25s
            self.cur_w += (self.target_w - self.cur_w) * k;
            dirty = true;
        }

        // Waveform envelope: attack 40%, release 15%, ±4% jitter per bar.
        let lvl = (self.level * 6.0).clamp(0.0, 1.2).min(1.0);
        // Compute the shared jitter values first so the loop below does not
        // borrow `self` twice in one expression.
        let jitters: [f64; 5] =
            std::array::from_fn(|_| 1.0 + self.next_rand() * 0.08);
        for (i, bar) in self.bars.iter_mut().enumerate() {
            let jitter = jitters[i];
            let target = (lvl * BAR_WEIGHTS[i] * jitter).clamp(0.08, 1.0);
            let rate = if target > *bar { 0.40 } else { 0.15 };
            *bar += (target - *bar) * rate;
        }
        dirty
    }

    fn scale(&self) -> f64 {
        let ease_out_back = |t: f64| {
            let c1 = 1.70158;
            let c3 = c1 + 1.0;
            1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
        };
        match self.mode {
            Mode::Entering => ease_out_back(self.entry_t.clamp(0.0, 1.0)),
            Mode::Shown => 1.0,
            Mode::Exiting => 1.0 - 0.22 * self.exit_t,
            Mode::Hidden => 0.0,
        }
    }

    fn alpha(&self) -> f64 {
        match self.mode {
            Mode::Entering => self.entry_t,
            Mode::Shown => 1.0,
            Mode::Exiting => 1.0 - self.exit_t,
            Mode::Hidden => 0.0,
        }
    }
}

fn min_width() -> f64 {
    PAD_LEFT + BARS_AREA_W + 14.0 + LABEL_MIN_W + PAD_RIGHT
}

/// The bottom-center capsule HUD.
pub struct Capsule {
    window: gtk::Window,
    anim: Rc<RefCell<Anim>>,
}

impl Capsule {
    pub fn new() -> Self {
        let window = gtk::Window::new(gtk::WindowType::Popup);
        // The capsule must never take input focus: while it is up, the
        // simulated Ctrl+V has to land in the user's real focused window
        // (XTEST delivers keys to the current focus owner).
        window.set_accept_focus(false);
        window.set_focus_on_map(false);
        window.set_decorated(false);
        window.set_skip_taskbar_hint(true);
        window.set_skip_pager_hint(true);
        window.set_keep_above(true);
        window.set_resizable(false);
        window.set_default_size(min_width() as i32, CAPSULE_HEIGHT as i32);
        window.set_app_paintable(true);

        if let Some(screen) = gtk::gdk::Screen::default() {
            if let Some(visual) = screen.rgba_visual() {
                window.set_visual(Some(&visual));
            }
        }

        let anim = Rc::new(RefCell::new(Anim::new()));

        {
            let anim = Rc::clone(&anim);
            window.connect_draw(move |_w, cr| {
                draw_capsule(cr, &anim.borrow());
                glib::Propagation::Proceed
            });
        }

        // 60 fps animation tick, always running (near-free while hidden).
        {
            let anim = Rc::clone(&anim);
            let win = window.clone();
            let last = Cell::new(std::time::Instant::now());
            glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
                let now = std::time::Instant::now();
                let dt = now.duration_since(last.get()).as_secs_f64().min(0.05);
                last.set(now);

                let (dirty, w, hidden) = {
                    let mut a = anim.borrow_mut();
                    if a.mode == Mode::Hidden {
                        (false, 0.0, true)
                    } else {
                        let dirty = a.tick(dt);
                        (dirty, a.cur_w, a.mode == Mode::Hidden)
                    }
                };
                if hidden {
                    win.hide();
                    return glib::ControlFlow::Continue;
                }
                if dirty {
                    win.resize(w.ceil() as i32, CAPSULE_HEIGHT as i32);
                    win.queue_draw();
                }
                glib::ControlFlow::Continue
            });
        }

        Self { window, anim }
    }

    /// X11 fallback: bottom-center via monitor geometry.
    fn position_bottom_center(&self) {
        let Some(display) = gtk::gdk::Display::default() else {
            log::warn!("no gdk display; cannot position capsule");
            return;
        };
        let monitor = self
            .window
            .window()
            .and_then(|gdwin| display.monitor_at_window(&gdwin))
            .or_else(|| display.primary_monitor())
            .or_else(|| display.monitor(0));
        if let Some(m) = monitor {
            let geo = m.geometry();
            let w = self.anim.borrow().cur_w as i32;
            let x = geo.x() + (geo.width() - w) / 2;
            let y = geo.y() + geo.height() - CAPSULE_HEIGHT as i32 - MARGIN_BOTTOM as i32;
            self.window.move_(x, y);
        }
    }

    pub fn show(&self) {
        {
            let mut a = self.anim.borrow_mut();
            a.mode = Mode::Entering;
            a.entry_t = 0.0;
            a.exit_t = 0.0;
            a.text.clear();
            a.refining = false;
            a.level = 0.0;
            a.bars = [0.08; 5];
            a.target_w = min_width();
            a.cur_w = min_width();
        }
        // X11: position bottom-center on the current monitor.
        self.window.realize();
        // Override-redirect: the WM (user runs a minimal tiling WM that tiles
        // and focuses every mapped window, stealing focus from the typing
        // target) must not manage this OSD at all. Must be set after realize
        // and before the window is shown.
        if let Some(gdk_win) = self.window.window() {
            gdk_win.set_override_redirect(true);
        }
        self.position_bottom_center();
        self.window.resize(min_width() as i32, CAPSULE_HEIGHT as i32);
        self.window.show_all();
        self.window.queue_draw();
    }

    pub fn hide(&self) {
        let mut a = self.anim.borrow_mut();
        if a.mode == Mode::Hidden {
            return;
        }
        a.mode = Mode::Exiting;
        a.exit_t = 0.0;
    }

    pub fn set_level(&self, rms: f32) {
        self.anim.borrow_mut().level = rms as f64;
    }

    pub fn set_text(&self, text: &str) {
        {
            let mut a = self.anim.borrow_mut();
            a.text = text.to_string();
            a.refining = false;
        }
        self.recompute_width();
    }

    pub fn set_refining(&self) {
        {
            let mut a = self.anim.borrow_mut();
            a.text = "Refining…".to_string();
            a.refining = true;
        }
        self.recompute_width();
    }

    /// Elastic label width 160–560 px drives the capsule width target.
    fn recompute_width(&self) {
        let text = self.anim.borrow().text.clone();
        let layout = self.window.create_pango_layout(None);
        let mut fd = pango::FontDescription::from_string("Sans 14");
        fd.set_weight(pango::Weight::Medium);
        layout.set_font_description(Some(&fd));
        layout.set_width((LABEL_MAX_W * pango::SCALE as f64) as i32);
        layout.set_ellipsize(pango::EllipsizeMode::End);
        layout.set_text(&text);
        let (w, _h) = layout.pixel_size();
        let label_w = (w as f64).clamp(LABEL_MIN_W, LABEL_MAX_W);
        self.anim.borrow_mut().target_w =
            PAD_LEFT + BARS_AREA_W + 14.0 + label_w + PAD_RIGHT;
    }
}

fn rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::FRAC_PI_2;
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}

fn draw_capsule(cr: &cairo::Context, a: &Anim) {
    let alpha = a.alpha();
    if alpha <= 0.0 {
        return;
    }
    let w = a.cur_w;
    let h = CAPSULE_HEIGHT;
    let s = a.scale().max(0.01);

    cr.save();
    // Entry/exit transforms pivot at the capsule's bottom-center.
    cr.translate(w / 2.0, h);
    cr.scale(s, s);
    cr.translate(-w / 2.0, -h);

    // Background.
    rounded_rect(cr, 0.5, 0.5, w - 1.0, h - 1.0, CORNER_RADIUS);
    cr.set_source_rgba(BG.0, BG.1, BG.2, BG.3 * alpha);
    let _ = cr.fill();

    // Waveform bars.
    let bars_x = PAD_LEFT;
    let bars_y = (h - BARS_AREA_H) / 2.0;
    cr.set_source_rgba(FG.0, FG.1, FG.2, 0.95 * alpha);
    for (i, level) in a.bars.iter().enumerate() {
        let bh = (level * BARS_AREA_H).max(5.0).min(BARS_AREA_H); // min visible height
        let bx = bars_x + i as f64 * (BAR_W + BAR_GAP);
        let by = bars_y + (BARS_AREA_H - bh) / 2.0;
        rounded_rect(cr, bx, by, BAR_W, bh, BAR_W / 2.0);
        let _ = cr.fill();
    }

    // Label.
    let text_x = bars_x + BARS_AREA_W + 14.0;
    let text_w = (w - text_x - PAD_RIGHT).max(20.0);
    let layout = pangocairo::functions::create_layout(cr);
    layout.set_width((text_w * pango::SCALE as f64) as i32);
    layout.set_ellipsize(pango::EllipsizeMode::End);
    let mut fd = pango::FontDescription::from_string("Sans 14");
    fd.set_weight(pango::Weight::Medium);
    layout.set_font_description(Some(&fd));
    let escaped = glib::markup_escape_text(&a.text).to_string();
    if a.refining {
        layout.set_markup(&format!("<i>{escaped}</i>"));
    } else {
        layout.set_markup(&escaped);
    }
    let (_tw, th) = layout.pixel_size();
    cr.set_source_rgba(FG.0, FG.1, FG.2, alpha);
    cr.move_to(text_x, (h - th as f64) / 2.0);
    pangocairo::functions::show_layout(cr, &layout);

    cr.restore();
}

// ---------------------------------------------------------------------------
// Settings window
// ---------------------------------------------------------------------------

pub struct SettingsWindow {
    window: gtk::Window,
    base_entry: gtk::Entry,
    key_entry: gtk::Entry,
    model_entry: gtk::Entry,
    status: gtk::Label,
    config: SharedConfig,
}

impl SettingsWindow {
    pub fn new(config: SharedConfig) -> Rc<RefCell<Option<Rc<SettingsWindow>>>> {
        let storage: Rc<RefCell<Option<Rc<SettingsWindow>>>> = Rc::new(RefCell::new(None));

        let cfg_guard = config.lock().ok();
        let cfg = cfg_guard.as_deref().cloned().unwrap_or_default();
        drop(cfg_guard);

        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_title("voicetype 设置");
        window.set_default_size(470, 220);
        window.set_resizable(false);
        window.set_position(gtk::WindowPosition::Center);
        window.set_keep_above(true);

        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 10);
        vbox.set_margin_top(14);
        vbox.set_margin_bottom(14);
        vbox.set_margin_start(14);
        vbox.set_margin_end(14);
        window.add(&vbox);

        let grid = gtk::Grid::new();
        grid.set_row_spacing(8);
        grid.set_column_spacing(10);
        vbox.add(&grid);

        let mk_label = |t: &str| {
            let l = gtk::Label::new(Some(t));
            l.set_halign(gtk::Align::Start);
            l
        };

        grid.attach(&mk_label("API Base URL"), 0, 0, 1, 1);
        let base_entry = gtk::Entry::new();
        base_entry.set_text(&cfg.llm.base_url);
        base_entry.set_hexpand(true);
        base_entry.set_width_chars(36);
        grid.attach(&base_entry, 1, 0, 1, 1);

        grid.attach(&mk_label("API Key"), 0, 1, 1, 1);
        let key_entry = gtk::Entry::new();
        key_entry.set_text(&cfg.llm.api_key);
        key_entry.set_visibility(true); // fully visible & clearable
        grid.attach(&key_entry, 1, 1, 1, 1);

        grid.attach(&mk_label("Model"), 0, 2, 1, 1);
        let model_entry = gtk::Entry::new();
        model_entry.set_text(&cfg.llm.model);
        grid.attach(&model_entry, 1, 2, 1, 1);

        let status = gtk::Label::new(None);
        status.set_halign(gtk::Align::Start);
        status.set_valign(gtk::Align::Center);
        status.set_hexpand(true);
        status.set_wrap(true);
        status.set_xalign(0.0);
        grid.attach(&status, 0, 3, 2, 1);

        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        hbox.set_halign(gtk::Align::End);
        vbox.add(&hbox);

        let test_btn = gtk::Button::with_label("Test");
        let save_btn = gtk::Button::with_label("Save");
        hbox.add(&test_btn);
        hbox.add(&save_btn);

        // ---- Test: real one-request chat call, result shown inline ----
        {
            let (tx, rx) = glib::MainContext::channel::<Result<String, String>>(glib::Priority::DEFAULT);
            let cfg_holder = storage.clone();
            let status_btn = status.clone();
            test_btn.connect_clicked(move |_| {
                status_btn.set_text("Testing…");
                let sw = cfg_holder.borrow().as_ref().map(Rc::clone);
                let Some(sw) = sw else { return };
                let cfg = sw.collect();
                let tx = tx.clone();
                std::thread::spawn(move || {
                    let r = llm::test(&cfg);
                    let _ = tx.send(r);
                });
            });
            let status_rx = status.clone();
            rx.attach(None, move |r| {
                status_rx.set_text(&match r {
                    Ok(msg) => format!("✓ 成功: {msg}"),
                    Err(e) => format!("✗ 失败: {e}"),
                });
                glib::ControlFlow::Continue
            });
        }

        // ---- Save ----
        {
            let cfg_holder = storage.clone();
            let config = config.clone();
            let window = window.clone();
            let status = status.clone();
            save_btn.connect_clicked(move |_| {
                let Some(sw) = cfg_holder.borrow().as_ref().map(Rc::clone) else {
                    return;
                };
                let llm_cfg = sw.collect();
                if let Ok(mut c) = config.lock() {
                    c.llm.base_url = llm_cfg.base_url;
                    c.llm.api_key = llm_cfg.api_key;
                    c.llm.model = llm_cfg.model;
                    // Saving complete credentials implies refinement readiness;
                    // the explicit enable toggle lives in the tray menu.
                    if let Err(e) = c.save() {
                        log::error!("save config: {e}");
                    }
                }
                status.set_text("已保存");
                window.hide();
            });
        }

        window.connect_delete_event(|w, _e| {
            w.hide();
            glib::Propagation::Stop
        });

        *storage.borrow_mut() = Some(Rc::new(SettingsWindow {
            window,
            base_entry,
            key_entry,
            model_entry,
            status,
            config,
        }));
        storage
    }
}

impl SettingsWindow {
    fn collect(&self) -> LlmConfig {
        LlmConfig {
            enabled: true,
            base_url: self.base_entry.text().to_string(),
            api_key: self.key_entry.text().to_string(),
            model: self.model_entry.text().to_string(),
        }
    }

    pub fn show(storage: &Rc<RefCell<Option<Rc<SettingsWindow>>>>) {
        let exists = storage.borrow().as_ref().map(Rc::clone);
        if let Some(sw) = exists {
            sw.window.show_all();
            sw.window.present();
        }
    }
}
