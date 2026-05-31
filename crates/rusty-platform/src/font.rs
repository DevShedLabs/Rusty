/// Discover system font paths for a given family name.
/// Returns paths to try in order.
pub fn discover(family: &str) -> Vec<std::path::PathBuf> {
    let _ = family;
    // TODO: fontconfig on Linux, CoreText on macOS, DirectWrite on Windows
    vec![]
}
