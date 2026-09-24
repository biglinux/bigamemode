//! Drive Turbo Mode from the command line, exactly as the Home button does.
//!
//! Usage: `turbo status | on | off | report`
use bigame_core::turbo::{self, Report};

fn print(report: &Report) {
    println!("Turbo {}", if report.turned_on { "ON" } else { "OFF" });
    for item in &report.items {
        println!(
            "  {:<16} {:<28} [{}] {}",
            format!("{:?}", item.section),
            match &item.kind {
                turbo::Kind::Knob(name) => name.clone(),
                other => format!("{other:?}"),
            },
            item.owner,
            item.detail
        );
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    match std::env::args().nth(1).as_deref().unwrap_or("status") {
        "status" => println!(
            "{:?} (owned since: {:?})",
            turbo::state().await?,
            turbo::owned_since()
        ),
        "on" => print(&turbo::turn_on(|step| println!("… {step:?}")).await?),
        "off" => print(&turbo::turn_off(|step| println!("… {step:?}")).await?),
        "report" => match Report::load_last() {
            Some(r) => print(&r),
            None => println!("no report yet"),
        },
        other => anyhow::bail!("unknown command {other}"),
    }
    Ok(())
}
