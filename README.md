# Junkcleaner

A Windows cleaner for temp files, caches and other junk. Pick the version you prefer (Python or Rust):

| Folder | Needs | Run |
|---|---|---|
| `python/` | Python 3 | `python python/junkcleaner.py` |
| `rust/` | [Rust](https://rustup.rs) | `cd rust && cargo run --release` |
| `python/` (command line) | Python 3 | `python python/junkcleaner.py --cli` (add `--clean` to delete, `--discord` to include Discord) |

Both versions have a GUI and can install themselves with `--install` (remove with `--uninstall`). The Python version also has a command-line mode (`--cli`).

## What gets cleaned

- **Temporary user files** - `%TEMP%`
- **Windows temp** - `Windows\Temp` (needs admin)
- **DirectX shader cache**
- **FiveM cache** - rebuilds automatically
- **Discord cache** - close Discord first

Files in use are skipped. Windows 10 or later.
