//! Fetching an update and installing it. What an update is, and whether to
//! trust it, is `squiggle_core::update`.
//!
//! Installing is running the bundle's own `packaging/install.sh`: the
//! installer stays the only place that knows how Squiggle is installed, and
//! an update lands exactly as a first install does.

use anyhow::{Context, Result, ensure};
use squiggle_core::update::{self, Release};
use std::{
    fs,
    io::Write,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Checked at start and then every few hours: a tray app can run for weeks
/// between logins.
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// After a check that failed: at login the network is often not up yet.
const RETRY: Duration = Duration::from_secs(15 * 60);

fn fetch(url: &str) -> Result<Vec<u8>> {
    let output = Command::new("curl")
        .args(["--silent", "--show-error", "--fail", "--location"])
        .args(["--proto", "=https", "--proto-redir", "=https"])
        .args(["--max-time", "300", url])
        .output()
        .context("curl is not installed")?;
    ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(output.stdout)
}

/// The release on offer, if it is newer than this build.
pub fn check() -> Result<Option<Release>> {
    let feed = fetch(update::FEED)?;
    update::offered(
        &String::from_utf8_lossy(&feed),
        VERSION,
        &update::platform(),
    )
}

/// A private folder for the unpacked bundle. Under the runtime directory on
/// Linux, which the session clears at logout.
fn scratch() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!("squiggle-update-{}", std::process::id()));
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    fs::DirBuilder::new().mode(0o700).create(&dir)?;
    Ok(dir)
}

/// Unpacks the verified bytes themselves, never a file read back from disk.
fn unpack(bundle: &[u8], into: &Path) -> Result<PathBuf> {
    let mut tar = Command::new("tar")
        .args(["-xzf", "-", "-C"])
        .arg(into)
        .stdin(Stdio::piped())
        .spawn()
        .context("tar is not installed")?;
    tar.stdin
        .take()
        .context("tar has no input")?
        .write_all(bundle)?;
    ensure!(tar.wait()?.success(), "the update could not be unpacked");
    let mut entries = fs::read_dir(into)?.collect::<std::io::Result<Vec<_>>>()?;
    ensure!(entries.len() == 1, "the update is not laid out as expected");
    Ok(entries.remove(0).path())
}

/// Started outside this process's unit or process group: the installer
/// stops Squiggle, and would otherwise be stopped with it. Waited for: the
/// running app is stopped while it waits, so coming back at all means the
/// installer is over, and `squiggle update` learns how it went.
#[cfg(target_os = "linux")]
fn run_installer(script: &Path) -> Result<()> {
    let unit = format!("squiggle-update-{}", std::process::id());
    let status = Command::new("systemd-run")
        .args([
            "--user",
            "--collect",
            "--quiet",
            "--wait",
            "--unit",
            &unit,
            "bash",
        ])
        .arg(script)
        .status()
        .context("systemd-run is not installed")?;
    ensure!(
        status.success(),
        "the installer failed; see `journalctl --user -u {unit}`"
    );
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_installer(script: &Path) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let status = Command::new("bash")
        .arg(script)
        .stdin(Stdio::null())
        .process_group(0)
        .status()
        .context("the installer could not be started")?;
    ensure!(
        status.success(),
        "the installer failed; its output is in the log"
    );
    Ok(())
}

/// Downloads `release`, and installs it only if Squiggle's key signed it and
/// it is the version the feed said. Returns when the installer is done; in
/// the running app it does not return, since the installer restarts it.
pub fn install(release: &Release) -> Result<()> {
    let bundle = fetch(&release.url)?;
    update::verify(&bundle, &release.signature)?;
    let root = unpack(&bundle, &scratch()?)?;
    // A signed bundle of an older version, offered as a newer one, would
    // otherwise be a way to put a fixed bug back.
    let reported = Command::new(root.join("bin/squiggle"))
        .arg("--version")
        .output()?;
    ensure!(
        String::from_utf8_lossy(&reported.stdout).trim() == format!("squiggle {}", release.version),
        "the update is not the version it says it is"
    );
    run_installer(&root.join("packaging/install.sh"))
}

/// One install at a time: a second click would empty the folder the first
/// one's installer is running from, after it has stopped Squiggle.
static INSTALLING: AtomicBool = AtomicBool::new(false);

/// Installs in the background, from a menu click.
pub fn install_from_menu(release: Release) {
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return;
    }
    thread::spawn(move || {
        // Reached only when the installer is over and Squiggle was not
        // replaced, so the menu's Update works again.
        if let Err(err) = install(&release) {
            note!("update not installed: {err:#}");
        }
        INSTALLING.store(false, Ordering::SeqCst);
    });
}

/// Sends each newer release found, at start and every few hours after. A
/// pre-release build is sent nothing; the menu says so.
pub fn watch() -> mpsc::Receiver<Release> {
    let (found, releases) = mpsc::channel();
    if !update::follows_releases(VERSION) {
        return releases;
    }
    thread::spawn(move || {
        loop {
            let wait = match check() {
                Ok(Some(release)) => {
                    if found.send(release).is_err() {
                        return;
                    }
                    EVERY
                }
                Ok(None) => EVERY,
                Err(err) => {
                    note!("update check: {err:#}");
                    RETRY
                }
            };
            thread::sleep(wait);
        }
    });
    releases
}

/// `squiggle update`.
pub fn run() -> Result<()> {
    if !update::follows_releases(VERSION) {
        println!("Squiggle {VERSION} is a pre-release: updates are not offered.");
        return Ok(());
    }
    match check()? {
        None => println!("Squiggle {VERSION} is the latest."),
        Some(release) => {
            println!("Installing Squiggle {}…", release.version);
            install(&release)?;
            println!("Squiggle {} is installed and running.", release.version);
        }
    }
    Ok(())
}
