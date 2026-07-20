# wanda
(noun) : cosmo??

WANDA runs [WeMod](https://www.wemod.com/) — a Windows game-trainer app — on Linux through
Wine/Proton, and gets it to actually hook into your Steam games. It handles the fiddly parts:
building a compatible Wine prefix, installing a working WeMod build, and launching WeMod and the
game together so WeMod can attach to the running process.

> **Status:** early (v0.1.0). Expect rough edges.

---

## Why this is non-trivial

WeMod can only apply trainers to a game if **WeMod and the game run in the same Wine prefix**
(the same `wineserver` instance). If the game launches in its own Proton session, WeMod can't see
it. Most of WANDA exists to force that co-location, plus to work around a handful of WeMod/Wine
compatibility issues:

- **WeMod version pinning** — WeMod 12.x renders a black window under Wine, so WANDA installs a
  known-good older build (11.6.0) instead of the latest.
- **.NET check** — WeMod's trainer engine requires .NET Framework ≥ 4.7. Wine Mono reports 4.0, so
  WANDA patches `mscorlib.dll`'s version metadata to satisfy the check, and installs real
  .NET 4.8 via Proton (not winetricks, which corrupts 64-bit Proton prefixes).
- **Proton selection** — Proton Experimental works best; GE-Proton 10.x breaks WeMod's renderer
  (wow64), so WANDA ranks versions accordingly and warns on bad picks.

---

## Requirements

- Linux with Steam installed (native or Flatpak)
- A working **Proton** — Proton Experimental or GE-Proton 8/9 recommended
  (install via Steam or [ProtonUp-Qt](https://github.com/DavidoTek/ProtonUp-Qt))
- `wine` and `winetricks` available on `PATH`
- Rust toolchain (2021 edition) to build from source; Node.js 20+ for the GUI

Run `wanda doctor` at any time to check that these are in place.

---

## Install

### From source (CLI)

```sh
cargo build --release -p wanda-cli
# binary at target/release/wanda
```

### Nix

A dev shell and a CLI package are provided:

```sh
nix develop           # dev shell with Rust, Node, Tauri deps
nix build             # builds the wanda-cli package
```

`shell.nix` additionally provides an FHS environment (`wanda-fhs`) for running Wine/Proton binaries
on non-FHS distros like NixOS, configured to use the system NVIDIA drivers.

### GUI (Tauri)

```sh
cd crates/wanda-gui/frontend && npm install
cd .. && cargo tauri dev      # dev mode
cargo tauri build             # production bundle
```

---

## Quick start

```sh
wanda init          # detect Steam + Proton, build the Wine prefix, install .NET + WeMod
wanda scan          # list your Proton/Windows games
wanda launch <game> # launch a game together with WeMod
wanda doctor        # diagnose problems
```

`wanda init` can take 10–15 minutes the first time (mostly the .NET 4.8 install).

---

## CLI reference

Global flags: `-v`/`-vv`/`-vvv` (verbosity), `-q`/`--quiet`, `--config <path>`.

### `wanda init`
Detects Steam and Proton, creates the default Wine prefix, installs .NET 4.8, downloads and
installs WeMod.

| Flag | Description |
|------|-------------|
| `--steam-path <PATH>` | Override Steam auto-detection |
| `--proton <NAME>` | Force a specific Proton version |
| `--skip-wemod` | Don't install WeMod |
| `--skip-dotnet` | Don't install .NET (WeMod trainers may then fail) |
| `-f, --force` | Recreate the prefix even if it already exists |

### `wanda scan`
Lists installed games.

| Flag | Description |
|------|-------------|
| `-f, --filter <STR>` | Filter by name |
| `--all` | Include non-Proton (native) games |
| `-l, --long` | Detailed output (path, size, prefix) |

### `wanda launch <game>`
Launches a game with WeMod. `<game>` is an App ID or a name substring.

| Flag | Description |
|------|-------------|
| `--no-wemod` | Launch via Steam without WeMod |
| `--standalone` | Start WeMod in the game's own prefix; you press **Play** in WeMod's UI |
| `--delay <SECS>` | Wait after starting WeMod before starting the game (default 3) |
| `--args <STR>` | Extra arguments passed to the game |
| `--proton <NAME>` | Override the Proton version for this launch |
| `-w, --wait` | Block until the session exits |

### `wanda inject %command%`
A **Steam launch-option** wrapper. In a game's Steam properties, set:

```
wanda inject %command%
```

WANDA then intercepts Steam's launch, sets up WeMod inside the game's own Proton prefix, and starts
WeMod so it shares the game's Wine session. If WeMod is already running for that prefix (e.g. started
via `launch --standalone`), it passes the launch straight through into the existing session.

### `wanda prefix <cmd>`
Manage Wine prefixes: `list`, `create <name>`, `delete <name> [-f]`, `repair [name]`,
`validate [name]`, `info [name]`. Most default to the `default` prefix.

### `wanda wemod <cmd>`
Manage WeMod: `status`, `install`, `update`, `uninstall`, `run` (launch WeMod standalone).

### `wanda config <cmd>`
`show`, `path`, `edit`, `reset`, and `set <key> <value>`. Settable keys:

```
steam.install_path        proton.preferred_version   prefix.base_path
steam.scan_flatpak        wemod.auto_update          prefix.auto_repair
                          wemod.version
```

### `wanda doctor`
Read-only diagnostic report (Steam, Proton, prefix health, WeMod, system deps).
`--full` lists every detected Proton version; `--export <path>` writes the report to a file.

### `wanda cleanup`
Kills stale WeMod/Wine processes (`WeMod`, `wineserver`, `winedevice`, `steam.exe`, …).

---

## GUI

The Tauri desktop app wraps the same core library with a guided setup flow and four views:

- **Setup** — checks Steam/Proton and runs first-time initialization
- **Games** — searchable grid; launch with or without WeMod
- **Prefixes** — prefix health, repair, and WeMod update status
- **Settings** — Steam/Proton/WeMod config plus an in-app doctor

---

## How it works

- **Steam discovery** (`wanda-core::steam`) — parses `libraryfolders.vdf` and `appmanifest_*.acf`
  with a small built-in VDF parser to find libraries, games, and existing Proton prefixes.
- **Prefix building** (`wanda-core::prefix`) — initializes the prefix via Proton's own
  `wineboot`, patches `mscorlib.dll`, and installs .NET 4.8 through `proton runinprefix`.
- **Co-location** — WANDA symlinks the real `steamapps/common` and app manifest into the prefix's
  fake `C:\Program Files (x86)\Steam`, writes Steam registry keys, and launches the game via its
  **C: drive path** so WeMod's process-image match succeeds. It does this through one of three
  paths: `launch` (batch coordinator), `launch --standalone` (game's own prefix), or `inject`
  (Steam launch option).

Paths used:

```
~/.config/wanda/config.toml     configuration
~/.local/share/wanda/           prefixes, metadata, wanda.log
~/.cache/wanda/downloads/       WeMod / .NET installers
```

---

## Known limitation: WeMod trainers need real .NET Framework

WeMod's UI is Electron (works under Wine once GPU compositing is disabled — WANDA
passes `--disable-gpu --disable-gpu-compositing --disable-direct-composition`), but its
**trainer engine** (`WeModAuxiliaryService.exe`) is a **.NET Framework** app that requires
a real Microsoft .NET runtime (≥ 4.7.2556). Wine Mono is **not** sufficient — the auxiliary
process exits with code `0xC8` and WeMod shows "failed to load".

WANDA patches `mscorlib.dll` so WeMod's *version check* passes, but that only masks the
missing runtime. You need **real .NET 4.8** (or 4.7.2) actually installed in the prefix, and
that install is fragile on modern Wine. Observed failure modes:

- **Proton Experimental (wine-11, wow64):** the .NET installer's MSI cabinet extraction fails
  (`err:msi:extract_cabinet FDICopy failed` on `netfx_core.mzz`) — real .NET won't install.
- **GE-Proton 9 (wine-9):** `winetricks dotnet48/dotnet472` deadlocks on the Wine-Mono
  *uninstall* step (`msiexec /x{…}` blocked in `anon_pipe_read`); even after killing the stuck
  uninstall, the dependency chain wedges without installing .NET.

So on some machines **WeMod's UI launches and the game runs, but trainers can't activate**
until real .NET is present. If your Wine stack does install .NET 4.8/4.7.2 cleanly (many do),
WANDA works end to end. For the .NET install itself, `winetricks dotnet48` against the WANDA
prefix, or the battle-tested [`wemod-launcher`](https://github.com/DaniAsh551/wemod-launcher)
project's .NET handling, are the paths most likely to succeed.

The `.NET 4.8 install` step in `wanda init` currently uses `proton runinprefix`, which often
silently no-ops; treat "installed successfully" with suspicion and verify (a real install makes
`mscorlib.dll` ~5 MB and writes `HKLM\...\NDP\v4\Full\Release`).

---

## Project layout

```
crates/
  wanda-core/   library: config, error, steam, prefix, wemod, launcher
  wanda-cli/    the `wanda` binary (clap subcommands)
  wanda-gui/    Tauri 2 backend + React/TypeScript/Vite frontend
```

## License

GPL-3.0
