#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::env;
use std::fs;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

use eframe::egui::{self, vec2, Color32, RichText, Sense, Stroke};
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

const APP_NAME: &str = "Junkcleaner";
const REPARSE_POINT: u32 = 0x400;

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(env::var(name).unwrap_or_default())
}

fn windir() -> PathBuf {
    env::var("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\Windows"))
}

fn install_dir() -> PathBuf {
    env_path("LOCALAPPDATA").join("Junkcleaner")
}

fn is_admin() -> bool {
    unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
}

fn norm(p: &Path) -> String {
    p.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn in_safe_root(path: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }
    let p = norm(path);
    let roots = [
        env_path("LOCALAPPDATA"),
        env_path("APPDATA"),
        env_path("TEMP"),
        windir().join("Temp"),
    ];
    roots
        .iter()
        .filter(|r| !r.as_os_str().is_empty())
        .map(|r| norm(r))
        .any(|root| p == root || p.starts_with(&format!("{root}\\")))
}

fn is_reparse(meta: &fs::Metadata) -> bool {
    meta.file_attributes() & REPARSE_POINT != 0
}

fn folder_size(path: &Path) -> u64 {
    let mut total = 0;
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    for entry in entries.flatten() {
        let Ok(meta) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if is_reparse(&meta) {
            continue;
        }
        if meta.is_dir() {
            total += folder_size(&entry.path());
        } else {
            total += meta.len();
        }
    }
    total
}

/// Returns (deleted, locked) in bytes.
fn clean_folder(path: &Path, log: &dyn Fn(String)) -> (u64, u64) {
    if !path.is_dir() {
        return (0, 0);
    }
    if !in_safe_root(path) {
        log(format!(
            "  springer over (uden for de sikre mapper): {}",
            path.display()
        ));
        return (0, 0);
    }
    let Ok(entries) = fs::read_dir(path) else {
        return (0, 0);
    };

    let (mut deleted, mut locked) = (0u64, 0u64);
    for entry in entries.flatten() {
        let p = entry.path();
        let Ok(meta) = fs::symlink_metadata(&p) else {
            continue;
        };
        if is_reparse(&meta) {
            continue;
        }
        if meta.is_dir() {
            let before = folder_size(&p);
            let _ = fs::remove_dir_all(&p);
            let after = folder_size(&p);
            deleted += before.saturating_sub(after);
            locked += after;
        } else {
            let size = meta.len();
            if fs::remove_file(&p).is_ok() {
                deleted += size;
            } else {
                locked += size;
            }
        }
    }
    (deleted, locked)
}

fn fmt(n: u64) -> String {
    let mut n = n as f64;
    for u in ["B", "KB", "MB", "GB"] {
        if n < 1024.0 {
            return format!("{n:.2} {u}");
        }
        n /= 1024.0;
    }
    format!("{n:.2} TB")
}

#[derive(Clone)]
struct Item {
    name: &'static str,
    desc: &'static str,
    paths: Vec<PathBuf>,
    enabled: bool,
    admin: bool,
}

fn default_items(admin: bool) -> Vec<Item> {
    let local = env_path("LOCALAPPDATA");
    let roaming = env_path("APPDATA");
    let fivem = local.join(r"FiveM\FiveM.app\data");
    let discord = roaming.join("discord");
    vec![
        Item {
            name: "Midlertidige brugerfiler",
            desc: "%TEMP% - din temp-mappe",
            paths: vec![env_path("TEMP")],
            enabled: true,
            admin: false,
        },
        Item {
            name: "Midlertidige Windows-filer",
            desc: "kræver du kører som administrator",
            paths: vec![windir().join("Temp")],
            enabled: admin,
            admin: true,
        },
        Item {
            name: "DirectX shader-cache",
            desc: "bygges op igen af sig selv",
            paths: vec![local.join("D3DSCache")],
            enabled: true,
            admin: false,
        },
        Item {
            name: "FiveM-cache",
            desc: "rører kun din cache",
            paths: vec![
                fivem.join("cache"),
                fivem.join("server-cache"),
                fivem.join("server-cache-priv"),
            ],
            enabled: true,
            admin: false,
        },
        Item {
            name: "Discord-cache",
            desc: "luk Discord først for bedste resultat",
            paths: vec![
                discord.join("Cache"),
                discord.join("Code Cache"),
                discord.join("GPUCache"),
            ],
            enabled: false,
            admin: false,
        },
    ]
}

enum Msg {
    Progress(usize),
    Log(String),
    ScanDone(Vec<u64>),
    CleanDone {
        total: u64,
        locked: u64,
        results: Vec<(&'static str, u64, u64)>,
    },
}

const BG: Color32 = Color32::from_rgb(0x12, 0x12, 0x12);
const HEADER: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x1a);
const SUB: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x1e);
const CHIP: Color32 = Color32::from_rgb(0x25, 0x25, 0x25);
const HOVER: Color32 = Color32::from_rgb(0x2c, 0x2c, 0x2c);
const BORDER: Color32 = Color32::from_rgb(0x33, 0x33, 0x33);
const GRAY300: Color32 = Color32::from_rgb(0xd1, 0xd5, 0xdb);
const GRAY400: Color32 = Color32::from_rgb(0x9c, 0xa3, 0xaf);
const GRAY500: Color32 = Color32::from_rgb(0x6b, 0x72, 0x80);
const GRAY600: Color32 = Color32::from_rgb(0x4b, 0x55, 0x63);
const GRAY700: Color32 = Color32::from_rgb(0x37, 0x41, 0x51);
const BLUE400: Color32 = Color32::from_rgb(0x60, 0xa5, 0xfa);
const BLUE500: Color32 = Color32::from_rgb(0x3b, 0x82, 0xf6);
const BLUE600: Color32 = Color32::from_rgb(0x25, 0x63, 0xeb);

const RADIUS: f32 = 3.0;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Primary,
    Secondary,
    Cancel,
    Confirm,
}

fn load_fonts(ctx: &egui::Context) {
    let Ok(bytes) = fs::read(windir().join(r"Fonts\segoeui.ttf")) else {
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("segoe".into(), egui::FontData::from_owned(bytes));
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "segoe".into());
    ctx.set_fonts(fonts);
}

fn apply_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let rounding = egui::Rounding::same(RADIUS);
    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = BG;
    v.extreme_bg_color = SUB;
    v.faint_bg_color = SUB;
    v.window_stroke = Stroke::new(1.0_f32, BORDER);
    v.window_shadow = egui::epaint::Shadow::NONE;
    v.popup_shadow = egui::epaint::Shadow::NONE;
    v.window_rounding = rounding;
    v.selection.bg_fill = BLUE500;
    v.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    v.widgets.noninteractive.bg_fill = BG;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, GRAY300);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.rounding = rounding;
        w.expansion = 0.0;
    }
    v.widgets.inactive.weak_bg_fill = CHIP;
    v.widgets.inactive.bg_fill = CHIP;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, GRAY300);
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.hovered.bg_fill = HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, BORDER);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    v.widgets.active.weak_bg_fill = HOVER;
    v.widgets.active.bg_fill = HOVER;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, BLUE500);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);

    style.spacing.item_spacing = vec2(8.0, 8.0);
    style.spacing.scroll = egui::style::ScrollStyle::thin();
    style.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
    style.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
    style.text_styles.insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
    style.text_styles.insert(egui::TextStyle::Monospace, egui::FontId::monospace(12.5));
    ctx.set_style(style);
}

fn kind_colors(kind: Kind) -> (Color32, Color32, Color32, Stroke) {
    match kind {
        Kind::Primary => (BLUE500, BLUE400, Color32::WHITE, Stroke::NONE),
        Kind::Secondary => (CHIP, HOVER, GRAY300, Stroke::new(1.0_f32, BORDER)),
        Kind::Cancel => (GRAY600, GRAY500, Color32::WHITE, Stroke::NONE),
        Kind::Confirm => (BLUE600, BLUE500, Color32::WHITE, Stroke::NONE),
    }
}

fn styled_button(ui: &mut egui::Ui, text: &str, kind: Kind, enabled: bool, small: bool) -> bool {
    ui.scope(|ui| {
        let (fill, over, fg, stroke) = kind_colors(kind);
        let (fill, over, fg, stroke) = if enabled {
            (fill, over, fg, stroke)
        } else {
            (CHIP, CHIP, GRAY500, Stroke::new(1.0_f32, BORDER))
        };
        ui.spacing_mut().button_padding = if small { vec2(10.0, 3.0) } else { vec2(16.0, 6.0) };
        let w = &mut ui.visuals_mut().widgets;
        for (state, bg) in [(&mut w.inactive, fill), (&mut w.hovered, over), (&mut w.active, over)] {
            state.weak_bg_fill = bg;
            state.bg_fill = bg;
            state.bg_stroke = stroke;
            state.fg_stroke = Stroke::new(1.0_f32, fg);
        }
        let size = if small { 13.0 } else { 14.0 };
        ui.add_enabled(
            enabled,
            egui::Button::new(RichText::new(text).size(size)),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    })
    .inner
}

fn chip(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::none()
        .fill(CHIP)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .rounding(RADIUS)
        .inner_margin(egui::Margin::symmetric(8.0, 2.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(12.0).color(color));
        });
}

fn checkbox(ui: &mut egui::Ui, checked: bool, locked: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
    let painter = ui.painter();
    if checked {
        let fill = if locked { GRAY600 } else { BLUE400 };
        painter.rect_filled(rect, RADIUS, fill);
        let p = |x: f32, y: f32| rect.min + vec2(x * 20.0, y * 20.0);
        painter.add(egui::Shape::line(
            vec![p(0.24, 0.52), p(0.43, 0.70), p(0.76, 0.32)],
            Stroke::new(2.0_f32, Color32::WHITE),
        ));
    } else {
        painter.rect(rect, RADIUS, SUB, Stroke::new(1.0_f32, BORDER));
    }
}

struct Modal {
    text: String,
    confirm: Option<Vec<usize>>,
}

enum ModalAction {
    Close,
    Clean(Vec<usize>),
}

struct App {
    admin: bool,
    items: Vec<Item>,
    sizes: Vec<Option<u64>>,
    working: bool,
    progress: (usize, usize),
    summary: String,
    log: String,
    modal: Option<Modal>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl App {
    fn new() -> Self {
        let admin = is_admin();
        let items = default_items(admin);
        let (tx, rx) = channel();
        Self {
            admin,
            sizes: vec![None; items.len()],
            items,
            working: false,
            progress: (0, 1),
            summary: "Klik 'Scan' for at starte scanning.".into(),
            log: String::new(),
            modal: None,
            tx,
            rx,
        }
    }

    fn log_line(&mut self, text: &str) {
        self.log.push_str(text);
        self.log.push('\n');
    }

    fn scan(&mut self, ctx: &egui::Context) {
        if self.working {
            return;
        }
        self.working = true;
        self.progress = (0, self.items.len());
        self.summary = "Scanner...".into();
        self.log_line("Scanner...");

        let paths: Vec<Vec<PathBuf>> = self.items.iter().map(|i| i.paths.clone()).collect();
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        thread::spawn(move || {
            let mut sizes = Vec::new();
            for (i, ps) in paths.iter().enumerate() {
                sizes.push(ps.iter().map(|p| folder_size(p)).sum());
                let _ = tx.send(Msg::Progress(i + 1));
                ctx.request_repaint();
            }
            let _ = tx.send(Msg::ScanDone(sizes));
            ctx.request_repaint();
        });
    }

    fn confirm_clean(&mut self) {
        if self.working {
            return;
        }
        let selected: Vec<usize> = (0..self.items.len())
            .filter(|&i| self.items[i].enabled)
            .collect();
        if selected.is_empty() {
            self.modal = Some(Modal {
                text: "Vælg mindst én ting der skal ryddes.".into(),
                confirm: None,
            });
            return;
        }
        let amount: u64 = selected.iter().filter_map(|&i| self.sizes[i]).sum();
        self.modal = Some(Modal {
            text: format!(
                "Ryd de valgte ting?\n\nCirka {} bliver fjernet.\n\n\
                 Filer der er i brug lige nu bliver sprunget over.",
                fmt(amount)
            ),
            confirm: Some(selected),
        });
    }

    fn start_clean(&mut self, ctx: &egui::Context, selected: Vec<usize>) {
        if self.working {
            return;
        }
        self.working = true;
        self.progress = (0, selected.len());
        self.summary = "Rydder...".into();
        self.log_line("Rydder...");

        let jobs: Vec<(&'static str, Vec<PathBuf>)> = selected
            .iter()
            .map(|&i| (self.items[i].name, self.items[i].paths.clone()))
            .collect();
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        thread::spawn(move || {
            let (mut total, mut locked_total) = (0, 0);
            let mut results = Vec::new();
            let log_tx = tx.clone();
            let log_ctx = ctx.clone();
            let log = move |m: String| {
                let _ = log_tx.send(Msg::Log(m));
                log_ctx.request_repaint();
            };
            for (i, (name, paths)) in jobs.iter().enumerate() {
                let (mut d, mut l) = (0, 0);
                for p in paths {
                    let (pd, pl) = clean_folder(p, &log);
                    d += pd;
                    l += pl;
                }
                total += d;
                locked_total += l;
                results.push((*name, d, l));
                let _ = tx.send(Msg::Progress(i + 1));
                ctx.request_repaint();
            }
            let _ = tx.send(Msg::CleanDone {
                total,
                locked: locked_total,
                results,
            });
            ctx.request_repaint();
        });
    }

    fn save_log(&mut self) {
        let name = format!("junkcleaner-{}.txt", chrono::Local::now().format("%Y-%m-%d"));
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(name)
            .add_filter("Tekstfil", &["txt"])
            .save_file()
        else {
            return;
        };
        if let Err(e) = fs::write(&path, &self.log) {
            self.modal = Some(Modal {
                text: format!("Kunne ikke gemme:\n{e}"),
                confirm: None,
            });
        }
    }

    fn handle_messages(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Progress(n) => self.progress.0 = n,
                Msg::Log(m) => self.log_line(&m),
                Msg::ScanDone(sizes) => {
                    let total: u64 = sizes.iter().sum();
                    self.sizes = sizes.into_iter().map(Some).collect();
                    self.working = false;
                    self.summary = "Scan færdig.".into();
                    self.log_line(&format!("Scan færdig - {} fundet.", fmt(total)));
                }
                Msg::CleanDone { total, locked, results } => {
                    for (name, d, l) in results {
                        let mut line = format!("[OK] {name}: {} fjernet", fmt(d));
                        if l > 0 {
                            line += &format!("  ({} sprunget over - i brug)", fmt(l));
                        }
                        self.log_line(&line);
                    }
                    self.log_line("");
                    self.log_line(&format!("Færdig. I alt fjernet: {}", fmt(total)));
                    self.working = false;
                    self.summary = format!("Færdig - fjernede {}.", fmt(total));

                    let mut text = format!("Rydning færdig!\n\nFjernet: {}", fmt(total));
                    if locked > 0 {
                        text += &format!("\nSprunget over (i brug): {}", fmt(locked));
                    }
                    self.modal = Some(Modal { text, confirm: None });
                    self.scan(ctx);
                }
            }
        }
    }

    fn header(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::none()
                    .fill(HEADER)
                    .inner_margin(egui::Margin::symmetric(16.0, 12.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(APP_NAME).size(18.0).color(Color32::WHITE));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let count = self.items.iter().filter(|i| i.enabled).count();
                        chip(ui, &format!("{count} valgt"), BLUE400);
                        if self.admin {
                            chip(ui, "Administrator", BLUE400);
                        } else {
                            chip(ui, "Uden administrator", GRAY400);
                        }
                    });
                });
            });
    }

    fn toolbar(&mut self, ctx: &egui::Context) {
        let mut scan = false;
        let mut clean = false;
        egui::TopBottomPanel::top("toolbar")
            .frame(
                egui::Frame::none()
                    .fill(SUB)
                    .inner_margin(egui::Margin::symmetric(16.0, 8.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let total: Option<u64> = if self.sizes.iter().all(|s| s.is_some()) {
                        Some(self.sizes.iter().flatten().sum())
                    } else {
                        None
                    };
                    ui.label(RichText::new("Fundet:").color(GRAY400));
                    ui.label(
                        RichText::new(total.map(fmt).unwrap_or_else(|| "-".into()))
                            .color(BLUE400),
                    );
                    ui.add_space(8.0);
                    ui.label(RichText::new(&self.summary).size(12.0).color(GRAY500));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        clean = styled_button(ui, "Ryd valgte", Kind::Primary, !self.working, false);
                        scan = styled_button(ui, "Scan", Kind::Secondary, !self.working, false);
                    });
                });
            });
        if scan {
            self.scan(ctx);
        }
        if clean {
            self.confirm_clean();
        }
    }

    fn progress_strip(&self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("progress")
            .exact_height(2.0)
            .show_separator_line(false)
            .frame(egui::Frame::none().fill(BG))
            .show(ctx, |ui| {
                let (done, max) = self.progress;
                if self.working && done > 0 {
                    let rect = ui.max_rect();
                    let w = rect.width() * done as f32 / max.max(1) as f32;
                    let bar = egui::Rect::from_min_size(rect.min, vec2(w, rect.height()));
                    ui.painter().rect_filled(bar, 0.0, BLUE500);
                }
            });
    }

    fn rows(&mut self, ui: &mut egui::Ui) {
        for i in 0..self.items.len() {
            let locked = self.items[i].admin && !self.admin;
            if locked {
                self.items[i].enabled = false;
            }
            let (rect, resp) = ui.allocate_exact_size(
                vec2(ui.available_width(), 52.0),
                if locked { Sense::hover() } else { Sense::click() },
            );
            let border = if resp.hovered() && !locked { BLUE500 } else { BORDER };
            ui.painter().rect(rect, RADIUS, SUB, Stroke::new(1.0_f32, border));

            let item = &self.items[i];
            let builder = egui::UiBuilder::new()
                .max_rect(rect.shrink2(vec2(12.0, 0.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center));
            let size = self.sizes[i];
            ui.allocate_new_ui(builder, |ui| {
                ui.spacing_mut().item_spacing = vec2(12.0, 0.0);
                checkbox(ui, item.enabled, locked);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 2.0);
                    ui.horizontal(|ui| {
                        let name_color = if locked { GRAY500 } else { Color32::WHITE };
                        ui.label(RichText::new(item.name).color(name_color));
                        if item.admin {
                            egui::Frame::none()
                                .fill(GRAY700)
                                .rounding(RADIUS)
                                .inner_margin(egui::Margin::symmetric(6.0, 0.0))
                                .show(ui, |ui| {
                                    ui.label(RichText::new("admin").size(11.0).color(GRAY300));
                                });
                        }
                    });
                    ui.label(RichText::new(item.desc).size(12.0).color(GRAY400));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let text = size.map(fmt).unwrap_or_else(|| "-".into());
                    ui.label(RichText::new(text).color(Color32::WHITE));
                });
            });

            if !locked {
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.items[i].enabled = !self.items[i].enabled;
                }
            }
        }
    }

    fn log_view(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Log").size(12.0).color(GRAY400));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if styled_button(ui, "Gem log", Kind::Secondary, true, true) {
                    self.save_log();
                }
            });
        });
        let height = (ui.available_height() - 20.0).max(40.0);
        egui::Frame::none()
            .fill(SUB)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .rounding(RADIUS)
            .inner_margin(egui::Margin::symmetric(10.0, 8.0))
            .show(ui, |ui| {
                ui.set_min_size(vec2(ui.available_width(), height));
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .max_height(height)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(&self.log).monospace().color(GRAY300))
                                .selectable(true),
                        );
                    });
            });
    }

    fn modal_view(&mut self, ctx: &egui::Context) -> Option<ModalAction> {
        let modal = self.modal.as_ref()?;
        let mut action = None;

        egui::Area::new("backdrop".into())
            .order(egui::Order::Middle)
            .fixed_pos(egui::Pos2::ZERO)
            .show(ctx, |ui| {
                let rect = ctx.screen_rect();
                ui.painter().rect_filled(rect, 0.0, Color32::from_black_alpha(128));
                ui.allocate_rect(rect, Sense::click_and_drag());
            });

        egui::Window::new("modal")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .fixed_size([440.0, 0.0])
            .frame(
                egui::Frame::none()
                    .fill(BG)
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .rounding(RADIUS),
            )
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                egui::Frame::none()
                    .fill(HEADER)
                    .inner_margin(egui::Margin::symmetric(16.0, 10.0))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(APP_NAME).size(15.0).color(Color32::WHITE));
                    });
                ui.separator();
                egui::Frame::none()
                    .inner_margin(egui::Margin::same(24.0))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(&modal.text).color(GRAY300));
                        ui.add_space(24.0);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing = vec2(12.0, 0.0);
                            match &modal.confirm {
                                Some(selected) => {
                                    if styled_button(ui, "Ryd", Kind::Confirm, true, false) {
                                        action = Some(ModalAction::Clean(selected.clone()));
                                    }
                                    if styled_button(ui, "Annuller", Kind::Cancel, true, false) {
                                        action = Some(ModalAction::Close);
                                    }
                                }
                                None => {
                                    if styled_button(ui, "OK", Kind::Confirm, true, false) {
                                        action = Some(ModalAction::Close);
                                    }
                                }
                            }
                        });
                    });
            });

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            action = Some(ModalAction::Close);
        }
        action
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_messages(ctx);

        self.header(ctx);
        self.toolbar(ctx);
        self.progress_strip(ctx);

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin::same(16.0)))
            .show(ctx, |ui| {
                self.rows(ui);
                self.log_view(ui);
            });

        match self.modal_view(ctx) {
            Some(ModalAction::Close) => self.modal = None,
            Some(ModalAction::Clean(selected)) => {
                self.modal = None;
                self.start_clean(ctx, selected);
            }
            None => {}
        }
    }
}

fn attach_console() {
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(
            windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
        );
    }
}

fn startmenu_dir() -> PathBuf {
    let base = env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env::var("USERPROFILE").unwrap_or_default()));
    base.join(r"Microsoft\Windows\Start Menu\Programs")
}

fn user_env_key() -> std::io::Result<RegKey> {
    RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)
}

fn install() -> std::io::Result<()> {
    let dir = install_dir();
    fs::create_dir_all(&dir)?;
    let src = env::current_exe()?;
    let dst = dir.join("junkcleaner.exe");
    if norm(&src) != norm(&dst) {
        fs::copy(&src, &dst)?;
    }

    // Add install dir to the user PATH (registry, so it isn't truncated like setx).
    if let Ok(key) = user_env_key() {
        let old: String = key.get_value("Path").unwrap_or_default();
        let dir_s = dir.to_string_lossy().to_string();
        if !old.to_lowercase().contains(&dir_s.to_lowercase()) {
            let new = if old.is_empty() { dir_s } else { format!("{old};{dir_s}") };
            let _ = key.set_value("Path", &new);
        }
    }

    let lnk = startmenu_dir().join("Junkcleaner.lnk");
    let exe = dst.to_string_lossy();
    let ps = format!(
        "$s = (New-Object -ComObject WScript.Shell).CreateShortcut('{}');\
         $s.TargetPath = '{exe}';\
         $s.WorkingDirectory = '{}';\
         $s.IconLocation = '{exe}';\
         $s.Save()",
        lnk.display(),
        dir.display()
    );
    let _ = Command::new("powershell").args(["-NoProfile", "-Command", &ps]).output();

    println!("Junkcleaner er installeret i:");
    println!("  {}", dir.display());
    println!("Åbn en ny terminal og skriv:  junkcleaner");
    Ok(())
}

fn uninstall() {
    // The running exe may live in the install dir; Windows won't let us delete it,
    // so removal of that one file is best-effort.
    let _ = fs::remove_dir_all(install_dir());
    let _ = fs::remove_file(startmenu_dir().join("Junkcleaner.lnk"));
    println!("Junkcleaner er fjernet. PATH-linjen kan ryddes manuelt i System-indstillinger.");
}

fn main() -> eframe::Result {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|a| a == "--install") {
        attach_console();
        if let Err(e) = install() {
            eprintln!("Installation fejlede: {e}");
        }
        return Ok(());
    }
    if args.iter().any(|a| a == "--uninstall") {
        attach_console();
        uninstall();
        return Ok(());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([880.0, 660.0])
            .with_min_inner_size([760.0, 580.0]),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|cc| {
            load_fonts(&cc.egui_ctx);
            apply_style(&cc.egui_ctx);
            Ok(Box::new(App::new()))
        }),
    )
}
