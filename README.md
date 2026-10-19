<div align="center">

<img src="usr/share/icons/hicolor/scalable/apps/com.biglinux.BiGameMode.svg" width="112" alt="Big Game Mode icon">

# Big Game Mode

**BigLinux's game mode.**<br>
One switch for per-game performance, presets for what games should favour,
a library that says how each game runs on Linux, AI Graphics with full backup
and undo, and a page that shows — with evidence — what is really in effect.

[![Version](https://img.shields.io/badge/version-3.0.0-6c5ce7?style=for-the-badge)](https://github.com/biglinux/bigamemode/releases)
[![License](https://img.shields.io/badge/license-GPL--3.0--or--later-2ea44f?style=for-the-badge)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.88%2B-dea584?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![GTK4](https://img.shields.io/badge/GTK4-libadwaita-4a86cf?style=for-the-badge&logo=gnome&logoColor=white)](https://gnome.pages.gitlab.gnome.org/libadwaita/)
[![Languages](https://img.shields.io/badge/languages-29-f39c12?style=for-the-badge)](#languages)

[Features](#features) ·
[Screenshots](#screenshots) ·
[Installation](#installation) ·
[Troubleshooting](#troubleshooting) ·
[Security](#security) ·
[Development](#development)

<br>

<picture>
  <source media="(prefers-color-scheme: light)" srcset="docs/screenshots/home-light.png">
  <img src="docs/screenshots/home.png" width="860" alt="Big Game Mode's Home page: the Turbo card with its three presets and Back to normal, and the Now card">
</picture>

</div>

---

## What it is

Big Game Mode (the BiGame-mode project) is the gaming hub of BigLinux. With one
button, **Turbo**, every game gets its own performance profile, applied by
[falcond](https://git.pika-os.com/general-packages/falcond) when the game
starts and undone when it closes. The application shows what is really in
effect, measures whether a change helped, and takes care of what happens
*inside* the game — upscaling and frame generation — always with a backup and
a way back.

> [!NOTE]
> One rule runs through the project: **nothing is offered that the machine
> cannot do, and nothing is called an improvement without a measurement.**

## What's new in 3.0.0

**A new layout, one purpose per page.** Seven pages became six, in the
order a person uses them:

| Page | What it holds |
|---|---|
| **Home** | One **Turbo** card — the switch, the three presets under it with what the chosen one does, and Back to normal — and a **Now** card with the game being played: its cover, launcher, how long it has run, its ProtonDB and anti-cheat tags, and one coloured flag per feature it really got. With no game, it says what Turbo is doing and where to start one. |
| **Games** | The library (formerly Profiles), each game with its profile and its compatibility. |
| **Optimization** | Tabs: **Overview** (the machine at a glance and the recommendations for its own hardware and packages), **Tuning**, **Details**, and **Network and apps** (the connection and DNS, the programs beside the game, Steam's launch options). |
| **Telemetry** | The live charts, as before. |
| **Records** | Tabs: **Reports** (the last Turbo report and the support report), **Games that crashed**, and **Logs**. |
| **Settings** | As before, with Game Information. |

- **Recommendations instead of Problems.** Each says whether it is
  *Recommended*, needs your *Attention*, is *Not supported* on this hardware,
  *Informative*, *Already adequate*, or *Needs measurement*, and leads to the
  page that changes it. Nothing is applied from the list: there is no
  "apply all", a DNS is never changed and a program never paused or closed
  without you asking.
- **Simple and advanced share everything.** Simple mode hides the Tuning,
  Details and Logs tabs instead of whole pages; a page saved by an older
  version opens where its content went.
- **Icons that are not in every theme** ship with the application, so no
  page shows a missing image outside BigLinux's icon theme.

**Your library knows how each game runs on Linux.** Every card says, at a
glance and without leaving the library:

- **How well it runs**: ProtonDB's rating (Platinum, Gold, Silver, Bronze,
  Borked), or **Native** when Steam lists a Linux version. A game nobody has
  reported on says so; it is never called Borked.
- **Whether its anti-cheat lets you play online**: the status from the
  community list Are We Anti-Cheat Yet? (Supported, Running, Planned,
  Broken, Denied), with the anti-cheats it uses.
- **How many play it on Steam now**, and on the game's page today's peak
  from Steam's own chart, or the highest count Big Game Mode read itself,
  said to be that, until there is history enough.

**Games from Heroic, Lutris and Faugus too.** One that is a Linux build
says **Native**; a Windows one is looked for on Steam's store by its exact
title (an edition's name aside, never a sequel or a DLC), and its rating,
anti-cheat status and players are said to be those of its Steam version.

A tag opens what it means, where it comes from and when, with a link to the
source; the game's page has the whole picture. In advanced mode the library
filters and sorts by compatibility, anti-cheat status and players. What was
learnt is kept, so the library works offline and opens at once; it is asked
again only once a source says it may have changed, a few games at a time,
never while a game runs, and never from SteamDB, which forbids it. Only a
game's public Steam number is sent. Settings → Game Information turns any of
it off, says when each source last answered, and clears what was
downloaded.

Also: a game whose community status is **Denied** is said before **Play**,
and the window never leaves the focus on a notice's button (keys meant for a
game could press it).

### 2.4.1

**A game that crashes is said, with what was in it.** Cyberpunk 2077 closed
after 25 minutes on an access violation in its own scripts, and Big Game
Mode only said "Cyberpunk 2077 closed", as for a game quit from its menu.

- **A crash is told from a quit,** from what it leaves and only what was
  left while the game ran: the game's own crash report (REDengine, Unreal
  Engine, Unity), a crash reporter still open, how the kernel says the
  process ended (a crash signal, or the code Wine ends a Windows game with
  when an exception stops it), the kernel's out-of-memory killer, and
  systemd-coredump's record.
- **The notice says what was in the game** besides the game itself (vkBasalt,
  FSR 4, Wine FSR, MangoHud, lsfg-vk, OptiScaler, Gamescope), read from the
  game while it ran, never from settings, with the game's reason and how
  many times it crashed this week.
- **Leave them out of this game:** one button keeps vkBasalt, FSR 4 and
  Wine FSR out of that game alone from its next start, written where its
  launcher reads it (an open Steam is closed and opened again around the
  write, never while it runs a game). Nothing changes until it is chosen;
  Games puts them back.
- **FSR 4 can be switched off for one game**, whatever the Pretty preset
  asks for the others.
- Details lists the last crashes, the support report carries them, and
  falcond's VRAM protection error is said in your language.

### 2.4.0

**First, do no harm.** A full review asked one question of every feature:
can it make a game slower without saying so? Where it could, it no longer
does.

- **Three presets, honest names.** **Fastest** adds nothing, **Quiet** caps
  the frame rate at a rate that fits the screen (the refresh rate up to
  75 Hz, half of it above: 80 on a 160 Hz screen, so every frame stays up
  for exactly two refreshes), and **Pretty** sharpens with no cap. Before,
  *Locked 60 FPS* and *Enhanced graphics* capped every game at 60, which on
  a 144–165 Hz screen without adaptive sync judders. A game that generates
  frames is never capped (under 60 it rendered 30). Saved presets load as
  the one that took their place.
- **A game's frame limit without Gamescope.** It used to start nested
  Gamescope for `-r`, which did not limit and made frame pacing worse; now
  DXVK and VKD3D-Proton hold it (MangoHud in native games).
- **Profiles keep falcond's protections:** a game's own profile no longer
  drops the VRAM protection and split-lock setting falcond's Proton profile
  gives every Proton game. No CPU scheduler is pressed on anyone: none was
  measured faster, and one set for the whole desktop is pointed out.
- **Out of the way while you play.** Nothing is read behind a game (a
  Details page left open used to read the game's memory map and the GPU's
  sensors every few seconds), the game's exit is an event instead of a
  process scan, the library is not rescanned during the game, in-game
  notifications wait in the notification list, and Big Game Mode's disk
  reads yield to the game's.
- **AI Graphics' Recommended installs only what was verified;** a game
  that ships AMD's FidelityFX API keeps its own FSR (FSR 4 through Proton)
  on RDNA 4, and OptiScaler's frame generation is an experimental choice,
  never planned on a GeForce GTX.
- **Simple mode, the default:** Turbo says ON or OFF in words, three
  choices, **Play** on every game and **Back to normal** to undo it all.
  Settings → Advanced brings Tuning, Details and Logs back.
- **Telemetry has its own page**, in the sidebar in simple mode too: the
  processor, memory, disk, network and one card per GPU, each with a chart
  of the last minute. The cards keep their size and place whatever the
  readings say, and the whole page fits the window.
- **One look on every page:** Home, Profiles, Tuning, Details, Logs and
  Settings use Telemetry's cards, with an icon and a title in each card's
  header, laid out in columns on a wide window and one under the other on a
  narrow one. Turbo's button is as it was.
- **What no setting fixes, said plainly:** a game library on a hard disk,
  adaptive sync switched off, and Steam launch options with a wrapper but no
  `%command%` (with a one-click fix).
- **No scheduler left behind:** a sched-ext scheduler nobody asks for (left
  by falcond, which put it back after every game) is unloaded as Turbo
  switches, and saving Tuning no longer restarts falcond under a running
  game.
- **AI Graphics, safer to leave:** every game's graphics restored in one go
  (Settings → Advanced, or `bigame-ui --restore-all-graphics` before removing
  the package), and a game that got anti-cheat in an update while OptiScaler
  is in it is named.
- **Lighter in the tray:** about 117 MiB after the window closes, from about
  480 MiB.

### 2.3.2

A stability release, from a full review of the code, the helper and the
package.

- **Turbo on processors falcond cannot run on** (Sandy Bridge, Ivy Bridge and
  older with a falcond built for x86-64-v3) no longer switches itself off a
  few seconds after it starts, while the power profile stayed at performance.
  See [CPU compatibility](docs/CPU-COMPATIBILITY.md).
- **Nothing left behind:** turning Turbo off or changing the preset reaches
  Steam and Heroic once they are closed, and deleting a profile takes its
  launch options out of Steam, Heroic and Lutris. Profiles says which launcher
  to close when one was open.
- **Your settings stay yours:** only what Big Game Mode added is removed,
  account by account (a native and a Flatpak Steam are two installs); your
  own launch options, Lutris' MangoHud line, and MangoHud and vkBasalt files
  (also ones that are not UTF-8, and dotfile-manager symlinks) are kept.
- **AI Graphics** makes one change at a time to a game, so a double click can
  no longer mix two installs' backups; Restore from Profiles works as on the
  AI Graphics page.
- **Idle cost:** one systemd reading for the tray and Home, no property cache
  traffic, slower polling while a game runs, and nothing woken while the
  window is hidden.
- **Game detection:** Electron/Chromium helpers, the Steam runtime's locale
  tools, Windows installers (`msiexec`, .NET) and crash reporters are not
  taken for the game.
- **A stricter helper:** `/run` and the machine-wide `/sys` trees read-only,
  memory and task limits, no systemd reload when nothing changes, profiles
  named after system processes refused. Its Polkit messages are now
  translated with the rest of the application.
- **Quality:** Rust 1.88 declared (the real minimum), French, Chinese and
  other languages get their own "label: value" punctuation, and the About
  dialog names the developer.

### 2.3.1

- Processors without AVX2/BMI2: a falcond built for x86-64-v3 is named as
  the cause instead of a retry being suggested, and Turbo comes on with the
  general settings
  ([#4](https://github.com/biglinux/bigamemode/issues/4)).
- A falcond that crashes at start no longer stays enabled; the Steam
  runtime's `ldconfig` is not taken for the game.

### 2.3.0

- **Big Game Mode** is the name you see everywhere; package names, the
  application id and file locations are unchanged.
- **The tray is a quick control:** Turbo on or off, the four presets, and its
  own symbolic icon drawn in the panel's colours.
- **Launchers opened before a preset** (Steam, Heroic, Lutris) are offered a
  reopen from a notice at the top of any page, never while one of their games
  runs.
- **Profiles:** the wizard is the main way to create one, and a profile can be
  started from its editor with **Launch (Turbo)**. Home names the CPU and every
  GPU, the gaming one first on hybrid laptops.
- **lsfg-vk 2.x:** its configuration format is read from the installed
  library and written in that format; a `Lossless.dll` lsfg-vk cannot use is
  reported instead of letting the game fail.
- **Hybrid NVIDIA laptops:** Gamescope no longer crashes (the render-offload
  variables reach only the game), and nested Gamescope shows the game the
  screen's size instead of 1280×720.
- **Turbo** rolls back cleanly when falcond fails to start, and a preset leaves
  nothing behind once Turbo is off. Restore Defaults now resets everything
  Tuning shows.
- **Game detection** reads Steam, Heroic and Lutris libraries directly and no
  longer takes launchers, trials, bootstrappers or scripts for the game.
- **AI Graphics:** frame generation is shown active only when OptiScaler's log
  proves it; Restore and Repair are safer; the download cache keeps only the
  releases games use; a game's built-in FSR under VKD3D-Proton stays its own.
- **A stricter privileged helper**, and about 0.15 % of one CPU core while idle
  in the tray (2 % before).
- **No extra repository needed:** falcond, lsfg-vk, sched-ext, Gamescope,
  MangoHud and vkBasalt are optional. What needs one says so and how to
  install it.

<details>
<summary><b>2.2.0 and 2.1.0</b></summary>

- **2.2.0:** a welcome screen, a rewritten About and Help; Tuning with
  Gamescope always in view and vkBasalt looks (CAS or the Nara Linux style);
  game profiles with the same rows as Tuning, reaching Steam and Heroic games;
  Launch (Turbo) through every launcher; a redesigned AI Graphics window;
  background programs can be closed or paused, a measured DNS applied with a
  backup, and falcond handed back and taken again.
- **2.1.0:** Turbo presets chosen before switching it on; a redrawn Home;
  Tuning, game profiles and the profile wizard on one model with every
  conflict asked; Gamescope and Wine FSR reach Steam games through their launch
  options; Details tells when falcond does not apply a game's profile, and why.

</details>

## Features

### Turbo and presets

Turbo is the master switch. Off, Big Game Mode leaves every game alone. On,
falcond applies each game's profile — power profile, sched-ext CPU scheduler,
3D V-Cache mode, idle inhibition — and restores everything when the game
closes.

Before switching it on, choose what games should favour:

| Preset | What it does |
|---|---|
| **Fastest** | Nothing added: Tuning as it is, no frame cap |
| **Quiet** | A frame cap that fits the screen — the refresh rate up to 75 Hz, half of it above (80 on 160 Hz) — through DXVK and VKD3D-Proton in Proton games and MangoHud in native games Big Game Mode starts; games that generate frames are left uncapped |
| **Pretty** | vkBasalt CAS sharpening (from Big Game Mode's own CAS-only file) and FSR 4 where the GPU has it; no cap |

A preset lives only in the running session (the user's systemd manager, never
`environment.d`) and leaves with Turbo, even when falcond stops on its own. Your
own `DXVK_CONFIG` and FSR 4 variables are kept and given back.

### Per-game profiles

Games from **Steam**, **Heroic**, **Lutris** and the application menu, native
or Flatpak — only the ones really installed. When an unknown game starts with
Turbo on, a notification offers to create its profile with a wizard that
explains every option. A profile has the same sections as Tuning, each option
starting at "General configuration". What a game needs reaches it where it can:
through **Launch (Turbo)**, Steam's launch options, or the game's settings in
Heroic — written with the launcher closed, touching only what Big Game Mode
added.

### Launch settings: Gamescope, MangoHud, vkBasalt, frame generation

**Tuning** holds the configuration every game shares:

- system performance through falcond (scheduler, V-Cache, profiles);
- **Gamescope** with sizes picked from standard resolutions and the main
  screen's;
- **Wine FSR** and **vkBasalt** (CAS or the Nara Linux look);
- **frame generation** with [lsfg-vk](https://github.com/PancakeTAS/lsfg-vk)
  (1.x and 2.x, with your own `Lossless.dll`, never shipped or downloaded);
- the **MangoHud** overlay, with Steam Deck–like styles.

Two technologies that do the same job never run in series: enabling one while
the other is on asks which to keep. What the machine cannot do is shown as
**Unsupported** or **Not installed**, with the command that fixes it — never as
a broken control.

### AI Graphics (OptiScaler)

AI Graphics analyses the game that really runs (following launcher stubs to
the real executable): its graphics API, the DLSS/XeSS/FSR it ships as a DLL or
builds into the executable, proxy DLLs, anti-cheat and the GPU it renders on.
It shows the verdict first, then why, and what to set in the game's own menu.
Only **Apply** changes anything. It installs
[OptiScaler](https://github.com/optiscaler/OptiScaler) from the official
release (HTTPS, pinned SHA-256) with a verified backup, or raises FSR 3.1 to
FSR 4 through Proton. **Check Again**, **Repair**, **Restore** and **Diagnose**
are always there. Games with anti-cheat are never touched.

### Optimization, Telemetry and Records

**Telemetry** shows, once a second, what the processor (load, the fastest
core's clock and the average, temperature), memory (and swap), the disks
(read and written), the network (latency, received and sent) and every GPU
(load, temperature, clock, video memory, power, and which one renders the
game) are doing, each with a chart of the last minute. The cards keep their
size and place; a reading the driver does not publish says so in its place.
Nothing is read while a game has the screen.

**Game information**: each card shows ProtonDB's rating (or Native), the
anti-cheat status from Are We Anti-Cheat Yet? and Steam's player count; the
game's page explains each, with its source, its age and a link.

**Optimization** opens on an **Overview**: the machine at a glance and the
**Recommendations** for its own hardware and packages, each *Recommended*,
*Attention*, *Not supported*, *Informative*, *Already adequate* or *Needs
measurement*, with the command to copy or a button to the page that fixes
it. Its **Details** tab says for every part whether it is **active**,
**waiting**, **configured but not detected**, **off**, **missing a
dependency** or **unsupported**, with the evidence and the fix, and adds the
video pipeline. **Network and apps** has the network with a DNS comparison,
background load and broken Steam launch options.

**Records** keeps the last Turbo report and the support report, the games
that closed with an error and what was in them, and the **Logs**: falcond,
the helper, power-profiles-daemon, scx_loader, Gamescope, GPU drivers,
crashes and, from Steam games' output, Proton/Wine, MangoHud, vkBasalt,
lsfg-vk and OptiScaler, filtered by source.

### Measure the difference

Compares a game with and without optimisations over several alternated runs
and gives a verdict with Welch's t-test. A gain is called one only when it
exceeds the run-to-run spread. Method and results:
[docs/BENCHMARKS.md](docs/BENCHMARKS.md).

### Tray

Closing the window keeps Big Game Mode in the tray, with Turbo, the presets and
**Open** a click away. On GNOME the tray needs an AppIndicator extension.

## Screenshots

<table>
<tr>
<td width="50%"><img src="docs/screenshots/games.png" alt="Games: the library of installed games, with ProtonDB, anti-cheat and player counts"></td>
<td width="50%"><img src="docs/screenshots/game.png" alt="A game's page: Linux compatibility, anti-cheat and Steam activity"></td>
</tr>
<tr>
<td align="center"><b>Games</b>: every installed game, with how it runs on Linux</td>
<td align="center"><b>A game's page</b>: its compatibility, sources and dates</td>
</tr>
<tr>
<td><img src="docs/screenshots/optimization.png" alt="Optimization: the machine at a glance and its recommendations"></td>
<td><img src="docs/screenshots/details.png" alt="Optimization, Details: what is in effect, with the evidence"></td>
</tr>
<tr>
<td align="center"><b>Optimization</b>: what would help this machine</td>
<td align="center"><b>Details</b>: what is in effect, with the evidence</td>
</tr>
<tr>
<td><img src="docs/screenshots/tuning.png" alt="Tuning: performance, Gamescope, image quality and frame generation"></td>
<td><img src="docs/screenshots/ai-graphics.png" alt="AI Graphics: the verdict for a game and what to do"></td>
</tr>
<tr>
<td align="center"><b>Tuning</b>: what the machine cannot do is shown as such</td>
<td align="center"><b>AI Graphics</b>: the verdict first, then why</td>
</tr>
<tr>
<td><img src="docs/screenshots/records.png" alt="Records: the last Turbo report"></td>
<td><img src="docs/screenshots/telemetry.png" alt="Telemetry: processor, memory, disks, network and every GPU"></td>
</tr>
<tr>
<td align="center"><b>Records</b>: the Turbo report, crashes and logs</td>
<td align="center"><b>Telemetry</b>: second by second</td>
</tr>
<tr>
<td><img src="docs/screenshots/settings.png" alt="Settings: theme, background start, notifications, game information"></td>
<td><img src="docs/screenshots/about.png" alt="About Big Game Mode"></td>
</tr>
<tr>
<td align="center"><b>Settings</b>: themes, game information, advanced options</td>
<td align="center"><b>About</b></td>
</tr>
</table>

## Installation

### BigLinux, Manjaro and Arch Linux

Big Game Mode builds and installs from the official Arch repositories alone
(`core`, `extra`, `multilib`). No extra repository has to be added.

```bash
sudo pacman -S --needed base-devel git
git clone https://github.com/biglinux/bigamemode.git
cd bigamemode/pkgbuild
makepkg -si
```

`makepkg` installs what the build needs, builds `main` from GitHub, checks the
translation template, runs the tests and installs the package. Then open
**Big Game Mode** from the application menu.

### Requirements

| Package | Why |
|---|---|
| `gtk4`, `libadwaita`, `glib2`, `hicolor-icon-theme` | the interface |
| `dbus`, `polkit`, `systemd` | the privileged helper: a system-bus service started by systemd, every method authorised by Polkit |
| `curl`, `libarchive` | downloading and unpacking OptiScaler (AI Graphics) |
| `hwdata` | graphics card names |
| `iputils`, `iproute2` | network latency (Telemetry) and queue discipline (Details) |

### Optional components

Each one unlocks features; without it the feature says it is unavailable and
how to install it.

| Package | Unlocks | Where it comes from |
|---|---|---|
| `falcond` | per-game performance profiles (without it Turbo applies only the general settings and presets) | BigLinux repository |
| `power-profiles-daemon` | the power profile games run with (falcond asks for it) | `extra` |
| `scx-scheds`, `scx-tools` | the sched-ext CPU schedulers falcond switches to per game | `extra` |
| `gamescope` | Gamescope per game and in presets | `extra` |
| `mangohud`, `lib32-mangohud` | the overlay, its frame cap, and *Measure the difference* (32-bit games need the `lib32` one) | `extra`, `multilib` |
| `vkbasalt` | CAS sharpening and the Nara Linux look | BigLinux repository |
| `lsfg-vk` | frame generation with your own `Lossless.dll` | BigLinux repository |
| `nvidia-utils` | GPU telemetry on NVIDIA cards (NVML) | `extra` |
| `networkmanager` | applying a DNS server from the DNS comparison | `extra` |
| `ntsync-autoload` | loading the NTSync driver at boot for Proton/Wine | `extra` |
| `libkscreen` (KDE Plasma) or `xorg-xrandr` | the screen's refresh rate (the Quiet preset's cap) and whether adaptive sync is off | `extra` |

On BigLinux every optional component above is in the distribution's own
repositories; on Arch, the three marked "BigLinux repository" come from the
AUR. The launchers themselves (Steam, Heroic, Lutris) are not dependencies:
Big Game Mode finds the ones installed, native or Flatpak.

The package installs `bigame-ui` (the application, run as your user),
`bigame-daemon` (the root helper) with its systemd unit, D-Bus files and Polkit
policy, the desktop entry, AppStream metadata, icons and translations.

> [!TIP]
> On upgrade the helper stops and comes back, new, at its next call. On
> removal falcond is handed back as it was before Big Game Mode took charge.
> Files AI Graphics placed in games stay until **Restore the game's graphics**,
> or, for every game at once, Settings → Advanced → **Restore every game's
> graphics** (`bigame-ui --restore-all-graphics` in a terminal) — do it before
> removing the package. The backups are in `~/.local/state/bigame-mode/graphics`.

## Running

Open **Big Game Mode** from the menu, or run `bigame-ui`. `bigame-ui --background`
starts it in the tray; `bigame-ui --diagnostics` prints the support report in a
terminal without opening a window (`--network` adds the DNS measurements), and
`bigame-ui --restore-all-graphics` puts back every game AI Graphics changed.

## Troubleshooting

| Symptom | What to do |
|---|---|
| Games seem stuck at 60 FPS | Before 2.4.0, *Locked 60 FPS* and *Enhanced graphics* capped every game at 60. Choose **Fastest**, then reopen Steam: it keeps the environment it was opened with |
| Stutter in big games that no setting fixes | Optimization → Overview says when the game library is on a hard disk or adaptive sync is off: move the games to an SSD, turn adaptive sync on |
| I can't find Tuning, Details or Logs | They are tabs of Optimization and Records, shown in advanced mode: Settings → Advanced → **Show advanced options** |
| Games get no performance profile | Install `falcond`: without it Turbo applies only the general settings. Optimization → Overview names anything else missing |
| A game does not get its profile | Optimization → Details shows whether falcond applies it and why not; falcond 2.0.3 or newer is needed for games that rename their main thread (Cyberpunk 2077) |
| A preset does not reach Steam's games | Steam keeps the settings it was opened with: use **Reopen Steam** in the notice |
| A frame generation option is disabled | lsfg-vk is not installed or your `Lossless.dll` is not usable by it; the row says which |
| AI Graphics left a game broken | Games → the game's menu → **Restore the game's graphics**, or AI Graphics → **Repair** |
| No tray icon on GNOME | Install and enable an AppIndicator extension |
| "falcond is not compatible with this processor" | The installed falcond was built for a newer x86-64 level than this processor's (falcond 2.0.14 built for x86-64-v3 stops with SIGILL without AVX2/BMI2). Turbo still comes on with the general settings; per-game profiles need a falcond built for the x86-64 baseline. Turning Turbo off and on again does not help. [CPU compatibility](docs/CPU-COMPATIBILITY.md) |
| A game's tag says "Denied" or "Broken" | Its anti-cheat refuses Linux or does not work there, as players report it: online play will not work, whatever Big Game Mode or Proton does. Open the tag for the source |
| A game shows no rating | It is not a Steam game, or nobody has reported on it yet, or Big Game Mode has not been able to ask: Settings → Game Information says which |
| A game closed by itself | Big Game Mode says so when the game left a sign of a crash, with what was in it: **Leave them out of this game** for a while tells whether vkBasalt or FSR 4 had a part in it. A crash with nothing added is the game's, its drivers' or the system's |
| Something else | Records → Reports → **Support report**, or `bigame-ui --diagnostics`, and Records → **Logs** |

## How it works

- **falcond owns system performance.** It recognises a game by its process
  name, applies the profile and restores everything when the game closes. Big
  Game Mode switches falcond on and off (Turbo) and writes the profiles it reads.
- **Every setting has one owner.** Big Game Mode never applies itself what
  falcond or power-profiles-daemon already apply. Feral GameMode is not used:
  both would fight over the same settings.
- **A Steam game is started by Steam,** out of Big Game Mode's reach. What its
  profile asks for goes into the game's launch options, written with Steam
  closed, on every account; only the part Big Game Mode wrote is ever changed
  or removed.
- **AI Graphics detects what a game really uses** from the executable's import
  table, not DLL names, and records tested combinations that crash in its game
  list. Every install is a transaction: verified backup, journal, atomic swap
  and verification; an interrupted install is undone at the next start.
- Big Game Mode **never redistributes third-party binaries** and never downloads
  or replaces NVIDIA DLLs.

More in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Security

```text
┌─────────────────────────┐  D-Bus (system bus) ┌─────────────────────────┐
│ bigame-ui      (user)   │ ──────────────────▶ │ bigame-daemon   (root)  │
│ GTK4 + libadwaita, tray │   every call goes   │ validates every argu-   │
│ AI Graphics, measure-   │   through Polkit    │ ment; falcond profiles  │
│ ments, logs             │                     │ and config, Turbo, sysfs│
└───────────┬─────────────┘                     └───────────┬─────────────┘
            │ reads the state                               ▼
            └──────────────────────────────▶ falcond ──▶ scx_loader,
                                             (profiles)  power-profiles-daemon
```

- **The interface never runs as root.** Only a small helper on the system bus
  is privileged, confined by systemd (`ProtectSystem=strict`, `/run` and `/sys`
  read-only except the CPU frequency and device attributes it sets, one
  capability, memory and task limits).
- **Every privileged method is authorised by Polkit first** (without Polkit,
  access is denied) and every argument is validated on the root side.
- **Narrow, atomic writes** to one known place each, never through a path the
  caller builds. Profiles cannot carry `start_script`/`stop_script`.
- **No command goes through a shell**; downloads are HTTPS-only with a pinned
  SHA-256 and a checked archive listing.

Details in [docs/SECURITY.md](docs/SECURITY.md).

## Compatibility

| | |
|---|---|
| **System** | BigLinux and other Arch/Manjaro systems with systemd. falcond 2.0.3 or newer is recommended |
| **CPU** | Any x86-64 processor: the package is built for the x86-64 baseline, and CI runs it on emulated processors from an AMD Opteron (2003) to current ones. falcond has to be built the same way ([CPU compatibility](docs/CPU-COMPATIBILITY.md)) |
| **Desktop** | Tested on KDE Plasma (Wayland); X11 sessions are supported but less tested |
| **Games** | Steam (including Proton), Heroic, Lutris and native games from the menu, native or Flatpak |
| **GPUs** | AMD, NVIDIA and Intel, including hybrid laptops (the GPU a game renders on is identified, with PRIME offload) |
| **Tested on** | Ryzen 7 5700G with Radeon RX 9060 XT (RDNA 4) and Radeon Vega; a hybrid laptop with Intel HD 630 and GeForce GTX 1050 Ti (NVIDIA 580); a virtual machine with a stock BigLinux |
| **Detected, not yet tested on real hardware** | RDNA 3, RTX, Intel Arc, hybrid CPUs, 3D V-Cache, laptops on battery, VRR and HDR. There, Big Game Mode offers only what it detects as supported |

## Benchmarks

Highlights (each a session of alternated runs with a Welch's t-test verdict):

| Measured | Result |
|---|---|
| Shadow of the Tomb Raider, 3440×1440, RX 9060 XT: OptiScaler FSR vs native TAA | **+10.1 %** (89.8 → 98.8 FPS); the game's own XeSS +4.9 % |
| Same game, GTX 1050 Ti laptop: OptiScaler FSR 3.1 vs the game's XeSS Quality | **+20.1 %** (24.2 → 29.0 FPS) |
| SuperTuxKart on the hybrid laptop: PRIME offload vs the integrated GPU | **4.7×** (13.0 → 61.2 FPS) |
| A 60 FPS cap (Locked 60, now Quiet), SuperTuxKart | 455 → 60 FPS, GPU power 53 → **31 W** |
| GPU DPM level pinned to `high` | 7.5–8.3 % **slower**, so Big Game Mode never forces it |
| Power profile, governor, sched-ext schedulers | no measurable change in GPU-bound games |

Method and every result: [docs/BENCHMARKS.md](docs/BENCHMARKS.md). Each
session's report and metrics are in `bigame-engine/benchmarks/`; the raw
captures are at the tag `benchmarks-raw-data`.

## Languages

Big Game Mode is written in English and translated into **29 languages**:
Bulgarian, Chinese, Croatian, Czech, Danish, Dutch, English, Estonian, Finnish,
French, German, Greek, Hebrew, Hungarian, Icelandic, Italian, Japanese, Korean,
Norwegian Bokmål, Polish, Portuguese, Brazilian Portuguese, Romanian, Russian,
Slovak, Spanish, Swedish, Turkish and Ukrainian. It follows the system
language. The catalogues are in [`locale/`](locale); corrections and new
languages are welcome.

## Development

Requires Rust 1.88 or newer, GTK 4.14+, libadwaita 1.7+,
`glib-compile-resources` (glib2) and, for the catalogues, gettext and Python 3.

```bash
cd bigame-engine
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings -A clippy::assert_is_empty
./target/debug/bigame-ui
./target/debug/bigame-ui --diagnostics
```

| Crate | Role |
|---|---|
| `bigame-core` | all the logic, no interface: detection, profiles, Turbo, AI Graphics, telemetry, measurements |
| `bigame-ui` | the GTK4/libadwaita application and the tray icon |
| `bigame-daemon` | the root helper on D-Bus, authorised by Polkit |

- `-A clippy::assert_is_empty` needs Clippy 1.99 or newer, where that lint
  exists; with an older one, leave it out.
- Without the package installed the interface runs, but what needs root
  (profiles, Turbo, falcond's configuration) is unavailable.
- `tests/daemon-authorization.sh` checks that the helper refuses every
  privileged method when Polkit is unreachable.
- `bigame-core/examples/` holds command-line tools: detection (`detect`,
  `library`, `running`, `health`), AI Graphics (`graphics_scan`,
  `graphics_plan`, `graphics_apply`, `graphics_status`,
  `graphics_capabilities`, `graphics_diagnose`, `graphics_native`), Turbo and
  Booster (`turbo`, `turbo_preset`, `booster_run`, `measure`), falcond control
  (`falcond_control`), the launch command (`launch_plan`), a game started
  through its launcher (`launch_through`), a game's settings in Heroic
  (`heroic_apply`), a game's Gamescope in Steam's launch options
  (`steam_gamescope`), the vkBasalt look (`vkbasalt_style`), lsfg-vk (`lsfg`)
  and benchmark reports (`bench_report`, `bench_native_report`). Run them with
  `cargo run -p bigame-core --example <name>`.

### Translations

Every visible text goes through `i18n`/`ni18n` in the interface and `N_` in
`bigame-core`. After changing texts:

```bash
python3 locale/extract-strings.py        # updates locale/bigame-mode.pot
for po in locale/*.po; do
    msgmerge -U --no-wrap --no-fuzzy-matching "$po" locale/bigame-mode.pot
done
```

The extractor refuses to run when a file with texts is missing from
`locale/POTFILES.in`, and the package build fails when the template is stale.
To try a translation from the build tree, compile the catalogues and point the
application at them:

```bash
for po in locale/*.po; do
    l=$(basename "$po" .po); mkdir -p /tmp/bgm-locale/$l/LC_MESSAGES
    msgfmt -o /tmp/bgm-locale/$l/LC_MESSAGES/bigame-mode.mo "$po"
done
BIGAME_LOCALEDIR=/tmp/bgm-locale LANGUAGE=pt_BR bigame-engine/target/debug/bigame-ui
```

### Packaging

`pkgbuild/PKGBUILD` builds `main` from GitHub with
`cargo build --release --frozen --workspace` (LTO), checks the translation
template, compiles the catalogues and runs `cargo test --release --frozen
--workspace` in `check()`.

One run checks every pull request and every push (details in
[`docs/CI.md`](docs/CI.md)), with two jobs in
parallel:

- the code (`.github/workflows/backend-tests.yml`): formatting, Clippy with
  warnings as errors, the tests, `cargo doc` with warnings as errors, the
  helper's authorization test as an ordinary user, the translation template
  and catalogues;
- the package (`.github/workflows/build-package.yml`): a `makepkg` build of
  that very commit with `pkgbuild/PKGBUILD`, checked with namcap,
  `desktop-file-validate`, `appstreamcli` and a look at every file it
  installs; the package is kept as an artifact.

Pushes to `main`, `testing-*` and `stable-*` run as *Build Package*; a
successful one is what the BigLinux package builders build. Pull requests run
as *Package check*, which is not handed to them.

## Author

**Rafael Ruscher**, author and lead developer · <rruscher@gmail.com>

I have always loved games, and I am a firm believer in gaming on Linux. In
recent years, with Valve's constant work, compatibility has become almost
complete. I play with friends like **Barnabé di Kartola** and follow
**Alessandro** and **Pacheco** of the **System Infotech** channel, who play
every day; seeing BigLinux in action on their channels keeps me going. Big Game
Mode exists so this community gets the most out of its hardware, with the latest
technology for every frame.

**Thanks to** Bruno Gonçalves, Barnabé di Kartola, Alessandro and Pacheco
(System Infotech), Narayan Silva (Nara Linux) and the BigLinux community.

## Built on

Big Game Mode builds on third-party projects, each with its own authors and
licence:

- **System:** [falcond](https://git.pika-os.com/general-packages/falcond)
  (PikaOS), [sched-ext](https://github.com/sched-ext/scx) and `scx_loader`,
  power-profiles-daemon, systemd, D-Bus and Polkit.
- **Games and graphics:** [OptiScaler](https://github.com/optiscaler/OptiScaler),
  Gamescope and Proton (Valve), DXVK, VKD3D-Proton,
  [lsfg-vk](https://github.com/PancakeTAS/lsfg-vk), MangoHud and vkBasalt. The
  vkBasalt **Nara Linux** look is by Narayan of the
  [Nara Linux](https://www.youtube.com/watch?v=GGBC-qMB_0Y) channel and uses
  shaders from [ReShade](https://github.com/crosire/reshade-shaders) and
  [SweetFX](https://github.com/CeeJayDK/SweetFX), downloaded only when that
  look is chosen, at fixed versions checked by SHA-256. DLSS, XeSS and FSR
  belong to NVIDIA, Intel and AMD under their licences.
- **Application:** Rust, GTK and libadwaita (GNOME), gtk4-rs, zbus, Tokio,
  Serde, ksni and gettext.

## License

Distributed under the **GPL-3.0-or-later**. See [LICENSE](LICENSE).

<div align="center">
<sub>Made with 🎮 for the <b>BigLinux</b> community.</sub>
</div>
