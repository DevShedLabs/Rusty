use anyhow::Result;

pub struct Clipboard;

impl Clipboard {
    pub fn get() -> Result<String> {
        // TODO: platform-specific implementation
        // macOS: pbpaste, Linux: xclip/wl-paste, Windows: GetClipboardData
        anyhow::bail!("clipboard not yet implemented")
    }

    pub fn set(text: &str) -> Result<()> {
        let _ = text;
        anyhow::bail!("clipboard not yet implemented")
    }
}
