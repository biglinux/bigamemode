//! Performance: Turbo, falcond, the power profile, the scheduler and 3D
//! V-Cache — each as a row whose line says how it stands and whose body
//! says what it means, what the evidence is, and what to do.

use adw::prelude::*;
use libadwaita as adw;

use bigame_core::capabilities::Support;
use bigame_core::overview::{AppliedProfile, RequestedBy, Snapshot, State};

use crate::i18n::{i18n, ni18n};
use crate::widgets::status::{Body, StatusRow};

/// The performance group.
#[derive(Clone)]
pub struct Performance {
    group: adw::PreferencesGroup,
    turbo: StatusRow,
    falcond: StatusRow,
    power: StatusRow,
    scheduler: StatusRow,
    vcache: StatusRow,
    cpu_model: String,
}

impl Performance {
    /// Build the group.
    #[must_use]
    pub fn new(hw: &bigame_core::hardware::Hardware) -> Self {
        let group = adw::PreferencesGroup::new();
        group.set_title(&i18n("Performance"));
        group.set_description(Some(&i18n(
            "What is applied to the system while a game runs, and how each item was verified.",
        )));
        let turbo = StatusRow::new(
            &i18n("Turbo"),
            &i18n("The main switch: falcond optimizes each game as it starts"),
            "power-profile-performance-symbolic",
        );
        let falcond = StatusRow::new(
            "falcond",
            &i18n("Matches the running game and applies its profile"),
            "system-run-symbolic",
        );
        let power = StatusRow::new(
            &i18n("Power profile"),
            &i18n("power-profiles-daemon's profile: performance while a game runs"),
            "battery-full-charging-symbolic",
        );
        let scheduler = StatusRow::new(
            &i18n("CPU scheduler (sched-ext)"),
            &i18n("A scheduler tuned for games, loaded by falcond through scx_loader"),
            "cpu-symbolic",
        );
        let vcache = StatusRow::new(
            &i18n("3D V-Cache"),
            &i18n("Which CCD games prefer on an AMD X3D processor"),
            "drive-multidisk-symbolic",
        );
        for r in [&turbo, &falcond, &power, &scheduler, &vcache] {
            group.add(r.widget());
        }

        Self {
            group,
            turbo,
            falcond,
            power,
            scheduler,
            vcache,
            cpu_model: crate::views::home::short_cpu(&hw.cpu.model),
        }
    }

    /// The group.
    #[must_use]
    pub fn group(&self) -> &adw::PreferencesGroup {
        &self.group
    }

    /// Show a reading.
    #[allow(clippy::too_many_lines)]
    pub fn show(&self, snap: &Snapshot) {
        let game = snap.game.is_some();
        let yes = i18n("Yes");
        let no = i18n("No");
        let none = i18n("none");

        // ── Turbo ───────────────────────────────────────────────────────
        let turbo_state = snap.turbo_state();
        self.turbo.set_state(
            turbo_state,
            None,
            &match turbo_state {
                State::Active => i18n("On: falcond runs and applies each game's profile"),
                State::Off => i18n("Off: BiGame-mode is not intervening in games"),
                State::Error => i18n("falcond's service failed"),
                State::Missing => i18n("falcond is not installed"),
                _ => String::new(),
            },
        );
        let mut body = Body::new()
            .note(&i18n(
                "Turbo is falcond's service, as systemd reports it: on means enabled and running, off means stopped. Nothing is claimed that systemd does not confirm.",
            ))
            .fact(
                &i18n("Service"),
                &if snap.turbo_on {
                    i18n("falcond.service active")
                } else {
                    i18n("falcond.service inactive")
                },
            );
        body = match turbo_state {
            State::Error => body.note(&i18n(
                "Turn Turbo off and on again at Home to restart it. Logs show why it failed.",
            )),
            State::Missing => body.command("sudo pacman -S falcond falcond-profiles"),
            State::Off => body.note(&i18n(
                "Turn Turbo on at Home. Until then no profile, scheduler or power profile is applied for a game.",
            )),
            _ => body,
        };
        self.turbo.set_body(body.build());

        // ── falcond ─────────────────────────────────────────────────────
        let falcond_state = snap.falcond_state();
        let (line, mut body) = match (&snap.falcond, falcond_state) {
            (Some(st), _) => {
                let line = match &snap.profile {
                    AppliedProfile::Own { name, .. } | AppliedProfile::Other(name) => {
                        i18n("Running · profile %s applied").replace("%s", name)
                    }
                    AppliedProfile::GenericProton => {
                        i18n("Running · general Proton profile applied")
                    }
                    AppliedProfile::None if game => {
                        i18n("Running · no profile for the running game")
                    }
                    AppliedProfile::None => i18n("Running · idle, no game"),
                };
                let mut body = Body::new()
                    .fact(&i18n("Profiles loaded"), &st.loaded_profiles.to_string())
                    .fact(
                        &i18n("Profile set"),
                        &if st.profile_mode.is_empty() {
                            i18n("desktop")
                        } else {
                            st.profile_mode.clone()
                        },
                    );
                if let Some(g) = &snap.game {
                    body = body.fact(&i18n("Game detected"), &g.process_name);
                }
                body = match &snap.profile {
                    AppliedProfile::Own { name, path, user } => body
                        .fact(&i18n("Profile applied"), name)
                        .fact(
                            &i18n("Kind"),
                            &if *user {
                                i18n("Your own profile")
                            } else {
                                i18n("Built into falcond")
                            },
                        )
                        .fact_opt(
                            &i18n("File"),
                            path.as_ref().map(|p| p.display().to_string()).as_deref(),
                        ),
                    AppliedProfile::GenericProton => body
                        .fact(&i18n("Profile applied"), "Proton")
                        .note(&i18n(
                            "\"Proton\" is falcond's general profile for any Proton game without a profile of its own: performance mode and screen-saver inhibit, no scheduler, no V-Cache preference. Create a profile in Profiles to choose more.",
                        )),
                    AppliedProfile::Other(name) => body.fact(&i18n("Profile applied"), name),
                    AppliedProfile::None if game => body.note(&i18n(
                        "falcond matched no profile to this process. It applies nothing for a game it does not know; create a profile in Profiles.",
                    )),
                    AppliedProfile::None => body,
                };
                if game || st.active_profile.is_some() {
                    body = body
                        .fact(
                            &i18n("Power profile → performance"),
                            if st.perf_mode_active { &yes } else { &no },
                        )
                        .fact(
                            &i18n("Screen-saver inhibited"),
                            if st.screensaver_inhibited { &yes } else { &no },
                        )
                        .fact(
                            &i18n("Scheduler"),
                            if st.current_scx.is_empty() { &none } else { &st.current_scx },
                        )
                        .fact(
                            &i18n("V-Cache"),
                            if st.current_vcache.is_empty() {
                                &none
                            } else {
                                &st.current_vcache
                            },
                        );
                }
                (line, body)
            }
            (None, State::Off) => (
                i18n("Stopped: Turbo is off"),
                Body::new().note(&i18n("falcond is stopped while Turbo is off. That is expected.")),
            ),
            (None, State::Missing) => (
                i18n("Not installed"),
                Body::new().command("sudo pacman -S falcond falcond-profiles"),
            ),
            (None, State::Error) => (
                i18n("The service failed"),
                Body::new().command("journalctl -u falcond -n 50"),
            ),
            (None, _) => (
                i18n("Running, but its status could not be read"),
                Body::new().note(&i18n(
                    "falcond writes its status to /var/lib/falcond/status (or /tmp/falcond_status). The file is missing, or not owned by root, so nothing is trusted from it.",
                )),
            ),
        };
        body = body.note(&i18n(
            "Evidence: falcond's own status file, read every few seconds and watched for changes.",
        ));
        self.falcond.set_state(falcond_state, None, &line);
        self.falcond.set_body(body.build());

        // ── Power profile ───────────────────────────────────────────────
        let power_state = snap.power_state();
        let current = snap
            .power_profile
            .clone()
            .unwrap_or_else(|| i18n("unavailable"));
        self.power.set_state(
            power_state,
            Some(&current),
            &match power_state {
                State::Active => i18n("performance while the game runs, as falcond asked"),
                State::NotDetected => {
                    i18n("A game runs under Turbo, but the profile is %s").replace("%s", &current)
                }
                State::Waiting => {
                    i18n("%s now; performance when a game starts").replace("%s", &current)
                }
                State::Off => i18n("%s; Turbo is off, so nothing changes it for games")
                    .replace("%s", &current),
                State::Missing => i18n("power-profiles-daemon is not reachable"),
                _ => current.clone(),
            },
        );
        let mut body = Body::new().note(&i18n(
            "falcond asks power-profiles-daemon for the performance profile while a game with performance mode runs, and puts back the profile it found when it started.",
        ));
        body = body.fact(&i18n("Managed by"), "falcond → power-profiles-daemon");
        if power_state == State::NotDetected {
            body = body.note(&i18n(
                "The game's profile may have performance mode off, or falcond found the game after another program changed the profile. Check the profile in Profiles.",
            ));
        }
        if power_state == State::Missing {
            body = body.command("sudo systemctl enable --now power-profiles-daemon");
        }
        self.power.set_body(body.build());

        // ── Scheduler ───────────────────────────────────────────────────
        let sched = &snap.scheduler;
        let sched_state = sched.state(game);
        let loaded = sched.loaded.clone();
        let requested = (!sched.requested.is_empty() && sched.requested != "none")
            .then(|| sched.requested.clone());
        let chip = loaded.clone().or_else(|| requested.clone());
        let line = match (sched_state, &loaded, &requested) {
            (State::Active, Some(l), _) => i18n("%s is running").replace("%s", l),
            (State::NotDetected, Some(l), Some(r)) => i18n("%r was asked for, but %l is running")
                .replace("%r", r)
                .replace("%l", l),
            (State::NotDetected, None, Some(r)) => {
                i18n("%s was asked for, but no scheduler is loaded").replace("%s", r)
            }
            (State::Waiting, _, Some(r)) => i18n("%s when a game starts").replace("%s", r),
            (State::Off, _, _) => i18n("None asked for: the kernel's default scheduler"),
            (State::Missing, _, _) => i18n("Asked for, but it cannot be switched"),
            (State::Unsupported, _, _) => i18n("This kernel has no sched-ext"),
            _ => String::new(),
        };
        self.scheduler
            .set_state(sched_state, chip.as_deref(), &line);
        let mut body = Body::new().note(&i18n(
            "sched-ext replaces the kernel's CPU scheduler with one loaded at run time. Game-oriented ones (lavd, bpfland) favour the interactive task — the game — over background work.",
        ));
        body = body.fact_opt(
            &i18n("Asked for"),
            requested
                .as_deref()
                .map(|r| match sched.requested_by {
                    RequestedBy::GameProfile => i18n("%s, by the game's profile").replace("%s", r),
                    RequestedBy::GlobalConfig => {
                        i18n("%s, by falcond's global setting").replace("%s", r)
                    }
                    RequestedBy::Nobody => r.to_owned(),
                })
                .as_deref(),
        );
        body = body.fact(
            &i18n("Loaded now"),
            loaded.as_deref().unwrap_or(&i18n("none (kernel default)")),
        );
        body = body.fact(&i18n("Managed by"), "falcond → scx_loader");
        let installed = if sched.caps.installed.is_empty() {
            i18n("none")
        } else {
            sched.caps.installed.join(", ")
        };
        body = body.fact(&i18n("Installed"), &installed);
        body = match sched.caps.switchable() {
            Support::Available => body.fact(&i18n("scx_loader"), &i18n("available")),
            Support::Unsupported(_) => body.note(&i18n(
                "The running kernel was built without sched_ext. Nothing to install; a kernel with sched_ext (the default BigLinux kernel has it) is needed.",
            )),
            Support::NotInstalled(pkg) if pkg == "scx-tools" => body
                .note(&i18n(
                    "Schedulers are installed but scx_loader is not, and falcond switches schedulers only through it.",
                ))
                .command("sudo pacman -S scx-tools && sudo systemctl enable --now scx_loader"),
            Support::NotInstalled(_) => body
                .note(&i18n("No sched-ext scheduler is installed."))
                .command("sudo pacman -S scx-scheds scx-tools"),
            Support::ServiceDown(_) => body
                .note(&i18n("scx_loader is installed but its service is not running."))
                .command("sudo systemctl enable --now scx_loader"),
        };
        if sched_state == State::NotDetected {
            body = body.note(&i18n(
                "falcond asks scx_loader when the game starts; a failure is in Logs (scx_loader, falcond). Turning Turbo off and on again makes it retry.",
            ));
        }
        body = body.note(&i18n(
            "Evidence: /sys/kernel/sched_ext (what the kernel runs), falcond's status (what it asked), the installed scx_* binaries.",
        ));
        self.scheduler.set_body(body.build());

        // ── 3D V-Cache ──────────────────────────────────────────────────
        let vc = &snap.vcache;
        let vc_state = vc.state(game);
        let (line, body) = if vc.available {
            let asked = (!vc.requested.is_empty() && vc.requested != "none")
                .then_some(vc.requested.as_str());
            let line = match (vc_state, asked, vc.current.as_deref()) {
                (State::Active, _, Some(c)) => i18n("Mode %s applied").replace("%s", c),
                (State::NotDetected, Some(a), Some(c)) => i18n("%a was asked for, but %c is set")
                    .replace("%a", a)
                    .replace("%c", c),
                (State::Waiting, Some(a), _) => i18n("%s when a game starts").replace("%s", a),
                (State::Off, _, _) => i18n("No preference asked for"),
                _ => String::new(),
            };
            let body = Body::new()
                .note(&i18n(
                    "On an AMD X3D processor the profile can prefer the CCD with the extra cache (cache) or the faster one (freq).",
                ))
                .fact(&i18n("CPU"), &self.cpu_model)
                .fact_opt(&i18n("Asked for"), asked)
                .fact_opt(&i18n("Set now"), vc.current.as_deref())
                .fact(&i18n("Managed by"), "falcond → amd_x3d_vcache");
            (line, body)
        } else {
            (
                i18n("Not supported on this processor"),
                Body::new()
                    .fact(&i18n("CPU"), &self.cpu_model)
                    .note(&i18n(
                        "This needs an AMD X3D processor (5800X3D, 7800X3D, 9800X3D…). Nothing to do: it is a hardware feature, not a fault.",
                    )),
            )
        };
        self.vcache.set_state(vc_state, None, &line);
        self.vcache.set_body(body.build());

        let _ = ni18n; // the plural helper is used by other groups of the page
    }
}
