//! Print the support diagnostics report.
fn main() {
    let network = std::env::args().any(|a| a == "--network");
    print!("{}", bigame_core::diagnostics::report(network));
}
