use std::process::Command;

fn main() {
    // BUILD_TS: compile timestamp for GET /version (set at every cargo build).
    let ts = Command::new("date")
        .arg("-u")
        .arg("+%Y-%m-%dT%H:%M:%SZ")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=BUILD_TS={ts}");
    println!("cargo:rerun-if-changed=build.rs");
}
