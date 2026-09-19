# Heino Cleaner

A Windows system cleaner for removing temporary files, cache, and other junk to free up disk space.

## Features

- Clean temporary files, Windows Temp, DirectX cache, FiveM cache, and Discord cache
- Dark/light theme support (auto-detects Windows theme)
- Non-admin mode supported (some features require admin)
- Real-time scan and cleaning progress
- Detailed operation logs

## Installation

Build (requires [Rust](https://rustup.rs)):

```powershell
cargo build --release
.\target\release\heino_cleaner.exe --install
```

After installation, run with:
```powershell
heino-cleaner
```

## Usage

1. Click "Scan" to see what can be cleaned
2. Select items to clean
3. Click "Ryd valgte" to confirm and clean

## System Requirements

- Windows 10 or later
- Rust (only to build from source)
- For full functionality, run as Administrator (optional)

## Uninstallation

```powershell
heino-cleaner --uninstall
```

## What Gets Cleaned

- **Temporary User Files** - %TEMP% directory
- **Windows Temporary Files** - Windows\Temp (requires admin)
- **DirectX Shader Cache** - D3D cache files
- **FiveM Cache** - Game cache (safe, rebuilds automatically)
- **Discord Cache** - Chat and media cache (optional)

## Notes

- Files in use are automatically skipped
- Most caches rebuild automatically when needed
- Close Discord before cleaning its cache
