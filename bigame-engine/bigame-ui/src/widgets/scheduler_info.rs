//! The `(i)` that explains the sched-ext schedulers.

use crate::i18n::i18n;

/// The `(i)` for the scheduler.
#[must_use]
pub fn button() -> gtk4::Button {
    crate::widgets::info::dialog_button(&i18n("Learn about Schedulers"), content)
}

fn content() -> (String, String, Vec<crate::widgets::info::Entry>) {
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
    (
        i18n("What is a Scheduler?"),
        i18n(
            "The CPU scheduler (the traffic controller of the processor) decides which programs run on which CPU cores and for how long. The default Linux scheduler divides time fairly among all apps. However, for Gaming, we don't want fairness—we want absolute priority for the game! With sched-ext (SCX), we can dynamically swap the default scheduler for a specialized one without rebooting.",
        ),
        schedulers
            .into_iter()
            .map(|(name, body)| crate::widgets::info::Entry {
                title: name.to_owned(),
                body,
            })
            .collect(),
    )
}
