# Junkcleaner

A Windows cleaner for temp files, caches and other junk. Pick the version you prefer:

| Folder | Needs | Run |
|---|---|---|
| `python/` | Python 3 | `python python/junkcleaner.py` |
| `rust/` | [Rust](https://rustup.rs) | `cd rust && cargo run --release` |
| `powershell/` | Nothing (built into Windows) | `.\powershell\junkcleaner.ps1` (add `-Clean` to delete) |

The Python and Rust versions have a GUI and can install themselves with `--install` (remove with `--uninstall`). The PowerShell version is a simple command-line scan/clean.

## What gets cleaned

- **Temporary user files** - `%TEMP%`
- **Windows temp** - `Windows\Temp` (needs admin)
- **DirectX shader cache**
- **FiveM cache** - rebuilds automatically
- **Discord cache** - close Discord first

Files in use are skipped. Windows 10 or later.
