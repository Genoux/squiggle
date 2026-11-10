//! What an update is and whether to trust it. Fetching and installing are
//! the app's; this is the part with no network or disk in it, so it is
//! tested on every platform.
//!
//! The feed is Tauri's `latest.json`, one format for every app the toolkit
//! covers. A signature is minisign's, as Tauri's are; the feed holds the
//! `.sig` file's text as it is.

use anyhow::{Context, Result, bail, ensure};
use minisign_verify::{PublicKey, Signature};
use serde::Deserialize;
use std::collections::HashMap;

const RELEASES: &str = "https://github.com/Genoux/squiggle/releases/";
pub const FEED: &str = "https://github.com/Genoux/squiggle/releases/latest/download/latest.json";
/// The private half is only in the repository's Actions secrets. A bundle
/// this key did not sign is never unpacked.
const PUBLIC_KEY: &str = include_str!("../../../packaging/updater.pub");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub signature: String,
}

#[derive(Deserialize)]
struct Feed {
    version: String,
    platforms: HashMap<String, Bundle>,
}

#[derive(Deserialize)]
struct Bundle {
    url: String,
    signature: String,
}

/// This build's key in the feed: `darwin-aarch64`, `linux-x86_64`.
pub fn platform() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

fn numbers(version: &str) -> Option<([u64; 3], bool)> {
    let (core, pre) = match version.split_once('-') {
        Some((core, _)) => (core, true),
        None => (version, false),
    };
    let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
    let numbers = [parts.next()??, parts.next()??, parts.next()??];
    parts.next().is_none().then_some((numbers, pre))
}

/// Whether `candidate` is a later version than `running`. A version that
/// cannot be read is never newer.
pub fn newer(candidate: &str, running: &str) -> bool {
    match (numbers(candidate), numbers(running)) {
        (Some((candidate, candidate_pre)), Some((running, running_pre))) => {
            candidate > running || (candidate == running && running_pre && !candidate_pre)
        }
        _ => false,
    }
}

/// The release the feed offers a build of `running` on `platform`, if it is
/// newer and has a bundle for it.
pub fn offered(feed: &str, running: &str, platform: &str) -> Result<Option<Release>> {
    let feed: Feed =
        serde_json::from_str(feed).context("the update feed is not what was expected")?;
    let version = feed.version.trim_start_matches('v');
    let Some(bundle) = feed.platforms.get(platform) else {
        return Ok(None);
    };
    if !newer(version, running) {
        return Ok(None);
    }
    // The signature is what makes a bundle trusted; this only keeps a
    // tampered feed from sending the download somewhere else.
    ensure!(
        bundle.url.starts_with(RELEASES) && !bundle.url.contains(".."),
        "the update is not one of Squiggle's releases: {}",
        bundle.url
    );
    Ok(Some(Release {
        version: version.to_string(),
        url: bundle.url.clone(),
        signature: bundle.signature.clone(),
    }))
}

fn verify_with(public_key: &str, bundle: &[u8], signature: &str) -> Result<()> {
    let key = PublicKey::decode(public_key).context("the built-in update key is unreadable")?;
    let signature = Signature::decode(signature).context("the update's signature is unreadable")?;
    if key.verify(bundle, &signature, false).is_err() {
        bail!("the update's signature does not match");
    }
    Ok(())
}

/// Fails unless `bundle` was signed with Squiggle's update key.
pub fn verify(bundle: &[u8], signature: &str) -> Result<()> {
    verify_with(PUBLIC_KEY, bundle, signature)
}

#[cfg(test)]
mod tests {
    use super::*;
    use minisign::KeyPair;
    use std::io::Cursor;

    const BUNDLE: &str = "https://github.com/Genoux/squiggle/releases/download/v0.6.0/squiggle-v0.6.0-linux-x86_64.tar.gz";

    fn feed(version: &str, url: &str) -> String {
        format!(
            r#"{{"version":"{version}","notes":"","pub_date":"2026-11-10T00:00:00Z",
                "platforms":{{"linux-x86_64":{{"url":"{url}","signature":"sig"}}}}}}"#
        )
    }

    fn signed(pair: &KeyPair, bundle: &[u8]) -> String {
        minisign::sign(None, &pair.sk, Cursor::new(bundle), None, None)
            .unwrap()
            .to_string()
    }

    #[test]
    fn a_later_version_is_newer_and_nothing_else_is() {
        assert!(newer("0.6.0", "0.5.0"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(newer("1.0.0", "1.0.0-rc.1"));
        assert!(!newer("0.5.0", "0.5.0"));
        assert!(!newer("0.4.9", "0.5.0"));
        assert!(!newer("1.0.0-rc.1", "1.0.0"));
        assert!(!newer("latest", "0.5.0"));
        assert!(!newer("0.6", "0.5.0"));
    }

    #[test]
    fn a_newer_release_for_this_platform_is_offered() {
        let release = offered(&feed("0.6.0", BUNDLE), "0.5.0", "linux-x86_64").unwrap();
        assert_eq!(
            release,
            Some(Release {
                version: "0.6.0".into(),
                url: BUNDLE.into(),
                signature: "sig".into(),
            })
        );
    }

    #[test]
    fn the_running_version_or_another_platform_is_not_offered() {
        assert_eq!(
            offered(&feed("0.5.0", BUNDLE), "0.5.0", "linux-x86_64").unwrap(),
            None
        );
        assert_eq!(
            offered(&feed("0.6.0", BUNDLE), "0.5.0", "darwin-aarch64").unwrap(),
            None
        );
    }

    #[test]
    fn a_bundle_from_elsewhere_is_refused() {
        let elsewhere = feed("0.6.0", "https://example.com/squiggle.tar.gz");
        assert!(offered(&elsewhere, "0.5.0", "linux-x86_64").is_err());
        let climbing = feed(
            "0.6.0",
            "https://github.com/Genoux/squiggle/releases/../../other/repo/squiggle.tar.gz",
        );
        assert!(offered(&climbing, "0.5.0", "linux-x86_64").is_err());
        assert!(offered("<html>", "0.5.0", "linux-x86_64").is_err());
    }

    #[test]
    fn only_a_bundle_the_key_signed_is_accepted() {
        let pair = KeyPair::generate_unencrypted_keypair().unwrap();
        let other = KeyPair::generate_unencrypted_keypair().unwrap();
        let public_key = pair.pk.to_box().unwrap().to_string();
        let bundle = b"the release bundle";

        assert!(verify_with(&public_key, bundle, &signed(&pair, bundle)).is_ok());
        assert!(verify_with(&public_key, b"the release bundlE", &signed(&pair, bundle)).is_err());
        assert!(verify_with(&public_key, bundle, &signed(&other, bundle)).is_err());
        assert!(verify_with(&public_key, bundle, "").is_err());
    }

    #[test]
    fn the_built_in_key_is_readable_and_refuses_a_stranger() {
        let stranger = KeyPair::generate_unencrypted_keypair().unwrap();
        let error = verify(b"bundle", &signed(&stranger, b"bundle")).unwrap_err();
        assert_eq!(error.to_string(), "the update's signature does not match");
    }
}
