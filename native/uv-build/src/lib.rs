//! Shared, reproducible build support for the editor and its native worker.
mod bundle;
pub use bundle::build as bundle;
use std::{env,fs,path::PathBuf};
pub fn kernel() {
    use sha2::{Digest,Sha256};
    let root=PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out=PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let archive=root.join("vendor/headers.zip");
    let bytes=fs::read(&archive).expect("vendored native UV headers");
    let hash=format!("{:x}",Sha256::digest(&bytes));
    let manifest:serde_json::Value=serde_json::from_slice(&fs::read(root.join("vendor/dependencies.json")).expect("native dependency manifest")).unwrap();
    assert_eq!(manifest["headers_sha256"].as_str().unwrap(),hash,"Native UV header archive checksum");
    let headers=out.join(format!("headers-{}",&hash[..16]));
    if !headers.join(".complete").is_file(){
        fs::create_dir_all(&headers).unwrap();
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap().extract(&headers).expect("extract native headers");
        fs::write(headers.join(".complete"),&hash).unwrap();
    }
    let p=root.join("vendor/partuv/src");
    let target=env::var("TARGET").unwrap();
    let msvc=env::var("CARGO_CFG_TARGET_ENV").as_deref()==Ok("msvc");
    let mut c=cc::Build::new();
    c.cpp(true).std("c++17").warnings(false).opt_level(2)
        .define("NOMINMAX",None).define("WIN32_LEAN_AND_MEAN",None).define("_USE_MATH_DEFINES",None)
        .define("DISABLE_EASY_PROFILER",None).define("CGAL_DISABLE_GMP",None)
        .include(&p).include(p.join("lscm/src")).include(p.join("triangle_intersection"))
        .include(root.join("vendor/include")).include(headers)
        .file(root.join("cpp/bridge.cpp")).file(root.join("vendor/xatlas.cpp"));
    for name in ["AgglomerativeClustering","Component","Distortion","IO","merge","Pack","pipeline",
        "UnwrapAgg","UnwrapBB","UnwrapMerge","UnwrapOne","UnwrapPlane"]{
        c.file(p.join(format!("{name}.cpp")));
    }
    for name in ["Boundary","Edge","Face","FormTrait","LSCM","HalfEdge","Homology","Mesh","Point","Vertex","Graph"]{
        c.file(p.join(format!("lscm/src/{name}.cpp")));
    }
    c.file(p.join("triangle_intersection/triangleHelper.cpp"));
    if msvc {
        c.flag("/bigobj").flag("/EHsc").flag("/openmp").flag("/permissive-").flag("/Zc:__cplusplus");
    } else {c.flag("-fopenmp");}
    let tool=c.get_compiler();
    let mut runtime=Vec::<PathBuf>::new();
    if msvc {
        assert!(!tool.is_like_clang(),"Windows UV build requires MSVC cl.exe; use CXX=cl.exe");
        let vc=tool.path().ancestors().find(|p|p.file_name().is_some_and(|n|n=="VC")).expect("Visual C++ tools directory");
        let arch=match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str(){"x86"=>"x86","aarch64"=>"arm64",_=>"x64"};
        let mut versions:Vec<_>=fs::read_dir(vc.join("Redist/MSVC")).expect("Visual C++ redistributables").filter_map(Result::ok).map(|e|e.path()).collect();versions.sort();
        let mut dll=None;
        for version in versions.into_iter().rev(){
            if let Ok(entries)=fs::read_dir(version.join(arch)){for entry in entries.filter_map(Result::ok){let candidate=entry.path().join("vcomp140.dll");if candidate.is_file(){dll=Some(candidate);break;}}}
            if dll.is_some(){break;}
        }
        runtime.push(dll.unwrap_or_else(||panic!("OpenMP redistributable missing for {target}")));
    }
    fs::write(out.join("native-runtime.json"),serde_json::to_vec(&runtime).unwrap()).unwrap();
    c.compile("bevy_partuv");
    println!("cargo:rustc-link-lib={}",if msvc {"vcomp"}else{"gomp"});
    println!("cargo:rerun-if-changed=cpp");
    println!("cargo:rerun-if-changed=vendor");
}
