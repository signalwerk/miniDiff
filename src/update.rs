//! Self-update (native).
//!
//! Every tagged release deploys `update.json` to GitHub Pages
//! (https://minidiff.signalwerk.ch/update.json). It names the newest version
//! and the GitHub Release asset to download. On macOS, an app running from a
//! `.app` bundle downloads the zip, verifies its SHA-256, swaps the bundle
//! in place and relaunches. Elsewhere we just link to the release page.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const MANIFEST_URL: &str = "https://minidiff.signalwerk.ch/update.json";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, Deserialize)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub notes_url: Option<String>,
    #[serde(default)]
    pub macos: Option<Asset>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Asset {
    pub url: String,
    pub sha256: String,
}

#[derive(Clone, Debug)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    Available(Manifest),
    Installing(Manifest),
    /// The new bundle is in place; relaunch to finish.
    Installed(PathBuf),
    Failed(String),
}

pub struct Updater {
    state: Arc<Mutex<State>>,
}

impl Updater {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(State::Idle)),
        }
    }

    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }

    fn set(state: &Arc<Mutex<State>>, ctx: &egui::Context, s: State) {
        *state.lock().unwrap() = s;
        ctx.request_repaint();
    }

    pub fn check(&self, ctx: &egui::Context) {
        if matches!(self.state(), State::Checking | State::Installing(_) | State::Installed(_)) {
            return;
        }
        Self::set(&self.state, ctx, State::Checking);
        let (state, ctx) = (self.state.clone(), ctx.clone());
        std::thread::spawn(move || {
            let next = match fetch_manifest() {
                Ok(m) if is_newer(&m.version, CURRENT) => State::Available(m),
                Ok(_) => State::UpToDate,
                Err(e) => State::Failed(e),
            };
            Self::set(&state, &ctx, next);
        });
    }

    /// Whether this process can replace itself (running from a .app bundle on macOS).
    pub fn can_self_install(m: &Manifest) -> bool {
        cfg!(target_os = "macos") && m.macos.is_some() && bundle_path().is_some()
    }

    pub fn install(&self, ctx: &egui::Context, m: Manifest) {
        let (Some(asset), Some(bundle)) = (m.macos.clone(), bundle_path()) else {
            return;
        };
        Self::set(&self.state, ctx, State::Installing(m));
        let (state, ctx) = (self.state.clone(), ctx.clone());
        std::thread::spawn(move || {
            let next = match download(&asset).and_then(|zip| replace_bundle(&bundle, &zip)) {
                Ok(()) => State::Installed(bundle),
                Err(e) => State::Failed(e),
            };
            Self::set(&state, &ctx, next);
        });
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .user_agent(format!("MiniDiff/{CURRENT}"))
        .build()
        .new_agent()
}

fn fetch_manifest() -> Result<Manifest, String> {
    agent()
        .get(MANIFEST_URL)
        .call()
        .map_err(|e| format!("Update check failed: {e}"))?
        .body_mut()
        .read_json::<Manifest>()
        .map_err(|e| format!("Invalid update manifest: {e}"))
}

fn download(asset: &Asset) -> Result<Vec<u8>, String> {
    let bytes = agent()
        .get(&asset.url)
        .call()
        .map_err(|e| format!("Download failed: {e}"))?
        .body_mut()
        .with_config()
        .limit(512 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| format!("Download failed: {e}"))?;
    let digest: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    if !digest.eq_ignore_ascii_case(asset.sha256.trim()) {
        return Err("Downloaded update is corrupt (checksum mismatch).".into());
    }
    Ok(bytes)
}

/// `1.2.10` > `1.2.9`; a leading `v` is ignored.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v')
            .split(['.', '-', '+'])
            .take(3)
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    parse(candidate) > parse(current)
}

/// `/Applications/MiniDiff.app` when running from a bundle.
pub fn bundle_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(Path::to_path_buf)
}

fn replace_bundle(bundle: &Path, zip: &[u8]) -> Result<(), String> {
    let parent = bundle.parent().ok_or("Invalid bundle location")?;
    let pid = std::process::id();
    // Stage next to the bundle so the final rename stays on one volume.
    let staging = parent.join(format!(".MiniDiff-update-{pid}"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)
        .map_err(|e| format!("Cannot write to {}: {e}", parent.display()))?;
    let cleanup = |r: Result<(), String>| {
        let _ = std::fs::remove_dir_all(&staging);
        r
    };

    let zip_path = staging.join("update.zip");
    if let Err(e) = std::fs::write(&zip_path, zip) {
        return cleanup(Err(format!("Cannot write update: {e}")));
    }
    let status = std::process::Command::new("/usr/bin/ditto")
        .args(["-x", "-k"])
        .arg(&zip_path)
        .arg(&staging)
        .status();
    if !status.is_ok_and(|s| s.success()) {
        return cleanup(Err("Could not unpack the update.".into()));
    }
    let new_app = staging.join("MiniDiff.app");
    if !new_app.join("Contents/MacOS/minidiff").exists() {
        return cleanup(Err("The update does not contain MiniDiff.app.".into()));
    }

    let backup = parent.join(format!(".MiniDiff-old-{pid}.app"));
    if let Err(e) = std::fs::rename(bundle, &backup) {
        return cleanup(Err(format!("Cannot replace {}: {e}", bundle.display())));
    }
    if let Err(e) = std::fs::rename(&new_app, bundle) {
        let _ = std::fs::rename(&backup, bundle);
        return cleanup(Err(format!("Cannot install update: {e}")));
    }
    let _ = std::fs::remove_dir_all(&backup);
    cleanup(Ok(()))
}

/// Start the freshly installed bundle; the caller then closes this window.
pub fn relaunch(bundle: &Path) {
    let _ = std::process::Command::new("/usr/bin/open").arg("-n").arg(bundle).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn swaps_bundle() {
        let root = std::env::temp_dir().join(format!("minidiff-update-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let make = |dir: &Path, marker: &str| {
            let macos = dir.join("MiniDiff.app/Contents/MacOS");
            std::fs::create_dir_all(&macos).unwrap();
            std::fs::write(macos.join("minidiff"), marker).unwrap();
        };
        let installed = root.join("Applications");
        let release = root.join("release");
        make(&installed, "old");
        make(&release, "new");
        let zip = root.join("update.zip");
        let ok = std::process::Command::new("/usr/bin/ditto")
            .args(["-c", "-k", "--keepParent"])
            .arg(release.join("MiniDiff.app"))
            .arg(&zip)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let bundle = installed.join("MiniDiff.app");
        replace_bundle(&bundle, &std::fs::read(&zip).unwrap()).unwrap();
        assert_eq!(std::fs::read_to_string(bundle.join("Contents/MacOS/minidiff")).unwrap(), "new");
        // No staging or backup leftovers.
        assert_eq!(std::fs::read_dir(&installed).unwrap().count(), 1);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("v1.10.0", "1.9.3"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
    }
}
