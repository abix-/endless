// Build script - runs at compile time
use std::process::Command;

fn main() {
    // Get current timestamp in Eastern time for dev/release labeling.
    let timestamp = if cfg!(windows) {
        Command::new("powershell")
            .args([
                "-Command",
                "$tz=[System.TimeZoneInfo]::FindSystemTimeZoneById('Eastern Standard Time');\
                 [System.TimeZoneInfo]::ConvertTimeFromUtc((Get-Date).ToUniversalTime(),$tz).ToString('yyyy-MM-dd HH:mm:ss')",
            ])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_else(|| "unknown".to_string())
    } else {
        Command::new("date")
            .env("TZ", "America/New_York")
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

    // No rerun-if-changed = rerun on any package file change (always fresh timestamp)
}
