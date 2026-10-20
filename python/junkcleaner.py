import os
import sys
import ctypes
import shutil
import threading
import subprocess
import tkinter as tk
from datetime import datetime
from tkinter import filedialog

app_name = "Junkcleaner"

local = os.environ.get("LOCALAPPDATA", "")
roaming = os.environ.get("APPDATA", "")
temp = os.environ.get("TEMP", "")
windir = os.environ.get("SystemRoot", r"C:\Windows")

safe_roots = [os.path.normcase(os.path.abspath(p))
              for p in (local, roaming, temp, os.path.join(windir, "Temp"))
              if p]

install_dir = os.path.join(local, "Junkcleaner")


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


c_bg = "#121212"
c_header = "#1a1a1a"
c_sub = "#1e1e1e"
c_chip = "#252525"
c_hover = "#2c2c2c"
c_border = "#333333"
c_white = "#ffffff"
c_g300 = "#d1d5db"
c_g400 = "#9ca3af"
c_g500 = "#6b7280"
c_g600 = "#4b5563"
c_g700 = "#374151"
c_b400 = "#60a5fa"
c_b500 = "#3b82f6"
c_b600 = "#2563eb"

font_ui = "Segoe UI"
font_medium = "Segoe UI Semibold"


def default_items():
    return {
        "Midlertidige brugerfiler": {
            "enabled": True,
            "paths": [temp],
            "desc": "%TEMP% - din temp-mappe",
            "admin": False,
        },
        "Midlertidige Windows-filer": {
            "enabled": True,
            "paths": [os.path.join(windir, "Temp")],
            "desc": "kræver du kører som administrator",
            "admin": True,
        },
        "DirectX shader-cache": {
            "enabled": True,
            "paths": [os.path.join(local, "D3DSCache")],
            "desc": "bygges op igen af sig selv",
            "admin": False,
        },
        "FiveM-cache": {
            "enabled": True,
            "paths": [
                os.path.join(local, r"FiveM\FiveM.app\data\cache"),
                os.path.join(local, r"FiveM\FiveM.app\data\server-cache"),
                os.path.join(local, r"FiveM\FiveM.app\data\server-cache-priv"),
            ],
            "desc": "rører kun din cache",
            "admin": False,
        },
        "Discord-cache": {
            "enabled": False,
            "paths": [
                os.path.join(roaming, r"discord\Cache"),
                os.path.join(roaming, r"discord\Code Cache"),
                os.path.join(roaming, r"discord\GPUCache"),
            ],
            "desc": "luk Discord først for bedste resultat",
            "admin": False,
        },
    }


def dark_titlebar(win):
    try:
        win.update_idletasks()
        hwnd = ctypes.windll.user32.GetParent(win.winfo_id())
        value = ctypes.c_int(1)
        for attr in (20, 19):
            ctypes.windll.dwmapi.DwmSetWindowAttribute(hwnd, attr, ctypes.byref(value), 4)
    except Exception:
        pass


def bind_hover(widget, on_enter, on_leave):
    def leave(e):
        try:
            w = widget.winfo_containing(e.x_root, e.y_root)
        except (KeyError, tk.TclError):
            w = None
        while w is not None:
            if w is widget:
                return
            w = w.master
        on_leave()

    widget.bind("<Enter>", lambda e: on_enter(), add="+")
    widget.bind("<Leave>", leave, add="+")


def walk(widget):
    yield widget
    for child in widget.winfo_children():
        yield from walk(child)


class Button(tk.Label):
    kinds = {
        "primary": (c_b500, c_b400, c_white),
        "secondary": (c_chip, c_hover, c_g300),
        "cancel": (c_g600, c_g500, c_white),
        "confirm": (c_b600, c_b500, c_white),
    }

    def __init__(self, parent, text, command, kind="secondary", small=False):
        self.normal, self.over, self.fg = self.kinds[kind]
        self.command = command
        self.enabled = True
        super().__init__(parent, text=text, bg=self.normal, fg=self.fg, bd=0,
                         font=(font_medium, 9 if small else 10), cursor="hand2",
                         padx=10 if small else 16, pady=4 if small else 7,
                         highlightthickness=1, highlightbackground=c_border if kind == "secondary" else self.normal,
                         highlightcolor=c_border)
        self.border = self.cget("highlightbackground")
        self.bind("<Enter>", lambda e: self.enabled and self.configure(bg=self.over))
        self.bind("<Leave>", lambda e: self.enabled and self.configure(bg=self.normal))
        self.bind("<Button-1>", self.click)

    def click(self, _e):
        if self.enabled:
            self.command()

    def set_enabled(self, value):
        self.enabled = value
        if value:
            self.configure(bg=self.normal, fg=self.fg, cursor="hand2", highlightbackground=self.border)
        else:
            self.configure(bg=c_chip, fg=c_g500, cursor="arrow", highlightbackground=c_border)


class Chip(tk.Label):
    def __init__(self, parent, text="", fg=c_b400):
        super().__init__(parent, text=text, bg=c_chip, fg=fg, font=(font_ui, 9), padx=8, pady=1,
                         highlightthickness=1, highlightbackground=c_border, bd=0)


class CheckBox(tk.Canvas):
    def __init__(self, parent, var, locked):
        super().__init__(parent, width=20, height=20, bg=c_sub, bd=0, highlightthickness=0)
        self.var = var
        self.locked = locked
        var.trace_add("write", lambda *_: self.draw())
        self.draw()

    def draw(self):
        self.delete("all")
        on = self.var.get()
        if on:
            fill = c_g600 if self.locked else c_b400
            self.create_rectangle(0, 0, 19, 19, fill=fill, outline=fill)
            self.create_line(5, 10, 9, 14, 15, 6, fill=c_white, width=2)
        else:
            self.create_rectangle(0, 0, 19, 19, fill=c_sub, outline=c_border, width=1)


class Row(tk.Frame):
    def __init__(self, parent, name, data, locked, on_toggle):
        super().__init__(parent, bg=c_sub, highlightthickness=1, highlightbackground=c_border,
                         highlightcolor=c_border)
        self.locked = locked
        self.var = data["enabled"]
        self.on_toggle = on_toggle

        inner = tk.Frame(self, bg=c_sub)
        inner.pack(fill="x", padx=12, pady=8)

        CheckBox(inner, self.var, locked).pack(side="left", padx=(0, 12))

        text = tk.Frame(inner, bg=c_sub)
        text.pack(side="left")
        top = tk.Frame(text, bg=c_sub)
        top.pack(anchor="w")
        tk.Label(top, text=name, bg=c_sub, fg=c_g500 if locked else c_white,
                 font=(font_medium, 10)).pack(side="left")
        if data["admin"]:
            tk.Label(top, text="admin", bg=c_g700, fg=c_g300, font=(font_ui, 8),
                     padx=6, pady=0).pack(side="left", padx=(8, 0))
        tk.Label(text, text=data["desc"], bg=c_sub, fg=c_g400, font=(font_ui, 9)).pack(anchor="w")

        self.size = tk.Label(inner, text="-", bg=c_sub, fg=c_white, font=(font_medium, 10))
        self.size.pack(side="right")

        for w in walk(self):
            w.bind("<Button-1>", self.toggle, add="+")
            if not locked:
                w.configure(cursor="hand2")
        if not locked:
            bind_hover(self, lambda: self.configure(highlightbackground=c_b500),
                       lambda: self.configure(highlightbackground=c_border))

    def toggle(self, _e):
        if self.locked:
            return
        self.var.set(not self.var.get())
        self.on_toggle()

    def set_size(self, text):
        self.size.configure(text=text)


class Dialog(tk.Toplevel):
    def __init__(self, parent, text, buttons, title=app_name):
        super().__init__(parent, bg=c_bg, highlightthickness=1, highlightbackground=c_border)
        self.result = None
        self.title(title)
        self.resizable(False, False)
        self.transient(parent)

        header = tk.Frame(self, bg=c_header)
        header.pack(fill="x")
        tk.Label(header, text=title, bg=c_header, fg=c_white, font=(font_medium, 11)).pack(
            anchor="w", padx=16, pady=10)
        tk.Frame(self, bg=c_border, height=1).pack(fill="x")

        tk.Label(self, text=text, bg=c_bg, fg=c_g300, font=(font_ui, 10), justify="left",
                 wraplength=392).pack(anchor="w", padx=24, pady=(24, 20))

        footer = tk.Frame(self, bg=c_bg)
        footer.pack(fill="x", padx=24, pady=(0, 24))
        for label, value, kind in reversed(buttons):
            Button(footer, label, lambda v=value: self.close(v), kind).pack(side="right", padx=(12, 0))

        cancel = buttons[0][1]
        self.bind("<Escape>", lambda e: self.close(cancel))
        self.protocol("WM_DELETE_WINDOW", lambda: self.close(cancel))

        self.geometry("440x1")
        self.update_idletasks()
        h = self.winfo_reqheight()
        x = parent.winfo_rootx() + (parent.winfo_width() - 440) // 2
        y = parent.winfo_rooty() + (parent.winfo_height() - h) // 2
        self.geometry(f"440x{h}+{max(0, x)}+{max(0, y)}")
        dark_titlebar(self)
        self.wait_visibility()
        self.grab_set()
        self.focus_set()

    def close(self, value):
        self.result = value
        self.grab_release()
        self.destroy()


def ask(parent, text, buttons):
    dlg = Dialog(parent, text, buttons)
    parent.wait_window(dlg)
    return dlg.result


class App(tk.Tk):
    def __init__(self):
        super().__init__()
        self.title(app_name)
        self.geometry("880x660")
        self.minsize(760, 580)
        self.configure(bg=c_bg)

        self.admin = is_admin()
        self.working = False
        self.sizes = {}
        self.progress = 0.0

        self.items = default_items()
        for data in self.items.values():
            data["enabled"] = tk.BooleanVar(value=data["enabled"])

        self.build()
        dark_titlebar(self)

    def build(self):
        header = tk.Frame(self, bg=c_header)
        header.pack(fill="x")
        head_in = tk.Frame(header, bg=c_header)
        head_in.pack(fill="x", padx=16, pady=12)
        tk.Label(head_in, text="Junkcleaner", bg=c_header, fg=c_white,
                 font=(font_medium, 13)).pack(side="left")
        self.chip_selected = Chip(head_in)
        self.chip_selected.pack(side="right")
        self.chip_admin = Chip(head_in, "Administrator" if self.admin else "Uden administrator",
                               fg=c_b400 if self.admin else c_g400)
        self.chip_admin.pack(side="right", padx=(0, 8))
        tk.Frame(self, bg=c_border, height=1).pack(fill="x")

        toolbar = tk.Frame(self, bg=c_sub)
        toolbar.pack(fill="x")
        tool_in = tk.Frame(toolbar, bg=c_sub)
        tool_in.pack(fill="x", padx=16, pady=8)
        tk.Label(tool_in, text="Fundet:", bg=c_sub, fg=c_g400, font=(font_ui, 10)).pack(side="left")
        self.found = tk.Label(tool_in, text="-", bg=c_sub, fg=c_b400, font=(font_medium, 10))
        self.found.pack(side="left", padx=(4, 16))
        self.summary = tk.Label(tool_in, text="Klik 'Scan' for at starte scanning.", bg=c_sub,
                                fg=c_g500, font=(font_ui, 9))
        self.summary.pack(side="left")

        self.clean_btn = Button(tool_in, "Ryd valgte", self.confirm_clean, "primary")
        self.clean_btn.pack(side="right")
        self.scan_btn = Button(tool_in, "Scan", self.scan)
        self.scan_btn.pack(side="right", padx=(0, 8))
        tk.Frame(self, bg=c_border, height=1).pack(fill="x")

        self.bar = tk.Canvas(self, height=2, bg=c_bg, bd=0, highlightthickness=0)
        self.bar.pack(fill="x")
        self.bar.bind("<Configure>", lambda e: self.draw_bar())

        body = tk.Frame(self, bg=c_bg)
        body.pack(fill="both", expand=True, padx=16, pady=16)

        self.rows = {}
        for name, data in self.items.items():
            locked = data["admin"] and not self.admin
            if locked:
                data["enabled"].set(False)
            row = Row(body, name, data, locked, self.update_selected)
            row.pack(fill="x", pady=(0, 8))
            self.rows[name] = row

        log_head = tk.Frame(body, bg=c_bg)
        log_head.pack(fill="x", pady=(8, 8))
        tk.Label(log_head, text="Log", bg=c_bg, fg=c_g400, font=(font_ui, 9)).pack(side="left")
        Button(log_head, "Gem log", self.save_log, small=True).pack(side="right")

        self.log = tk.Text(body, height=8, state="disabled", relief="flat", bd=0, wrap="word",
                           font=("Consolas", 9), bg=c_sub, fg=c_g300, insertwidth=0,
                           padx=10, pady=8, highlightthickness=1, highlightbackground=c_border,
                           highlightcolor=c_border, selectbackground=c_b500,
                           selectforeground=c_white, cursor="arrow")
        self.log.pack(fill="both", expand=True)

        self.update_selected()

    def update_selected(self):
        count = sum(1 for d in self.items.values() if d["enabled"].get())
        self.chip_selected.configure(text=f"{count} valgt")

    def draw_bar(self):
        self.bar.delete("all")
        width = self.bar.winfo_width()
        if self.progress > 0:
            self.bar.create_rectangle(0, 0, width * self.progress, 2, fill=c_b500, outline=c_b500)

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
            initialfile=f"junkcleaner-{datetime.now():%Y-%m-%d}.txt",
            filetypes=[("Tekstfil", "*.txt")],
        )
        if not path:
            return
        try:
            with open(path, "w", encoding="utf-8") as fh:
                fh.write(self.log.get("1.0", "end"))
        except OSError as e:
            ask(self, f"Kunne ikke gemme:\n{e}", [("OK", None, "primary")])

    def set_busy(self, busy, text="", maximum=1):
        self.working = busy
        self.maximum = max(1, maximum)
        self.scan_btn.set_enabled(not busy)
        self.clean_btn.set_enabled(not busy)
        self.progress = 0.0
        self.draw_bar()
        if text:
            self.summary.config(text=text)

    def set_progress(self, value):
        if self.winfo_exists():
            self.progress = value / self.maximum
            self.draw_bar()

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
            self.rows[name].set_size(fmt(amount))
        self.found.config(text=fmt(total))
        self.set_busy(False, "Scan færdig.")
        self.log_line(f"Scan færdig - {fmt(total)} fundet.")

    def confirm_clean(self):
        if self.working:
            return
        selected = [n for n, d in self.items.items() if d["enabled"].get()]
        if not selected:
            ask(self, "Vælg mindst én ting der skal ryddes.", [("OK", None, "primary")])
            return

        amount = sum(self.sizes.get(n, 0) for n in selected)
        answer = ask(
            self,
            "Ryd de valgte ting?\n\n"
            f"Cirka {fmt(amount)} bliver fjernet.\n\n"
            "Filer der er i brug lige nu bliver sprunget over.",
            [("Annuller", False, "cancel"), ("Ryd", True, "confirm")],
        )
        if not answer:
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
        ask(self, msg, [("OK", None, "primary")])
        self.scan()


def cli(args):
    admin = is_admin()
    items = default_items()
    if "--discord" in args:
        items["Discord-cache"]["enabled"] = True
    chosen = {n: d for n, d in items.items() if d["enabled"] and (admin or not d["admin"])}
    if not admin:
        print("Kører uden administrator - Windows Temp springes over.")

    sizes = {}
    for name, data in chosen.items():
        sizes[name] = sum(folder_size(p) for p in data["paths"])
        print(f"{name:<30}{fmt(sizes[name]):>12}")
    print(f"{'I alt':<30}{fmt(sum(sizes.values())):>12}")

    if "--clean" not in args:
        print("Kun scanning. Kør med --clean for at rydde (--discord tager Discord-cache med).")
        return
    if input("Ryd de valgte ting? [j/N] ").strip().lower() not in ("j", "y"):
        return

    total = 0
    locked_total = 0
    for name, data in chosen.items():
        deleted = 0
        locked = 0
        for path in data["paths"]:
            d, l = clean_folder(path, log=print)
            deleted += d
            locked += l
        total += deleted
        locked_total += locked
        line = f"[OK] {name}: {fmt(deleted)} fjernet"
        if locked:
            line += f"  ({fmt(locked)} sprunget over - i brug)"
        print(line)
    print(f"Færdig. I alt fjernet: {fmt(total)}")


def _startmenu_dir():
    base = roaming or os.path.expanduser("~")
    return os.path.join(base, r"Microsoft\Windows\Start Menu\Programs")


def install():
    src = os.path.abspath(__file__)
    os.makedirs(install_dir, exist_ok=True)
    dst = os.path.join(install_dir, "junkcleaner.py")
    if os.path.normcase(src) != os.path.normcase(dst):
        shutil.copy2(src, dst)

    shim = os.path.join(install_dir, "junkcleaner.cmd")
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
        lnk = os.path.join(_startmenu_dir(), "Junkcleaner.lnk")
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

    print("Junkcleaner er installeret i:")
    print("  " + install_dir)
    print('Åbn en ny terminal og skriv:  junkcleaner')


def uninstall():
    shutil.rmtree(install_dir, ignore_errors=True)
    try:
        os.remove(os.path.join(_startmenu_dir(), "Junkcleaner.lnk"))
    except OSError:
        pass
    print("Junkcleaner er fjernet. PATH-linjen kan ryddes manuelt i System-indstillinger.")


if __name__ == "__main__":
    if "--cli" in sys.argv:
        cli(sys.argv)
    elif "--install" in sys.argv:
        install()
    elif "--uninstall" in sys.argv:
        uninstall()
    else:
        App().mainloop()
