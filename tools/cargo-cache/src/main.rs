fn main() {
    let code = match bevy_cargo_cache::main_args(std::env::args_os().skip(1)) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("Build cache: {error}");
            1
        }
    };
    std::process::exit(code);
}
