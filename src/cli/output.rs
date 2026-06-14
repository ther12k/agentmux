/// Format the version output string.
///
/// ```text
/// agentmux 0.1.0-alpha
/// build: release
/// target: x86_64-unknown-linux-gnu
/// ```
pub fn version_string() -> String {
    let build = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    // Construct a reasonable target triple from std constants.
    let abi = match std::env::consts::OS {
        "linux" => "linux-gnu",
        other => other,
    };
    let target = format!("{}-unknown-{}", std::env::consts::ARCH, abi);
    format!(
        "agentmux {}\nbuild: {}\ntarget: {}",
        env!("CARGO_PKG_VERSION"),
        build,
        target
    )
}
