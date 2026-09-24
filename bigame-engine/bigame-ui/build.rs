use std::process::Command;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let workspace = std::path::Path::new(&manifest_dir)
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root");

    let data_dir = workspace.join("data");
    let style_dir = workspace.join("style");
    let gresource_xml = data_dir.join("resources.gresource.xml");
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let output = format!("{out_dir}/resources.gresource");

    // Stage extra assets in OUT_DIR (never mutate source tree)
    let stage_dir = std::path::Path::new(&out_dir).join("gresource_stage");
    std::fs::create_dir_all(&stage_dir).expect("create stage dir");

    // Copy style.css into stage so glib-compile-resources can find it
    let css_src = style_dir.join("style.css");
    if css_src.exists() {
        std::fs::copy(&css_src, stage_dir.join("style.css")).expect("copy style.css");
    }

    // Copy usr/share/icons/ → stage/icons/ so glib-compile-resources sees
    // icons/hicolor/scalable/apps/... as referenced in resources.gresource.xml
    let icons_src = workspace.join("usr/share/icons");
    if icons_src.exists() {
        let _ = Command::new("cp")
            .args([
                "-r",
                icons_src.to_str().unwrap(),
                stage_dir.join("icons").to_str().unwrap(),
            ])
            .status();
    }

    let status = Command::new("glib-compile-resources")
        .arg("--sourcedir")
        .arg(&data_dir)
        .arg("--sourcedir")
        .arg(&stage_dir)
        .arg("--target")
        .arg(&output)
        .arg(&gresource_xml)
        .status()
        .expect("glib-compile-resources");

    assert!(status.success(), "glib-compile-resources failed");

    println!("cargo::rerun-if-changed={}", gresource_xml.display());
    println!(
        "cargo::rerun-if-changed={}",
        style_dir.join("style.css").display()
    );
    println!(
        "cargo::rerun-if-changed={}",
        data_dir.join("icons/com.biglinux.BiGameMode.svg").display()
    );
    println!(
        "cargo::rerun-if-changed={}",
        workspace
            .join("usr/share/icons/hicolor/scalable/apps/com.biglinux.BiGameMode.svg")
            .display()
    );
}
