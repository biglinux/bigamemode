//! Dialog to explain SCX schedulers to the user.

use adw::prelude::*;
use gtk4;
use libadwaita as adw;

use crate::i18n::i18n;

/// Building a widget tree is inherently linear — splitting it yields helpers
/// with a single caller and no independent meaning — so the length lint is
/// allowed here rather than worked around.
#[allow(clippy::too_many_lines)]
/// Show an informational dialog explaining the various sched-ext schedulers.
pub fn show(parent: &gtk4::Window) {
    let dialog = adw::Window::builder()
        .title(i18n("Scheduler Information"))
        .modal(true)
        .default_width(600)
        .default_height(500)
        .build();
    dialog.set_transient_for(Some(parent));

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let header = adw::HeaderBar::new();
    vbox.append(&header);

    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vexpand(true)
        .build();

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    content.set_margin_top(24);
    content.set_margin_bottom(24);
    content.set_margin_start(24);
    content.set_margin_end(24);

    // Intro
    let title = gtk4::Label::builder()
        .label(format!(
            "<span size='large' weight='bold'>{}</span>",
            i18n("What is a Scheduler?")
        ))
        .use_markup(true)
        .halign(gtk4::Align::Start)
        .build();
    content.append(&title);

    let desc = gtk4::Label::builder()
        .label(i18n("The CPU scheduler (the traffic controller of the processor) decides which programs run on which CPU cores and for how long. The default Linux scheduler divides time fairly among all apps. However, for Gaming, we don't want fairness—we want absolute priority for the game! With sched-ext (SCX), we can dynamically swap the default scheduler for a specialized one without rebooting."))
        .wrap(true)
        .halign(gtk4::Align::Start)
        .build();
    content.append(&desc);

    // List of Schedulers
    let schedulers = [
        (
            "LAVD (scx_lavd)",
            i18n(
                "Latency-Aware Virtual Deadline. The absolute best choice for gaming! It heavily prioritizes interactive tasks (like your game rendering frames and reading your mouse clicks). It dramatically reduces stuttering and stabilizes FPS.",
            ),
        ),
        (
            "RUSTY (scx_rusty)",
            i18n(
                "Written in Rust, this is a highly intelligent and balanced scheduler. It perfectly understands modern CPU architectures with mixed cores (like Intel P-Cores/E-Cores or AMD X3D cache CCDs) and routes workloads perfectly.",
            ),
        ),
        (
            "FLASH (scx_flash)",
            i18n(
                "Designed for pure speed and minimal overhead. It makes decisions extremely fast and consumes almost zero CPU power for itself, leaving maximum raw performance for predictable workloads.",
            ),
        ),
        (
            "BPFLAND (scx_bpfland)",
            i18n(
                "A highly robust, general-purpose scheduler that ensures extreme fairness. It guarantees that no background app gets 'starved' while keeping the foreground responsive.",
            ),
        ),
        (
            "CAKE (scx_cake)",
            i18n(
                "Based on the famous CAKE networking algorithm, adapted for CPUs. It focuses on keeping task queues short and responsive under heavy system load.",
            ),
        ),
        (
            "COSMOS (scx_cosmos)",
            i18n(
                "An experimental scheduler designed to intelligently map complex workloads across massive multi-core systems, prioritizing fairness and topology.",
            ),
        ),
        (
            "LAYERED (scx_layered)",
            i18n(
                "Organizes tasks into strict priority layers. High-priority tasks are guaranteed to run before lower-priority ones, giving you absolute control over system priority.",
            ),
        ),
        (
            "RUSTLAND (scx_rustland)",
            i18n(
                "An innovative scheduler that runs most of its logic in user-space rather than kernel-space. Great for development, testing, and system safety.",
            ),
        ),
        (
            "BEERLAND (scx_beerland)",
            i18n(
                "An experimental and educational variation used within the SCX community to test new BPF capabilities. Fun to try, but maybe not for competitive gaming!",
            ),
        ),
        (
            "TICKLESS (scx_tickless)",
            i18n(
                "Designed to eliminate unnecessary CPU 'ticks' (timer wakes). This allows cores to sleep deeper and longer, making it an incredible choice for saving laptop battery life.",
            ),
        ),
        (
            "P2DQ (scx_p2dq)",
            i18n(
                "Power-of-2-Choices Double Queue. A highly academic, low-latency scheduler that uses mathematical probability to rapidly balance load across many CPU cores.",
            ),
        ),
        (
            "CHAOS (scx_chaos)",
            i18n(
                "A testing scheduler that makes scheduling decisions completely randomly. Used by developers to uncover hidden concurrency bugs. Do not use for gaming!",
            ),
        ),
        (
            "PANDEMONIUM (scx_pandemonium)",
            i18n(
                "Similar to Chaos, it aggressively stress-tests the CPU by constantly migrating tasks between cores in the most unpredictable ways possible.",
            ),
        ),
    ];

    for (name, description) in schedulers {
        let group = gtk4::Box::new(gtk4::Orientation::Vertical, 4);

        let s_title = gtk4::Label::builder()
            .label(format!("<span weight='bold'>{name}</span>"))
            .use_markup(true)
            .halign(gtk4::Align::Start)
            .build();
        group.append(&s_title);

        let s_desc = gtk4::Label::builder()
            .label(&description)
            .wrap(true)
            .halign(gtk4::Align::Start)
            .build();
        group.append(&s_desc);

        content.append(&group);
    }

    scroll.set_child(Some(&content));
    vbox.append(&scroll);
    dialog.set_content(Some(&vbox));

    dialog.present();
}
