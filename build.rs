fn main() {
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
