//! Put a Turbo preset in force or take it away, the way Turbo does, and
//! print what the session holds afterwards.
//!
//! Usage: `turbo_preset <standard|more_fps|locked60|enhanced|off|status>`
use bigame_core::turbo_preset::{self, Preset};

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_else(|| "status".into());
    let result = match arg.as_str() {
        "status" => {
            println!(
                "chosen {:?}, in force {:?}, levers {:?}",
                turbo_preset::chosen(),
                turbo_preset::active(),
                turbo_preset::active_levers()
            );
            return;
        }
        "off" => turbo_preset::deactivate(),
        id => {
            let Some(p) = Preset::from_id(id) else {
                eprintln!("unknown preset {id}");
                std::process::exit(2);
            };
            turbo_preset::activate(p)
        }
    };
    match result {
        Ok(set) => {
            for a in set {
                println!("{a}");
            }
        }
        Err(e) => {
            eprintln!("{e:#}");
            std::process::exit(1);
        }
    }
}
