//! Print what the PE reader sees in each file given: CPU, imports, delay
//! imports and file version.
fn main() {
    for arg in std::env::args().skip(1) {
        let path = std::path::Path::new(&arg);
        match bigame_core::graphics::pe::parse_file(path, 64 << 20) {
            Ok(info) => println!(
                "{}\n  machine: {:?}\n  imports: {}\n  delay:   {}",
                path.display(),
                info.machine,
                info.imports.join(" "),
                info.delay_imports.join(" ")
            ),
            Err(e) => println!("{}: {e:#}", path.display()),
        }
        println!(
            "  version: {:?}",
            bigame_core::graphics::pe::read_file_version(path)
        );
    }
}
