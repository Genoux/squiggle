//! Signs a release bundle for the updater, or makes the key pair once.
//!
//!   sign <file>            writes <file>.sig, from UPDATER_SIGNING_KEY and
//!                          UPDATER_SIGNING_KEY_PASSWORD
//!   sign --generate <dir>  writes updater.key and updater.pub in <dir>,
//!                          encrypted with UPDATER_SIGNING_KEY_PASSWORD

use anyhow::{Context, Result, bail};
use minisign::{KeyPair, SecretKeyBox};
use std::{env, fs, io::Cursor, path::Path};

fn password() -> Result<String> {
    env::var("UPDATER_SIGNING_KEY_PASSWORD").context("UPDATER_SIGNING_KEY_PASSWORD is not set")
}

fn generate(dir: &Path) -> Result<()> {
    let pair = KeyPair::generate_encrypted_keypair(Some(password()?))?;
    fs::write(dir.join("updater.key"), pair.sk.to_box(None)?.to_string())?;
    fs::write(dir.join("updater.pub"), pair.pk.to_box()?.to_string())?;
    Ok(())
}

fn sign(file: &Path) -> Result<()> {
    let key = env::var("UPDATER_SIGNING_KEY").context("UPDATER_SIGNING_KEY is not set")?;
    let key = SecretKeyBox::from_string(&key)?.into_secret_key(Some(password()?))?;
    let name = file.file_name().context("not a file")?.to_string_lossy();
    let signature = minisign::sign(None, &key, Cursor::new(fs::read(file)?), Some(&name), None)?;
    fs::write(format!("{}.sig", file.display()), signature.to_string())?;
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["--generate", dir] => generate(Path::new(dir)),
        [file] => sign(Path::new(file)),
        _ => bail!("usage: sign <file> | sign --generate <dir>"),
    }
}
