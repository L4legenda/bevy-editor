//! Build and bundle the independent native worker; no setup or network at runtime.
use std::{
    env, fs,
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Command, Stdio},
};
pub fn build() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    for path in [
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
        "src",
        "cpp",
        "vendor",
    ] {
        println!("cargo:rerun-if-changed=native/uv/{path}");
    }
    println!("cargo:rerun-if-env-changed=BEVY_EDITOR_UV_NATIVE_DIR");
    let target = env::var("TARGET").unwrap();
    let (executable, runtimes) = if let Some(folder) = env::var_os("BEVY_EDITOR_UV_NATIVE_DIR") {
        let folder = PathBuf::from(folder);
        let executable = folder.join(if target.contains("windows") {
            "bevy-uv-native.exe"
        } else {
            "bevy-uv-native"
        });
        let dlls = fs::read_dir(&folder)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "dll"))
            .collect::<Vec<_>>();
        (executable, dlls)
    } else {
        let mut command = Command::new(env::var_os("CARGO").unwrap());
        // This independent lockfile can require crates the editor build did not fetch.
        // Keep versions locked while allowing Cargo's normal network configuration.
        command
            .args([
                "build",
                "--release",
                "--locked",
                "--message-format=json",
                "--manifest-path",
            ])
            .arg(root.join("native/uv/Cargo.toml"))
            .arg("--target-dir")
            .arg(root.join("target/uv-native-build"))
            .arg("--target")
            .arg(&target)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS");
        if target.contains("windows-msvc") {
            command.env(
                "CARGO_ENCODED_RUSTFLAGS",
                "-C\u{1f}target-feature=+crt-static",
            );
        }
        let mut child = command.spawn().expect("start native UV build");
        let mut executable = None;
        let mut runtime = None;
        for line in BufReader::new(child.stdout.take().unwrap()).lines() {
            let line = line.expect("native build output");
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            if v["reason"] == "compiler-artifact" && v["target"]["name"] == "bevy-uv-native" {
                if let Some(path) = v["executable"].as_str() {
                    executable = Some(PathBuf::from(path));
                }
            }
            if v["reason"] == "build-script-executed"
                && v["package_id"]
                    .as_str()
                    .is_some_and(|p| p.contains("bevy-uv-native"))
            {
                runtime = v["out_dir"]
                    .as_str()
                    .map(|p| PathBuf::from(p).join("native-runtime.json"));
            }
            if v["reason"] == "compiler-message" {
                if let Some(message) = v["message"]["rendered"].as_str() {
                    eprint!("{message}");
                }
            }
        }
        assert!(
            child.wait().expect("native build status").success(),
            "Native UV worker build failed"
        );
        let runtimes: Vec<PathBuf> = runtime
            .map(|p| {
                serde_json::from_slice(&fs::read(p).expect("native runtime manifest"))
                    .expect("native runtime manifest JSON")
            })
            .unwrap_or_default();
        (executable.expect("native UV executable artifact"), runtimes)
    };
    let bundle = out.join("uv-native");
    fs::create_dir_all(&bundle).unwrap();
    let mut files = vec![executable];
    files.extend(runtimes);
    let mut code = String::from("const NATIVE_BUNDLE: &[(&str, &[u8])] = &[\n");
    for source in files {
        let name = source.file_name().unwrap().to_str().unwrap();
        let path = bundle.join(name);
        fs::copy(&source, &path)
            .unwrap_or_else(|e| panic!("bundle native UV component {}: {e}", source.display()));
        code.push_str(&format!(
            "({name:?}, include_bytes!({:?})),\n",
            path.to_str().unwrap()
        ));
    }
    code.push_str("];\n");
    fs::write(out.join("uv_native_bundle.rs"), code).unwrap();
}
