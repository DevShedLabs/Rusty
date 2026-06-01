use anyhow::{bail, Context, Result};
use std::io::Write;

const REPO: &str = "DevShedLabs/Rusty";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug)]
pub enum UpdateStatus {
    AlreadyLatest,
    Updated { from: String, to: String },
}

/// Check GitHub releases and return the latest tag, or None if already current.
pub fn check() -> Result<Option<String>> {
    let release = latest_release()?;
    let tag = release["tag_name"]
        .as_str()
        .context("missing tag_name")?
        .to_owned();
    let remote = tag.trim_start_matches('v');
    if remote == CURRENT {
        Ok(None)
    } else {
        Ok(Some(tag))
    }
}

/// Download and install the latest release binary. Replaces the running binary atomically.
pub fn install() -> Result<UpdateStatus> {
    let release = latest_release()?;
    let tag = release["tag_name"]
        .as_str()
        .context("missing tag_name")?
        .to_owned();
    let remote = tag.trim_start_matches('v');

    if remote == CURRENT {
        return Ok(UpdateStatus::AlreadyLatest);
    }

    let assets = release["assets"]
        .as_array()
        .context("missing assets")?;

    let asset = assets
        .iter()
        .find(|a| {
            a["name"]
                .as_str()
                .map(|n| n.contains("macos") || n.contains("darwin") || n == "rusty")
                .unwrap_or(false)
        })
        .or_else(|| assets.first())
        .context(
            "No binary asset found in this release.\n\
             Attach a compiled macOS binary to the GitHub release and try again.",
        )?;

    let url = asset["browser_download_url"]
        .as_str()
        .context("missing download URL")?;

    let current_exe = std::env::current_exe()?;
    let parent = current_exe.parent().context("no parent dir")?;
    let tmp = parent.join(".rusty-update.tmp");

    {
        let resp = ureq::get(url)
            .set("User-Agent", &format!("rusty/{CURRENT}"))
            .call()
            .context("download failed")?;
        let mut file = std::fs::File::create(&tmp)?;
        std::io::copy(&mut resp.into_reader(), &mut file)?;
        file.flush()?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&tmp)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&tmp, perms)?;
    }

    let old = parent.join(".rusty.old");
    std::fs::rename(&current_exe, &old)?;
    std::fs::rename(&tmp, &current_exe)?;

    Ok(UpdateStatus::Updated {
        from: CURRENT.to_owned(),
        to: tag,
    })
}

fn latest_release() -> Result<serde_json::Value> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let resp = ureq::get(&url)
        .set("User-Agent", &format!("rusty/{CURRENT}"))
        .set("Accept", "application/vnd.github+json")
        .call();

    match resp {
        Ok(r) => {
            let json: serde_json::Value = r.into_json()?;
            if json["message"].as_str() == Some("Not Found") {
                bail!(
                    "No releases found for {REPO}.\n\
                     Create a GitHub release and attach a binary asset."
                );
            }
            Ok(json)
        }
        Err(ureq::Error::Status(code, _)) => bail!("GitHub API returned HTTP {code}"),
        Err(e) => Err(e).context("network error"),
    }
}
