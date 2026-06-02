fn main() {
    if cfg!(all(target_os = "windows", target_env = "msvc")) {
        println!("cargo:rustc-link-arg=/STACK:8388608");
    }
}
