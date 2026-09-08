import os
import sys
import ctypes
import shutil
import threading
import subprocess
import tkinter as tk
from datetime import datetime
from tkinter import ttk, messagebox, filedialog

try:
    import winreg
except ImportError:
    winreg = None

app_name = "Heino Cleaner"

local = os.environ.get("LOCALAPPDATA", "")
roaming = os.environ.get("APPDATA", "")
temp = os.environ.get("TEMP", "")
windir = os.environ.get("SystemRoot", r"C:\Windows")

safe_roots = [os.path.normcase(os.path.abspath(p))
              for p in (local, roaming, temp, os.path.join(windir, "Temp"))
              if p]

install_dir = os.path.join(local, "HeinoCleaner")


def is_admin():
    try:
        return bool(ctypes.windll.shell32.IsUserAnAdmin())
    except Exception:
        return False


def in_safe_root(path):
    if not path or not os.path.isabs(path):
        return False
    p = os.path.normcase(os.path.abspath(path))
    for root in safe_roots:
        if p == root or p.startswith(root + os.sep):
            return True
    return False


def is_reparse_point(path):
    try:
        attrs = ctypes.windll.kernel32.GetFileAttributesW(str(path))
        if attrs == -1:
            return False
        return bool(attrs & 0x400)
    except Exception:
        return False


def folder_size(path):
    total = 0
    if not path or not os.path.isdir(path):
        return 0
    for root, dirs, files in os.walk(path):
        dirs[:] = [d for d in dirs if not is_reparse_point(os.path.join(root, d))]
        for f in files:
            try:
                total += os.path.getsize(os.path.join(root, f))
            except OSError:
                pass
    return total


def clean_folder(path, log=None):
    deleted = 0
    locked = 0
    if not path or not os.path.isdir(path):
        return 0, 0
    if not in_safe_root(path):
        if log:
            log(f"  springer over (uden for de sikre mapper): {path}")
        return 0, 0

    try:
        entries = list(os.scandir(path))
    except OSError:
        return 0, 0

    for item in entries:
        try:
            if is_reparse_point(item.path):
                continue

            if item.is_file(follow_symlinks=False):
                st = item.stat(follow_symlinks=False).st_size
                try:
                    os.remove(item.path)
                    deleted += st
                except OSError:
                    locked += st

            elif item.is_dir(follow_symlinks=False):
                before = folder_size(item.path)
                shutil.rmtree(item.path, ignore_errors=True)
                after = folder_size(item.path)
                deleted += max(0, before - after)
                locked += after
        except OSError:
            pass

    return deleted, locked


def fmt(n):
    n = float(n)
    if n < 0:
        n = 0.0
    for u in ("B", "KB", "MB", "GB", "TB"):
        if n < 1024 or u == "TB":
            return f"{n:.2f} {u}"
        n /= 1024


def windows_is_dark():
    if winreg is None:
        return False
    try:
        key = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, key) as k:
            value, _ = winreg.QueryValueEx(k, "AppsUseLightTheme")
            return value == 0
    except OSError:
        return False


light_theme = {
    "bg": "#f3f3f3", "surface": "#ffffff", "field": "#ffffff",
    "text": "#1a1a1a", "muted": "#606060", "border": "#cfcfcf",
    "accent": "#0a6cff", "accent_text": "#ffffff", "warn": "#a06000",
    "accent_hover": "#0855cc", "field_hover": "#f5f5f5", "border_hover": "#b3b3b3",
    "combobox": "#ffffff",
    "log_bg": "#ffffff", "log_fg": "#1a1a1a",
}

dark_theme = {
    "bg": "#1e1e1e", "surface": "#252526", "field": "#2d2d30",
    "text": "#e4e4e4", "muted": "#9a9a9a", "border": "#3c3c3c",
    "accent": "#0d7fff", "accent_text": "#ffffff", "warn": "#ffb74d",
    "accent_hover": "#1a8fff", "field_hover": "#353539", "border_hover": "#4a4a4e",
    "combobox": "#3c3c41",
    "log_bg": "#181818", "log_fg": "#d6d6d6",
}


class App(tk.Tk):
    def __init__(self):
        super().__init__()
        self.title(app_name)
        self.geometry("820x660")
        self.minsize(760, 580)

        self.admin = is_admin()
        self.working = False
        self.sizes = {}
        self.theme_choice = tk.StringVar(value="Auto")
        self._last_dark = windows_is_dark()

        self.items = {
            "Midlertidige brugerfiler": {
                "enabled": tk.BooleanVar(value=True),
                "paths": [temp],
                "desc": "%TEMP% - din temp-mappe",
                "admin": False,
            },
            "Midlertidige Windows-filer": {
                "enabled": tk.BooleanVar(value=True),
                "paths": [os.path.join(windir, "Temp")],
                "desc": "kræver du kører som administrator",
                "admin": True,
            },
            "DirectX shader-cache": {
                "enabled": tk.BooleanVar(value=True),
                "paths": [os.path.join(local, "D3DSCache")],
                "desc": "bygges op igen af sig selv",
                "admin": False,
            },
            "FiveM-cache": {
                "enabled": tk.BooleanVar(value=True),
                "paths": [
                    os.path.join(local, r"FiveM\FiveM.app\data\cache"),
                    os.path.join(local, r"FiveM\FiveM.app\data\server-cache"),
                    os.path.join(local, r"FiveM\FiveM.app\data\server-cache-priv"),
                ],
                "desc": "rører kun din cache",
                "admin": False,
            },
            "Discord-cache": {
                "enabled": tk.BooleanVar(value=False),
                "paths": [
                    os.path.join(roaming, r"discord\Cache"),
                    os.path.join(roaming, r"discord\Code Cache"),
                    os.path.join(roaming, r"discord\GPUCache"),
                ],
                "desc": "luk Discord først for bedste resultat",
                "admin": False,
            },
        }

        self.build()
        self.apply_theme()
        self._theme_tick()

    def build(self):
        self.style = ttk.Style(self)
        try:
            self.style.theme_use("clam")
        except tk.TclError:
            pass

        outer = ttk.Frame(self, padding=20, style="Bg.TFrame")
        outer.pack(fill="both", expand=True)

        header = ttk.Frame(outer, style="Bg.TFrame")
        header.pack(fill="x")

        title = ttk.Frame(header, style="Bg.TFrame")
        title.pack(side="left", fill="x", expand=True)
        ttk.Label(title, text="Heino Cleaner", style="H1.TLabel").pack(anchor="w")
        ttk.Label(title, text="Scan maskinen, se hvad der kan ryddes, og ryd det.",
                  style="Sub.TLabel").pack(anchor="w", pady=(0, 6))

        theme_box = ttk.Frame(header, style="Bg.TFrame")
        theme_box.pack(side="right", anchor="ne")
        ttk.Label(theme_box, text="Tema", style="Sub.TLabel").pack(side="left", padx=(0, 6))
        picker = ttk.Combobox(theme_box, textvariable=self.theme_choice, width=8,
                              state="readonly", values=["Auto", "Lyst", "Mørkt"])
        picker.pack(side="left")
        picker.bind("<<ComboboxSelected>>", lambda _e: self.apply_theme())
        picker.bind("<Enter>", lambda e: picker.config(cursor="hand2"))
        picker.bind("<Leave>", lambda e: picker.config(cursor=""))

        if not self.admin:
            ttk.Label(outer,
                      text="Kører uden administrator - Windows Temp bliver nok ikke ryddet helt.",
                      style="Warn.TLabel").pack(anchor="w", pady=(4, 10))
        else:
            ttk.Frame(outer, height=8, style="Bg.TFrame").pack()

        self.summary = ttk.Label(outer, text="Klik 'Scan' for at starte scanning.", style="Summary.TLabel")
        self.summary.pack(anchor="w", pady=(0, 10))

        self.bar = ttk.Progressbar(outer, mode="determinate", maximum=1, value=0)
        self.bar.pack(fill="x", pady=(0, 12))

        frame = ttk.LabelFrame(outer, text="Ting der kan ryddes", padding=10,
                               style="Card.TLabelframe")
        frame.pack(fill="x")

        self.size_labels = {}
        self.row_frames = {}
        for name, data in self.items.items():
            row = ttk.Frame(frame, style="Card.TFrame")
            row.pack(fill="x", pady=6)
            self.row_frames[name] = row

            cb = ttk.Checkbutton(row, text=name, variable=data["enabled"],
                                 style="Card.TCheckbutton")
            cb.pack(side="left", padx=(0, 8))
            if data["admin"] and not self.admin:
                cb.state(["disabled"])
                data["enabled"].set(False)

            ttk.Label(row, text=data["desc"], style="Muted.TLabel").pack(side="left", padx=12)

            label = ttk.Label(row, text="—", style="Card.TLabel")
            label.pack(side="right", padx=(8, 0))
            self.size_labels[name] = label

            self._add_hover_effect(row)

        buttons = ttk.Frame(outer, style="Bg.TFrame")
        buttons.pack(fill="x", pady=20)

        self.scan_btn = ttk.Button(buttons, text="Scan", command=self.scan)
        self.scan_btn.pack(side="left", fill="x", expand=True, padx=(0, 8))
        self.scan_btn.bind("<Enter>", lambda e: self.scan_btn.config(cursor="hand2"))
        self.scan_btn.bind("<Leave>", lambda e: self.scan_btn.config(cursor=""))

        self.clean_btn = ttk.Button(buttons, text="Ryd valgte", command=self.confirm_clean,
                                    style="Accent.TButton")
        self.clean_btn.pack(side="left", fill="x", expand=True, padx=(8, 0))
        self.clean_btn.bind("<Enter>", lambda e: self.clean_btn.config(cursor="hand2"))
        self.clean_btn.bind("<Leave>", lambda e: self.clean_btn.config(cursor=""))

        topline = ttk.Frame(outer, style="Bg.TFrame")
        topline.pack(fill="x", pady=(12, 8))
        ttk.Label(topline, text="Log", style="Sub.TLabel").pack(side="left")
        save_log_btn = ttk.Button(topline, text="Gem log", command=self.save_log)
        save_log_btn.pack(side="right")
        save_log_btn.bind("<Enter>", lambda e: save_log_btn.config(cursor="hand2"))
        save_log_btn.bind("<Leave>", lambda e: save_log_btn.config(cursor=""))

        self.log = tk.Text(outer, height=13, state="disabled", relief="flat",
                           font=("Consolas", 9), borderwidth=8, padx=8, pady=8)
        self.log.pack(fill="both", expand=True, pady=(5, 0))
        self.log.bind("<Enter>", lambda e: self.log.config(cursor="arrow"))
        self.log.bind("<Leave>", lambda e: self.log.config(cursor=""))

    def _dark_now(self):
        choice = self.theme_choice.get()
        if choice == "Lyst":
            return False
        if choice == "Mørkt":
            return True
        return windows_is_dark()

    def apply_theme(self):
        dark = self._dark_now()
        self._last_dark = windows_is_dark()
        c = dark_theme if dark else light_theme
        s = self.style

        self.configure(bg=c["bg"])
        s.configure(".", background=c["bg"], foreground=c["text"],
                    fieldbackground=c["field"], bordercolor=c["border"])

        s.configure("Bg.TFrame", background=c["bg"])
        s.configure("Card.TFrame", background=c["surface"])
        s.configure("CardHover.TFrame", background=c["field_hover"])
        s.configure("Card.TLabelframe", background=c["surface"], bordercolor=c["border"])
        s.configure("Card.TLabelframe.Label", background=c["surface"], foreground=c["muted"])

        s.configure("H1.TLabel", background=c["bg"], foreground=c["text"],
                    font=("Segoe UI", 22, "bold"))
        s.configure("Sub.TLabel", background=c["bg"], foreground=c["muted"],
                    font=("Segoe UI", 10))
        s.configure("Summary.TLabel", background=c["bg"], foreground=c["text"],
                    font=("Segoe UI", 11, "bold"))
        s.configure("Warn.TLabel", background=c["bg"], foreground=c["warn"],
                    font=("Segoe UI", 9))
        s.configure("Muted.TLabel", background=c["surface"], foreground=c["muted"])
        s.configure("Card.TLabel", background=c["surface"], foreground=c["text"], font=("Segoe UI", 10, "bold"))
        s.configure("Card.TCheckbutton", background=c["surface"], foreground=c["text"], padding=4)
        s.map("Card.TCheckbutton", background=[("active", c["field_hover"]), ("disabled", c["surface"])])

        s.configure("TButton", background=c["field"], foreground=c["text"],
                    bordercolor=c["border"], focuscolor=c["accent"], padding=10, relief="raised")
        s.map("TButton", background=[("active", c["field_hover"]), ("pressed", c["border"]), ("disabled", c["bg"])],
              foreground=[("disabled", c["muted"])], relief=[("pressed", "sunken"), ("active", "raised")])
        s.configure("Accent.TButton", background=c["accent"], foreground=c["accent_text"], padding=10, relief="raised")
        s.map("Accent.TButton", background=[("active", c["accent_hover"]), ("pressed", c["accent"]), ("disabled", c["border"])],
              relief=[("pressed", "sunken"), ("active", "raised")])

        s.configure("TProgressbar", background=c["accent"], troughcolor=c["field"],
                    bordercolor=c["border"], thickness=8, lightcolor=c["accent"], darkcolor=c["accent"])
        s.configure("TCombobox", fieldbackground=c["combobox"], background=c["combobox"],
                    foreground=c["text"], arrowcolor=c["text"], bordercolor=c["border"],
                    relief="solid", padding=4, selectbackground=c["accent"],
                    selectforeground=c["accent_text"])
        s.map("TCombobox", fieldbackground=[("readonly", c["combobox"]), ("focus", c["field_hover"])],
              background=[("readonly", c["combobox"]), ("focus", c["field_hover"])])

        self.log.configure(bg=c["log_bg"], fg=c["log_fg"], insertbackground=c["log_fg"])

    def _add_hover_effect(self, widget):
        def on_enter(e):
            widget.configure(style="CardHover.TFrame")

        def on_leave(e):
            widget.configure(style="Card.TFrame")

        widget.bind("<Enter>", on_enter)
        widget.bind("<Leave>", on_leave)

    def _theme_tick(self):
        if not self.winfo_exists():
            return
        if self.theme_choice.get() == "Auto" and windows_is_dark() != self._last_dark:
            self.apply_theme()
        self.after(3000, self._theme_tick)

    def log_line(self, text):
        if not self.winfo_exists():
            return
        self.log.configure(state="normal")
        self.log.insert("end", text + "\n")
        self.log.see("end")
        self.log.configure(state="disabled")

    def save_log(self):
        path = filedialog.asksaveasfilename(
            defaultextension=".txt",
            initialfile=f"heino-cleaner-{datetime.now():%Y-%m-%d}.txt",
            filetypes=[("Tekstfil", "*.txt")],
        )
        if not path:
            return
        try:
            with open(path, "w", encoding="utf-8") as fh:
                fh.write(self.log.get("1.0", "end"))
        except OSError as e:
            messagebox.showerror(app_name, f"Kunne ikke gemme:\n{e}")

    def set_busy(self, busy, text="", maximum=1):
        self.working = busy
        state = "disabled" if busy else "normal"
        self.scan_btn.config(state=state)
        self.clean_btn.config(state=state)

        if busy:
            self.bar.config(mode="determinate", maximum=max(1, maximum), value=0)
        else:
            self.bar.config(mode="determinate", maximum=1, value=0)

        if text:
            self.summary.config(text=text)

    def set_progress(self, value):
        if self.winfo_exists():
            self.bar.config(value=value)

    def scan(self):
        if self.working:
            return
        self.set_busy(True, "Scanner...", maximum=len(self.items))
        self.log_line("Scanner...")
        threading.Thread(target=self._scan, daemon=True).start()

    def _scan(self):
        sizes = {}
        for i, (name, data) in enumerate(self.items.items(), start=1):
            sizes[name] = sum(folder_size(p) for p in data["paths"])
            if self.winfo_exists():
                self.after(0, self.set_progress, i)
        if self.winfo_exists():
            self.after(0, self.finish_scan, sizes)

    def finish_scan(self, sizes):
        if not self.winfo_exists():
            return
        self.sizes = sizes
        total = sum(sizes.values())
        for name, amount in sizes.items():
            self.size_labels[name].config(text=fmt(amount))
        self.set_busy(False, f"Fandt {fmt(total)} der kan ryddes.")
        self.log_line(f"Scan færdig - {fmt(total)} fundet.")

    def confirm_clean(self):
        if self.working:
            return
        selected = [n for n, d in self.items.items() if d["enabled"].get()]
        if not selected:
            messagebox.showwarning(app_name, "Vælg mindst én ting der skal ryddes.")
            return

        amount = sum(self.sizes.get(n, 0) for n in selected)
        if not messagebox.askyesno(
            app_name,
            "Ryd de valgte ting?\n\n"
            f"Cirka {fmt(amount)} bliver fjernet.\n\n"
            "Filer der er i brug lige nu bliver sprunget over."
        ):
            return

        self.set_busy(True, "Rydder...", maximum=len(selected))
        self.log_line("Rydder...")
        threading.Thread(target=self._clean, args=(selected,), daemon=True).start()

    def _clean(self, selected):
        total = 0
        locked_total = 0
        results = {}

        for i, name in enumerate(selected, start=1):
            deleted = 0
            locked = 0
            for path in self.items[name]["paths"]:
                d, l = clean_folder(path, log=lambda m: self.after(0, self.log_line, m))
                deleted += d
                locked += l
            results[name] = (deleted, locked)
            total += deleted
            locked_total += locked
            if self.winfo_exists():
                self.after(0, self.set_progress, i)

        if self.winfo_exists():
            self.after(0, self.finish_clean, total, locked_total, results)

    def finish_clean(self, total, locked_total, results):
        if not self.winfo_exists():
            return
        for name, (deleted, locked) in results.items():
            line = f"[OK] {name}: {fmt(deleted)} fjernet"
            if locked:
                line += f"  ({fmt(locked)} sprunget over - i brug)"
            self.log_line(line)

        self.log_line("")
        self.log_line(f"Færdig. I alt fjernet: {fmt(total)}")
        self.set_busy(False, f"Færdig - fjernede {fmt(total)}.")

        msg = f"Rydning færdig!\n\nFjernet: {fmt(total)}"
        if locked_total:
            msg += f"\nSprunget over (i brug): {fmt(locked_total)}"
        messagebox.showinfo(app_name, msg)
        self.scan()


def _startmenu_dir():
    base = roaming or os.path.expanduser("~")
    return os.path.join(base, r"Microsoft\Windows\Start Menu\Programs")


def install():
    src = os.path.abspath(__file__)
    os.makedirs(install_dir, exist_ok=True)
    dst = os.path.join(install_dir, "heino_cleaner.py")
    if os.path.normcase(src) != os.path.normcase(dst):
        shutil.copy2(src, dst)

    shim = os.path.join(install_dir, "heino-cleaner.cmd")
    with open(shim, "w", encoding="utf-8") as fh:
        fh.write(f'@echo off\r\n"{sys.executable}" "{dst}" %*\r\n')

    if install_dir.lower() not in os.environ.get("PATH", "").lower():
        try:
            old = os.environ.get("PATH", "")
            subprocess.run(["setx", "PATH", f"{old};{install_dir}"],
                           check=False, capture_output=True)
        except OSError:
            pass

    try:
        lnk = os.path.join(_startmenu_dir(), "Heino Cleaner.lnk")
        ps = (
            f"$s = (New-Object -ComObject WScript.Shell).CreateShortcut('{lnk}');"
            f"$s.TargetPath = '{sys.executable}';"
            f"$s.Arguments = '\"{dst}\"';"
            f"$s.WorkingDirectory = '{install_dir}';"
            f"$s.IconLocation = '{sys.executable}';"
            f"$s.Save()"
        )
        subprocess.run(["powershell", "-NoProfile", "-Command", ps],
                       check=False, capture_output=True)
    except OSError:
        pass

    print("Heino Cleaner er installeret i:")
    print("  " + install_dir)
    print('Åbn en ny terminal og skriv:  heino-cleaner')


def uninstall():
    shutil.rmtree(install_dir, ignore_errors=True)
    try:
        os.remove(os.path.join(_startmenu_dir(), "Heino Cleaner.lnk"))
    except OSError:
        pass
    print("Heino Cleaner er fjernet. PATH-linjen kan ryddes manuelt i System-indstillinger.")


if __name__ == "__main__":
    if "--install" in sys.argv:
        install()
    elif "--uninstall" in sys.argv:
        uninstall()
    else:
        App().mainloop()
