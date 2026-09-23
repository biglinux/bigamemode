//! Print what `hardware::Hardware::detect()` sees on this machine.
fn main() {
    let hw = bigame_core::hardware::Hardware::detect();
    println!("kernel   {}", hw.kernel);
    println!(
        "session  {:?}   chassis {:?}   power {:?}",
        hw.session, hw.chassis, hw.power_source
    );
    println!("cpu      {:?} {}", hw.cpu.vendor, hw.cpu.model);
    println!(
        "         {}C/{}T smt={} hybrid={}",
        hw.cpu.physical_cores, hw.cpu.logical_cpus, hw.cpu.smt, hw.cpu.hybrid
    );
    println!(
        "         driver={:?} gov={:?} avail={:?}",
        hw.cpu.scaling_driver, hw.cpu.current_governor, hw.cpu.available_governors
    );
    println!(
        "         epp={:?} avail_epp={:?} pstate={:?}",
        hw.cpu.current_epp, hw.cpu.available_epp, hw.cpu.amd_pstate_status
    );
    println!("         vcache={:?}", hw.cpu.vcache);
    for (i, g) in hw.gpus.iter().enumerate() {
        println!(
            "gpu[{i}]   {} {:?} {} drv={} discrete={} vram={:?}",
            g.card, g.vendor, g.pci_id, g.driver, g.discrete, g.vram_total_bytes
        );
        println!(
            "         outputs={:?} hwmon={:?}",
            g.connected_outputs, g.hwmon
        );
        println!(
            "         dpm={:?} busy={:?} temp={:?} power_uw={:?}",
            g.dpm_level(),
            g.busy_percent(),
            g.hwmon_u64("temp1_input"),
            g.hwmon_u64("power1_average")
        );
    }
    println!("render   {:?}", hw.render_gpu().map(|g| &g.card));
    for d in &hw.displays {
        println!(
            "display  {} on {} max={:?} vrr={:?}",
            d.connector, d.card, d.max_mode, d.vrr_capable
        );
    }

    let caps = bigame_core::capabilities::Capabilities::detect();
    println!("\n--- capabilities ---");
    match &caps.gamescope {
        Some(g) => println!(
            "gamescope  v{:?}, {} flags, -F={} --adaptive-sync={} --hdr-enabled={} --fsr={}",
            g.version,
            g.flags.len(),
            g.has_flag("F"),
            g.has_flag("adaptive-sync"),
            g.has_flag("hdr-enabled"),
            g.has_flag("fsr")
        ),
        None => println!("gamescope  not installed"),
    }
    println!(
        "mangohud={} mangoapp={} vkbasalt={} lsfg_vk={} steam={}",
        caps.mangohud, caps.mangoapp, caps.vkbasalt, caps.lsfg_vk, caps.steam
    );
    println!(
        "falcond    installed={} running={}",
        caps.falcond_installed, caps.falcond_running
    );
    println!("gamemode   {}", caps.gamemode);
    println!(
        "ppd        reachable={} profiles={:?}",
        caps.power_profiles, caps.power_profiles_available
    );
    println!(
        "sched_ext  kernel={} state={:?} scxctl={} loader={}",
        caps.sched_ext.kernel_support,
        caps.sched_ext.state,
        caps.sched_ext.scxctl,
        caps.sched_ext.loader_service
    );
    println!(
        "           installed({}) = {:?}",
        caps.sched_ext.installed.len(),
        caps.sched_ext.installed
    );
    println!("           switchable -> {:?}", caps.sched_ext.switchable());
}
