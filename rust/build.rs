// Build script - runs at compile time
use std::process::Command;

fn main() {
    // Get current timestamp (works on Windows via PowerShell)
    let timestamp = if cfg!(windows) {
        Command::new("powershell")
            .args(["-Command", "Get-Date -Format 'yyyy-MM-dd HH:mm:ss'"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_else(|| "unknown".to_string())
    } else {
        Command::new("date")
            .args(["+%Y-%m-%d %H:%M:%S"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_else(|| "unknown".to_string())
    };
    let timestamp = timestamp.trim();

    // Get git commit hash (short)
    let commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| "unknown".to_string());
    let commit = commit.trim();

    // Pass to compiler as environment variables
    println!("cargo:rustc-env=BUILD_TIMESTAMP={}", timestamp);
    println!("cargo:rustc-env=BUILD_COMMIT={}", commit);

    // Rebuild if git HEAD changes or source changes
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=src/");
}
