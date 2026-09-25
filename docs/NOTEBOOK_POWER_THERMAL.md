# Notebook power and thermals

What the lab laptop's power and temperature do during games, and what
BiGame-mode does about it.

## Power supply

The laptop has **no battery** (`ACPI: battery: Slot [BAT0] (battery absent)`);
it runs on mains only. The two batteries upower lists are a Logitech keyboard
and mouse (`scope=Device`). Nothing about battery behaviour could be measured
here; BiGame-mode's Booster already skips power profile, governor and GPU DPM
when it detects a battery as the power source at Turbo-on time, and never
re-plans on plug or unplug (see "Pending").

## Who sets CPU frequency policy

| Setting | Owner |
|---|---|
| Power profile | power-profiles-daemon; falcond asks for `performance` for each game with a profile and restores it after |
| Governor | power-profiles-daemon's BigLinux companion, `power-profiles-daemon-biglinux-cpufreq`: performance → `performance`, balanced → `schedutil`, power-saver → `conservative`, on every profile change |
| EPP | power-profiles-daemon (unchanged here: `balance_performance`) |

The Booster used to write the governor too. On this laptop the companion put
`schedutil` back as soon as falcond restored the balanced profile after a
game, while the Turbo report kept saying "performance, verified". The
governor is now left to power-profiles-daemon wherever it runs. A game cycle
with Turbo on, checked on the machine:

| | Power profile | Governor | falcond profile |
|---|---|---|---|
| before | balanced | schedutil | none |
| in game | performance | performance | `SOTTR.exe` |
| after | balanced | schedutil | none |

## Temperatures and limits in a game

Shadow of the Tomb Raider's benchmark, lowest preset (averages per run):

| | Turbo off | Turbo on |
|---|---|---|
| CPU package | 87–88 °C | 88–89 °C |
| GPU | 75–78 °C | 77–78 °C |
| GPU clock | 1593–1608 MHz | 1563–1615 MHz |
| GPU at its power limit | 30–31 % of the time | 30–38 % |

- The CPU package peaked at **99 °C**. Its throttle counters over the
  afternoon of benchmarks (cpu0): core throttling events 373 → 14 219
  (76 s slowed), package throttling events 2 081 → 42 746 (289 s slowed).
- The GTX never hit a thermal limit; its clock is held by its power limit
  (NVML's "SW power cap").
- Turbo made no difference to frame rate and none to temperature.

## What BiGame-mode shows now

- Diagnostics → **CPU temperature**: the kernel's throttle counters, with a
  warning once the CPU has hit its limit — a CPU capped by its cooling is
  not helped by a performance governor.
- Details → **GPU**: the game GPU's clock, load and temperature, and the
  reason its clock is held down ("power limit", "temperature limit").

## Pending

- Battery: no battery on this machine.
- Plug/unplug during a session: the Booster plans once, at Turbo-on; there is
  no UPower watch. A profile's `performance_mode` is decided by the power
  source at the moment the profile is created and then kept.
- Power draw: the GTX 1050 Ti Mobile does not report board power through NVML
  (`power.draw [N/A]`), so the energy cost of any setting was not measured.
