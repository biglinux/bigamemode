use std::path::Path;
use std::process::Command;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let root = Path::new(&manifest_dir)
        .parent()
        .and_then(Path::parent)
        .expect("repository root");

    let gresource_xml = root.join("data/resources.gresource.xml");
    let output = format!("{}/resources.gresource", std::env::var("OUT_DIR").unwrap());

    // The bundle lists style.css and icons/hicolor/…; each is found in one of
    // these directories, so nothing has to be staged or copied first.
    let sources = [root.join("data"), root.join("style"), root.join("usr/share")];

    let mut command = Command::new("glib-compile-resources");
    for dir in &sources {
        command.arg("--sourcedir").arg(dir);
    }
    let status = command
        .arg("--target")
        .arg(&output)
        .arg(&gresource_xml)
        .status()
        .expect("glib-compile-resources (from glib2) must be installed");
    assert!(status.success(), "glib-compile-resources failed");

    println!("cargo::rerun-if-changed={}", gresource_xml.display());
    println!("cargo::rerun-if-changed={}", root.join("style").display());
    println!(
        "cargo::rerun-if-changed={}",
        root.join("usr/share/icons/hicolor/scalable/apps").display()
    );
}
