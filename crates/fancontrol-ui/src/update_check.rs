//! Manual "check for updates" against the GitHub Releases API.
//!
//! Check-only: shows a banner with a link to the release page. Never downloads or
//! installs anything automatically (see `specs/01-product.md` / `docs/SECURITY.md`).

use serde::Deserialize;
use std::sync::{Arc, Mutex};
use std::thread;

const REPO: &str = "Leyukaka/fancontrol-rs";

#[derive(Debug, Clone)]
pub enum UpdateStatus {
    Checking,
    UpToDate,
    Available { version: String, url: String },
    Error(String),
}

#[derive(Deserialize)]
struct ReleaseResponse {
    tag_name: String,
    html_url: String,
}

#[derive(Default, Clone)]
pub struct UpdateChecker {
    state: Arc<Mutex<Option<UpdateStatus>>>,
}

impl UpdateChecker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn status(&self) -> Option<UpdateStatus> {
        self.state.lock().ok().and_then(|g| g.clone())
    }

    /// Kick off a one-shot background check. Safe to call repeatedly (e.g. on every
    /// button click) - this is not a polling loop.
    pub fn check_now(&self) {
        {
            let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
            *g = Some(UpdateStatus::Checking);
        }
        let state = Arc::clone(&self.state);
        thread::Builder::new()
            .name("fancontrol-update-check".into())
            .spawn(move || {
                let status = match fetch_latest_release() {
                    Ok(release) => compare_versions(&release),
                    Err(e) => UpdateStatus::Error(e),
                };
                if let Ok(mut g) = state.lock() {
                    *g = Some(status);
                }
            })
            .ok();
    }
}

fn fetch_latest_release() -> Result<ReleaseResponse, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let mut resp = ureq::get(&url)
        .header("User-Agent", "fancontrol-rs-update-check")
        .call()
        .map_err(|e| format!("request failed: {e}"))?;
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("read failed: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("parse failed: {e}"))
}

fn compare_versions(release: &ReleaseResponse) -> UpdateStatus {
    let current = env!("CARGO_PKG_VERSION");
    let latest = release.tag_name.trim_start_matches('v');
    let Some(latest_v) = parse_version(latest) else {
        return UpdateStatus::Error(format!(
            "could not parse release tag '{}'",
            release.tag_name
        ));
    };
    let current_v = parse_version(current).unwrap_or((0, 0, 0));
    if latest_v > current_v {
        UpdateStatus::Available {
            version: release.tag_name.clone(),
            url: release.html_url.clone(),
        }
    } else {
        UpdateStatus::UpToDate
    }
}

fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let mut parts = s.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_version() {
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("bogus"), None);
    }

    #[test]
    fn detects_newer_release() {
        let release = ReleaseResponse {
            tag_name: "v99.0.0".into(),
            html_url: "https://example.invalid/releases/v99.0.0".into(),
        };
        match compare_versions(&release) {
            UpdateStatus::Available { version, .. } => assert_eq!(version, "v99.0.0"),
            other => panic!("expected Available, got {other:?}"),
        }
    }
}
