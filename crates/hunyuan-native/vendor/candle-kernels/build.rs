use std::{env, path::PathBuf};

fn kernels() -> bindgen_cuda::Builder {
    bindgen_cuda::Builder::default()
        .arg("--expt-relaxed-constexpr")
        .arg("-std=c++17")
        .arg("-O3")
}

fn main() {
    println!("cargo::rerun-if-changed=src");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    kernels()
        .kernel_paths_glob("src/*.cu")
        .build_ptx()
        .unwrap()
        .write(out.join("ptx.rs"))
        .unwrap();

    // Candle links these entry points even though Hunyuan does not use MoE.
    // Keep them in a separate native library for Ampere so the tensor PTX can target Pascal.
    let moe = kernels().compute_cap(80).kernel_paths_glob("src/moe/*.cu");
    let msvc = env::var("TARGET").is_ok_and(|target| target.contains("msvc"));
    let moe = if msvc {
        moe.arg("-D_USE_MATH_DEFINES")
    } else {
        moe.arg("-Xcompiler").arg("-fPIC")
    };
    moe.build_lib(out.join("libmoe.a"));
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rustc-link-lib=moe");
    println!("cargo:rustc-link-lib=dylib=cudart");
    if !msvc {
        println!("cargo:rustc-link-lib=stdc++");
    }
}
