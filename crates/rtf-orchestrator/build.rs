fn main() {
    println!("cargo:rerun-if-env-changed=GIT_SHA");

    let sha = std::env::var("GIT_SHA").unwrap_or_else(|_| {
        std::process::Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "unknown".to_string())
    });

    println!("cargo:rustc-env=GIT_SHA={sha}");
}
