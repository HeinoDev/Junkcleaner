#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::env;
use std::fs;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};
use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
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

fn windows_is_dark() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(
            r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            KEY_READ,
        )
        .and_then(|k| k.get_value::<u32, _>("AppsUseLightTheme"))
        .map(|v| v == 0)
        .unwrap_or(false)
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

#[derive(PartialEq, Clone, Copy)]
enum ThemeChoice {
    Auto,
    Light,
    Dark,
}

struct App {
    admin: bool,
    items: Vec<Item>,
    sizes: Vec<Option<u64>>,
    theme: ThemeChoice,
    last_theme_check: Instant,
    last_dark: bool,
    applied_dark: Option<bool>,
    working: bool,
    progress: (usize, usize),
    summary: String,
    log: String,
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
            theme: ThemeChoice::Auto,
            last_theme_check: Instant::now(),
            last_dark: windows_is_dark(),
            applied_dark: None,
            working: false,
            progress: (0, 1),
            summary: "Klik 'Scan' for at starte scanning.".into(),
            log: String::new(),
            tx,
            rx,
        }
    }

    fn log_line(&mut self, text: &str) {
        self.log.push_str(text);
        self.log.push('\n');
    }

    fn dark_now(&self) -> bool {
        match self.theme {
            ThemeChoice::Light => false,
            ThemeChoice::Dark => true,
            ThemeChoice::Auto => self.last_dark,
        }
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        if self.last_theme_check.elapsed() > Duration::from_secs(3) {
            self.last_theme_check = Instant::now();
            self.last_dark = windows_is_dark();
        }
        let dark = self.dark_now();
        if self.applied_dark == Some(dark) {
            return;
        }
        self.applied_dark = Some(dark);
        let mut v = if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        let accent = if dark {
            Color32::from_rgb(0x0d, 0x7f, 0xff)
        } else {
            Color32::from_rgb(0x0a, 0x6c, 0xff)
        };
        v.selection.bg_fill = accent;
        v.hyperlink_color = accent;
        ctx.set_visuals(v);
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

    fn confirm_clean(&mut self, ctx: &egui::Context) {
        if self.working {
            return;
        }
        let selected: Vec<usize> = (0..self.items.len())
            .filter(|&i| self.items[i].enabled)
            .collect();
        if selected.is_empty() {
            MessageDialog::new()
                .set_title(APP_NAME)
                .set_level(MessageLevel::Warning)
                .set_description("Vælg mindst én ting der skal ryddes.")
                .show();
            return;
        }
        let amount: u64 = selected.iter().filter_map(|&i| self.sizes[i]).sum();
        let answer = MessageDialog::new()
            .set_title(APP_NAME)
            .set_buttons(MessageButtons::YesNo)
            .set_description(format!(
                "Ryd de valgte ting?\n\nCirka {} bliver fjernet.\n\n\
                 Filer der er i brug lige nu bliver sprunget over.",
                fmt(amount)
            ))
            .show();
        if answer != MessageDialogResult::Yes {
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

    fn save_log(&self) {
        let name = format!("junkcleaner-{}.txt", chrono::Local::now().format("%Y-%m-%d"));
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(name)
            .add_filter("Tekstfil", &["txt"])
            .save_file()
        else {
            return;
        };
        if let Err(e) = fs::write(&path, &self.log) {
            MessageDialog::new()
                .set_title(APP_NAME)
                .set_level(MessageLevel::Error)
                .set_description(format!("Kunne ikke gemme:\n{e}"))
                .show();
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
                    self.summary = format!("Fandt {} der kan ryddes.", fmt(total));
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
                    MessageDialog::new()
                        .set_title(APP_NAME)
                        .set_level(MessageLevel::Info)
                        .set_description(text)
                        .show();
                    self.scan(ctx);
                }
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_messages(ctx);
        self.apply_theme(ctx);
        ctx.request_repaint_after(Duration::from_secs(3));

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(APP_NAME).size(26.0).strong());
                    ui.label(
                        RichText::new("Scan maskinen, se hvad der kan ryddes, og ryd det.")
                            .weak(),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                    egui::ComboBox::from_id_salt("theme")
                        .width(80.0)
                        .selected_text(match self.theme {
                            ThemeChoice::Auto => "Auto",
                            ThemeChoice::Light => "Lyst",
                            ThemeChoice::Dark => "Mørkt",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.theme, ThemeChoice::Auto, "Auto");
                            ui.selectable_value(&mut self.theme, ThemeChoice::Light, "Lyst");
                            ui.selectable_value(&mut self.theme, ThemeChoice::Dark, "Mørkt");
                        });
                    ui.label(RichText::new("Tema").weak());
                });
            });

            ui.add_space(6.0);
            if !self.admin {
                ui.colored_label(
                    Color32::from_rgb(0xff, 0xb7, 0x4d),
                    "Kører uden administrator - Windows Temp bliver nok ikke ryddet helt.",
                );
            }
            ui.add_space(6.0);
            ui.label(RichText::new(&self.summary).strong().size(15.0));
            ui.add_space(6.0);

            let (done, max) = self.progress;
            let frac = if self.working { done as f32 / max.max(1) as f32 } else { 0.0 };
            ui.add(egui::ProgressBar::new(frac).desired_height(8.0));
            ui.add_space(10.0);

            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Ting der kan ryddes").weak());
                for (i, item) in self.items.iter_mut().enumerate() {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let locked = item.admin && !self.admin;
                        if locked {
                            item.enabled = false;
                        }
                        ui.add_enabled(!locked, egui::Checkbox::new(&mut item.enabled, item.name));
                        ui.label(RichText::new(item.desc).weak());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let text = self.sizes[i].map(fmt).unwrap_or_else(|| "-".into());
                            ui.label(RichText::new(text).strong());
                        });
                    });
                }
            });

            ui.add_space(14.0);
            let w = (ui.available_width() - 8.0) / 2.0;
            ui.horizontal(|ui| {
                let scan = egui::Button::new("Scan");
                if ui.add_enabled(!self.working, scan.min_size(egui::vec2(w, 36.0))).clicked() {
                    self.scan(ctx);
                }
                let clean = egui::Button::new(RichText::new("Ryd valgte").color(Color32::WHITE))
                    .fill(Color32::from_rgb(0x0a, 0x6c, 0xff))
                    .min_size(egui::vec2(w, 36.0));
                if ui.add_enabled(!self.working, clean).clicked() {
                    self.confirm_clean(ctx);
                }
            });

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Log").weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Gem log").clicked() {
                        self.save_log();
                    }
                });
            });
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(&self.log).monospace()).selectable(true));
                });
        });
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
            .with_inner_size([820.0, 660.0])
            .with_min_inner_size([760.0, 580.0]),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|_cc| Ok(Box::new(App::new()))),
    )
}
