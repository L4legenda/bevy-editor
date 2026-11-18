fn main() {
    if std::env::var_os("CARGO_FEATURE_EDITOR").is_some() {
        bevy_uv_build::bundle();
        configure_macos_debug_linker();
    }
    println!("cargo:rerun-if-changed=assets/icons/bevy-editor.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || std::env::var_os("CARGO_FEATURE_EDITOR").is_none()
    {
        return;
    }

    let icon = std::path::Path::new(&std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("assets/icons/bevy-editor.ico");
    let resource =
        std::path::Path::new(&std::env::var_os("OUT_DIR").unwrap()).join("bevy-editor-icon.rc");
    std::fs::write(
        &resource,
        format!(
            "#pragma code_page(65001)\n1 ICON \"{}\"\n",
            icon.to_string_lossy().replace('\\', "/")
        ),
    )
    .expect("write application icon resource");
    embed_resource::compile_for(&resource, ["bevy-editor"], embed_resource::NONE)
        .manifest_required()
        .expect("embed application icon into bevy-editor.exe");
}

fn configure_macos_debug_linker() {
    use std::{env, process::Command};

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos")
        || env::var("PROFILE").as_deref() != Ok("debug")
        || env::var_os("RUSTC_LINKER").is_some()
    {
        return;
    }

    // The large debug binary can exceed the compact table's 24-bit DWARF offset.
    // Silence just that warning on Apple linkers that support the option.
    const FLAG: &str = "-no_warn_eh_frame_too_large";
    println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");
    if Command::new("xcrun")
        .args(["ld", FLAG, "-v"])
        .output()
        .is_ok_and(|output| output.status.success())
    {
        println!("cargo:rustc-link-arg-bin=bevy-editor=-Wl,{FLAG}");
    }
}
