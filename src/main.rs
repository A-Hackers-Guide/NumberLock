#![cfg_attr(windows, windows_subsystem = "windows")]

mod core;
mod fsio;

use eframe::egui::{self, Color32, CornerRadius, FontId, Margin, RichText, Stroke, Vec2};
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

const ICON_PNG: &[u8] = include_bytes!("../assets/icon-256.png");
const CONTENT_WIDTH: f32 = 520.0;

fn decode_png(bytes: &[u8]) -> (Vec<u8>, u32, u32) {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().expect("icon png");
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).expect("icon png");
    buf.truncate(info.buffer_size());
    (buf, info.width, info.height)
}

fn fmt_size(n: usize) -> String {
    match n {
        n if n < 1024 => format!("{n} B"),
        n if n < 1 << 20 => format!("{:.1} KB", n as f64 / 1024.0),
        n if n < 1 << 30 => format!("{:.2} MB", n as f64 / 1048576.0),
        n => format!("{:.2} GB", n as f64 / 1073741824.0),
    }
}

fn fmt_count(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[derive(Clone, Copy)]
struct Palette {
    bg: Color32,
    card: Color32,
    ink: Color32,
    muted: Color32,
    line: Color32,
    accent: Color32,
    on_accent: Color32,
    danger: Color32,
}

impl Palette {
    fn for_dark(dark: bool) -> Self {
        if dark {
            Palette {
                bg: Color32::from_rgb(0x15, 0x14, 0x0f),
                card: Color32::from_rgb(0x1f, 0x1d, 0x17),
                ink: Color32::from_rgb(0xec, 0xe6, 0xd8),
                muted: Color32::from_rgb(0xa3, 0x9b, 0x89),
                line: Color32::from_rgb(0x36, 0x32, 0x2a),
                accent: Color32::from_rgb(0x7f, 0xc3, 0xac),
                on_accent: Color32::from_rgb(0x10, 0x20, 0x1a),
                danger: Color32::from_rgb(0xe4, 0x8a, 0x78),
            }
        } else {
            Palette {
                bg: Color32::from_rgb(0xf4, 0xf1, 0xea),
                card: Color32::from_rgb(0xff, 0xfd, 0xf8),
                ink: Color32::from_rgb(0x1d, 0x1b, 0x16),
                muted: Color32::from_rgb(0x6b, 0x65, 0x58),
                line: Color32::from_rgb(0xdd, 0xd6, 0xc6),
                accent: Color32::from_rgb(0x2f, 0x5d, 0x50),
                on_accent: Color32::WHITE,
                danger: Color32::from_rgb(0xa2, 0x3b, 0x2a),
            }
        }
    }
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Lock,
    Unlock,
}

enum Done {
    Locked(core::Locked),
    Unlocked(core::Unlocked),
    Failed(String),
}

enum Status {
    None,
    Working(String),
    Ok(String),
    Err(String),
}

struct App {
    tab: Tab,
    logo: egui::TextureHandle,
    selection: Option<fsio::Selection>,
    pw: String,
    pw2: String,
    lock_status: Status,
    stats: Option<[(String, &'static str); 4]>,
    last_txt: Option<(String, String)>,
    txt_source: Option<(String, String)>, // (file name, contents)
    paste: String,
    unlock_pw: String,
    unlock_status: Status,
    job: Option<mpsc::Receiver<Done>>,
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        let (rgba, w, h) = decode_png(ICON_PNG);
        let logo = cc.egui_ctx.load_texture(
            "logo",
            egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba),
            egui::TextureOptions::LINEAR,
        );
        cc.egui_ctx.all_styles_mut(|s| {
            s.spacing.item_spacing = Vec2::new(8.0, 8.0);
            s.spacing.button_padding = Vec2::new(14.0, 8.0);
            s.text_styles.insert(egui::TextStyle::Body, FontId::proportional(15.0));
            s.text_styles.insert(egui::TextStyle::Button, FontId::proportional(15.0));
            s.text_styles.insert(egui::TextStyle::Monospace, FontId::monospace(12.5));
        });
        for dark in [false, true] {
            let p = Palette::for_dark(dark);
            let theme = if dark { egui::Theme::Dark } else { egui::Theme::Light };
            cc.egui_ctx.style_mut_of(theme, |s| {
                let v = &mut s.visuals;
                v.panel_fill = p.bg;
                v.extreme_bg_color = p.bg;
                v.override_text_color = Some(p.ink);
                v.selection.bg_fill = p.accent.gamma_multiply(0.35);
                v.selection.stroke = Stroke::new(1.0, p.accent);
                v.widgets.inactive.weak_bg_fill = p.bg;
                v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.line);
                v.widgets.inactive.corner_radius = CornerRadius::same(8);
                v.widgets.hovered.weak_bg_fill = p.card;
                v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent);
                v.widgets.hovered.corner_radius = CornerRadius::same(8);
                v.widgets.active.corner_radius = CornerRadius::same(8);
                v.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
                v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.line);
            });
        }
        App {
            tab: Tab::Lock,
            logo,
            selection: None,
            pw: String::new(),
            pw2: String::new(),
            lock_status: Status::None,
            stats: None,
            last_txt: None,
            txt_source: None,
            paste: String::new(),
            unlock_pw: String::new(),
            unlock_status: Status::None,
            job: None,
        }
    }

    fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn run(&mut self, ctx: &egui::Context, work: impl FnOnce() -> Done + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let _ = tx.send(work());
            ctx.request_repaint();
        });
        self.job = Some(rx);
    }

    fn set_selection(&mut self, paths: Vec<PathBuf>) {
        self.selection = fsio::select(paths);
        self.stats = None;
        self.last_txt = None;
        self.lock_status = Status::None;
    }

    fn load_txt(&mut self, path: &PathBuf) {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.txt_source = Some((name, text));
                self.paste.clear();
                self.unlock_status = Status::None;
            }
            Err(e) => self.unlock_status = Status::Err(format!("Couldn't read that file: {e}")),
        }
    }

    fn start_lock(&mut self, ctx: &egui::Context) {
        if self.pw != self.pw2 {
            self.lock_status = Status::Err("Passwords don't match.".into());
            return;
        }
        let Some(sel) = &self.selection else { return };
        let roots = sel.roots.clone();
        let pw = self.pw.clone();
        self.stats = None;
        self.last_txt = None;
        self.lock_status = Status::Working("Compressing, encrypting, turning into numbers…".into());
        self.run(ctx, move || {
            let Some(sel) = fsio::select(roots) else { return Done::Failed("Nothing to lock.".into()) };
            match fsio::collect(&sel).and_then(|entries| core::lock(&entries, &sel.kind, &sel.name, &pw)) {
                Ok(l) => Done::Locked(l),
                Err(e) => Done::Failed(e),
            }
        });
    }

    fn start_unlock(&mut self, ctx: &egui::Context) {
        let text = if !self.paste.trim().is_empty() {
            self.paste.clone()
        } else if let Some((_, t)) = &self.txt_source {
            t.clone()
        } else {
            self.unlock_status = Status::Err("Add a .txt or paste the numbers first.".into());
            return;
        };
        let pw = self.unlock_pw.clone();
        self.unlock_status = Status::Working("Unlocking…".into());
        self.run(ctx, move || match core::unlock(&text, &pw) {
            Ok(u) => Done::Unlocked(u),
            Err(e) => Done::Failed(e),
        });
    }

    fn save_txt(&mut self) {
        let Some((name, txt)) = &self.last_txt else { return };
        if let Some(path) = rfd::FileDialog::new().set_title("Save locked numbers").set_file_name(name).add_filter("Text", &["txt"]).save_file() {
            match std::fs::write(&path, txt) {
                Ok(()) => self.lock_status = Status::Ok(format!("Saved {}", path.display())),
                Err(e) => self.lock_status = Status::Err(format!("Couldn't save: {e}")),
            }
        } else {
            self.lock_status = Status::Ok("Locked. Not saved yet — use “Save .txt”.".into());
        }
    }

    fn finish(&mut self, done: Done) {
        match (self.tab, done) {
            (_, Done::Locked(l)) => {
                let name = self.selection.as_ref().map(|s| s.name.clone()).unwrap_or_else(|| "files".into());
                self.stats = Some([
                    (fmt_size(l.raw_size), "original"),
                    (fmt_size(l.packed_size), "after compression"),
                    (fmt_count(l.digits), "digits"),
                    (fmt_size(l.txt.len()), ".txt size"),
                ]);
                self.last_txt = Some((format!("{name}.locked.txt"), l.txt));
                self.save_txt();
            }
            (_, Done::Unlocked(u)) => self.unlock_status = save_unlocked(u),
            (Tab::Lock, Done::Failed(e)) => self.lock_status = Status::Err(e),
            (Tab::Unlock, Done::Failed(e)) => self.unlock_status = Status::Err(e),
        }
    }
}

fn save_unlocked(u: core::Unlocked) -> Status {
    if u.kind == "file" && u.entries.len() == 1 {
        let Some(dest) = rfd::FileDialog::new().set_title("Save unlocked file").set_file_name(&u.name).save_file() else {
            return Status::Err("Unlocked, but not saved. Press Unlock again to save.".into());
        };
        return match fsio::write_single(&u.entries[0], &dest) {
            Ok(()) => Status::Ok(format!("Unlocked! Saved {}", dest.display())),
            Err(e) => Status::Err(format!("Couldn't save: {e}")),
        };
    }
    let Some(dir) = rfd::FileDialog::new().set_title(format!("Where should “{}” go?", u.name)).pick_folder() else {
        return Status::Err("Unlocked, but not saved. Press Unlock again to save.".into());
    };
    match fsio::restore(&u.kind, &u.name, &u.entries, &dir) {
        Ok((root, 0)) => Status::Ok(format!("Unlocked! {} item(s) restored to {}", u.entries.len(), root.display())),
        Ok((root, skipped)) => Status::Ok(format!(
            "Unlocked to {}. {skipped} shortcut link(s) were skipped (this system can't create them).",
            root.display()
        )),
        Err(e) => Status::Err(format!("Couldn't restore: {e}")),
    }
}

fn card(ui: &mut egui::Ui, p: Palette, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::default()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(20))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        });
}

fn drop_zone(ui: &mut egui::Ui, p: Palette, hovering: bool, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::default()
        .fill(if hovering { p.accent.gamma_multiply(0.12) } else { p.bg })
        .stroke(Stroke::new(if hovering { 2.0 } else { 1.0 }, if hovering { p.accent } else { p.line }))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(16, 22))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(add);
        });
}

fn primary(ui: &mut egui::Ui, p: Palette, label: &str, enabled: bool) -> bool {
    let btn = egui::Button::new(RichText::new(label).size(16.0).strong().color(p.on_accent))
        .fill(p.accent)
        .corner_radius(CornerRadius::same(10))
        .min_size(Vec2::new(ui.available_width(), 46.0));
    ui.add_enabled(enabled, btn).clicked()
}

fn field(ui: &mut egui::Ui, p: Palette, label: &str, value: &mut String, password: bool) {
    ui.label(RichText::new(label).size(13.0).strong().color(p.ink));
    ui.add(
        egui::TextEdit::singleline(value)
            .password(password)
            .desired_width(f32::INFINITY)
            .margin(Margin::symmetric(10, 8)),
    );
    ui.add_space(4.0);
}

fn status(ui: &mut egui::Ui, p: Palette, s: &Status) {
    match s {
        Status::None => {}
        Status::Working(m) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(m);
            });
        }
        Status::Ok(m) => {
            ui.label(RichText::new(m).color(p.accent));
        }
        Status::Err(m) => {
            ui.label(RichText::new(m).color(p.danger));
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(rx) = &self.job {
            match rx.try_recv() {
                Ok(done) => {
                    self.job = None;
                    self.finish(done);
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
                Err(mpsc::TryRecvError::Disconnected) => self.job = None,
            }
        }

        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if !dropped.is_empty() && !self.busy() {
            match self.tab {
                Tab::Lock => self.set_selection(dropped),
                Tab::Unlock => self.load_txt(&dropped[0]),
            }
        }

        let p = Palette::for_dark(ctx.style().visuals.dark_mode);
        egui::CentralPanel::default().frame(egui::Frame::default().fill(p.bg)).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                let pad = ((ui.available_width() - CONTENT_WIDTH) / 2.0).max(18.0);
                ui.horizontal(|ui| {
                    ui.add_space(pad);
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width() - pad);
                        ui.add_space(26.0);
                        self.content(ui, ctx, p, hovering);
                        ui.add_space(26.0);
                    });
                });
            });
        });
    }
}

impl App {
    fn content(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, p: Palette, hovering: bool) {
        ui.horizontal(|ui| {
            ui.add(egui::Image::new(&self.logo).fit_to_exact_size(Vec2::splat(52.0)));
            ui.vertical(|ui| {
                ui.add_space(2.0);
                ui.label(RichText::new("NumberLock").size(26.0).strong().color(p.ink));
                ui.label(RichText::new("Turn files and folders into password-locked numbers, and back.").color(p.muted));
            });
        });
        ui.add_space(14.0);

        // Segmented tab control
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 8.0) / 2.0;
            for (tab, label) in [(Tab::Lock, "Lock into numbers"), (Tab::Unlock, "Unlock back to files")] {
                let on = self.tab == tab;
                let btn = egui::Button::new(RichText::new(label).strong().color(if on { p.on_accent } else { p.ink }))
                    .fill(if on { p.accent } else { p.card })
                    .stroke(Stroke::new(1.0, if on { p.accent } else { p.line }))
                    .corner_radius(CornerRadius::same(10))
                    .min_size(Vec2::new(w, 42.0));
                if ui.add_enabled(!self.busy() || on, btn).clicked() {
                    self.tab = tab;
                }
            }
        });
        ui.add_space(10.0);

        match self.tab {
            Tab::Lock => self.lock_card(ui, ctx, p, hovering),
            Tab::Unlock => self.unlock_card(ui, ctx, p, hovering),
        }

        ui.add_space(14.0);
        ui.label(
            RichText::new(
                "AES-256-GCM encryption with a key stretched from your password (PBKDF2-SHA256, 600,000 rounds). \
                 Everything happens on this computer. If the password is lost, the data is gone for good.",
            )
            .size(12.5)
            .color(p.muted),
        );
    }

    fn lock_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, p: Palette, hovering: bool) {
        let busy = self.busy();
        card(ui, p, |ui| {
            drop_zone(ui, p, hovering, |ui| {
                ui.label(RichText::new("Drop a file, folder or app here").strong().size(16.0).color(p.ink));
                ui.label(RichText::new("or").color(p.muted));
                ui.horizontal(|ui| {
                    let w = 140.0 * 2.0 + 8.0;
                    ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
                    if ui.add_enabled(!busy, egui::Button::new("Choose files…").min_size(Vec2::new(140.0, 34.0))).clicked() {
                        if let Some(files) = rfd::FileDialog::new().set_title("Choose files to lock").pick_files() {
                            self.set_selection(files);
                        }
                    }
                    if ui.add_enabled(!busy, egui::Button::new("Choose folder…").min_size(Vec2::new(140.0, 34.0))).clicked() {
                        if let Some(dir) = rfd::FileDialog::new().set_title("Choose a folder to lock").pick_folder() {
                            self.set_selection(vec![dir]);
                        }
                    }
                });
                if let Some(sel) = &self.selection {
                    ui.add_space(6.0);
                    let icon = if sel.kind == "file" { "File" } else { "Folder" };
                    ui.label(
                        RichText::new(format!("{icon}: {}  ·  {} file(s), {}", sel.name, fmt_count(sel.files), fmt_size(sel.bytes as usize)))
                            .monospace()
                            .color(p.ink),
                    );
                }
            });
            ui.add_space(12.0);
            field(ui, p, "Password", &mut self.pw, true);
            field(ui, p, "Confirm password", &mut self.pw2, true);
            ui.add_space(6.0);
            if primary(ui, p, "Lock it", !busy && self.selection.is_some() && !self.pw.is_empty()) {
                self.start_lock(ctx);
            }
            ui.add_space(6.0);
            status(ui, p, &self.lock_status);

            if let Some(stats) = &self.stats {
                ui.add_space(6.0);
                ui.columns(4, |cols| {
                    for (col, (value, label)) in cols.iter_mut().zip(stats.iter()) {
                        egui::Frame::default()
                            .stroke(Stroke::new(1.0, p.line))
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(Margin::same(10))
                            .show(col, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(RichText::new(value).monospace().size(15.0).strong().color(p.ink));
                                ui.label(RichText::new(*label).size(11.5).color(p.muted));
                            });
                    }
                });
            }
            if self.last_txt.is_some() {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("Copy numbers").clicked() {
                        if let Some((_, txt)) = &self.last_txt {
                            ctx.copy_text(txt.clone());
                            self.lock_status = Status::Ok("Copied to clipboard.".into());
                        }
                    }
                    if ui.button("Save .txt…").clicked() {
                        self.save_txt();
                    }
                });
            }
        });
    }

    fn unlock_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, p: Palette, hovering: bool) {
        let busy = self.busy();
        card(ui, p, |ui| {
            drop_zone(ui, p, hovering, |ui| {
                ui.label(RichText::new("Drop a NumberLock .txt here").strong().size(16.0).color(p.ink));
                if ui.add_enabled(!busy, egui::Button::new("Choose .txt…").min_size(Vec2::new(140.0, 34.0))).clicked() {
                    if let Some(f) = rfd::FileDialog::new().set_title("Choose a locked .txt").add_filter("Text", &["txt"]).pick_file() {
                        self.load_txt(&f);
                    }
                }
                if let Some((name, text)) = &self.txt_source {
                    ui.add_space(6.0);
                    ui.label(RichText::new(format!("{name}  ·  {}", fmt_size(text.len()))).monospace().color(p.ink));
                }
            });
            ui.add_space(10.0);
            ui.vertical_centered(|ui| ui.label(RichText::new("or paste the numbers").size(13.0).color(p.muted)));
            let changed = ui
                .add(
                    egui::TextEdit::multiline(&mut self.paste)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("Paste numbers here…")
                        .desired_rows(5)
                        .desired_width(f32::INFINITY),
                )
                .changed();
            if changed && !self.paste.is_empty() {
                self.txt_source = None;
            }
            ui.add_space(8.0);
            field(ui, p, "Password", &mut self.unlock_pw, true);
            ui.add_space(6.0);
            if primary(ui, p, "Unlock", !busy) {
                self.start_unlock(ctx);
            }
            ui.add_space(6.0);
            status(ui, p, &self.unlock_status);
        });
    }
}

/// Command-line use: `numberlock lock <path>... -o out.txt` / `numberlock unlock in.txt -o <folder>`,
/// with the password in the NUMBERLOCK_PASSWORD environment variable.
fn cli(args: &[String]) -> Result<(), String> {
    let pw = std::env::var("NUMBERLOCK_PASSWORD").map_err(|_| "Set NUMBERLOCK_PASSWORD")?;
    let o = args.iter().position(|a| a == "-o").ok_or("Missing -o <output>")?;
    let out = PathBuf::from(args.get(o + 1).ok_or("Missing -o <output>")?);
    let inputs: Vec<PathBuf> = args[1..o].iter().map(PathBuf::from).collect();
    match args[0].as_str() {
        "lock" => {
            let sel = fsio::select(inputs).ok_or("Nothing to lock")?;
            let l = core::lock(&fsio::collect(&sel)?, &sel.kind, &sel.name, &pw)?;
            std::fs::write(&out, l.txt).map_err(|e| e.to_string())
        }
        "unlock" => {
            let text = std::fs::read_to_string(inputs.first().ok_or("Missing input .txt")?).map_err(|e| e.to_string())?;
            let u = core::unlock(&text, &pw)?;
            if u.kind == "file" && u.entries.len() == 1 {
                std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
                fsio::write_single(&u.entries[0], &out.join(&u.name))
            } else {
                fsio::restore(&u.kind, &u.name, &u.entries, &out).map(|_| ())
            }
        }
        _ => Err("Use: lock <paths…> -o out.txt | unlock in.txt -o folder".into()),
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("lock" | "unlock")) {
        if let Err(e) = cli(&args) {
            eprintln!("numberlock: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    let (rgba, width, height) = decode_png(ICON_PNG);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("NumberLock")
            .with_inner_size([600.0, 780.0])
            .with_min_inner_size([420.0, 560.0])
            .with_app_id("numberlock")
            .with_icon(Arc::new(egui::IconData { rgba, width, height })),
        ..Default::default()
    };
    eframe::run_native("NumberLock", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
