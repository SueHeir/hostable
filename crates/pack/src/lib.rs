//! Hostable game packs: build, sign, check and load `.gamepack` files.
//!
//! A pack is a zip holding `manifest.json`, optional `rules.wasm` (or
//! `rules.js` for the QuickJS runtime), optional `table.json`, and assets.
//! Packs come from anyone, so everything here treats them as hostile:
//! sizes are capped while reading, paths can't escape, only known file types
//! are allowed, and the author's ed25519 signature must cover every file.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

use base64::Engine as _;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub mod check;
pub mod load;

pub use check::{check_pack, CheckOptions, Issue, IssueKind, Report};
pub use load::{load_rules, LoadedGame};

/// Largest pack, counting every file uncompressed.
pub const MAX_PACK_BYTES: u64 = 200 * 1024 * 1024;
/// Most files in one pack.
pub const MAX_FILES: usize = 10_000;
/// Where the signature lives inside the zip. Not covered by itself.
pub const SIGNATURE_FILE: &str = "SIGNATURE.json";
pub const MANIFEST_FILE: &str = "manifest.json";

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackError {
    /// Not a zip, or a zip we refuse to read (bad paths, duplicates).
    Unreadable(String),
    TooLarge(u64),
    BadManifest(String),
    ForbiddenFile(String),
    Unsigned,
    BadSignature(String),
    KeyMismatch(String),
}

impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackError::Unreadable(m) => write!(f, "unreadable pack: {m}"),
            PackError::TooLarge(n) => write!(
                f,
                "pack is over the {} MB limit ({} bytes or more)",
                MAX_PACK_BYTES / 1024 / 1024,
                n
            ),
            PackError::BadManifest(m) => write!(f, "bad manifest: {m}"),
            PackError::ForbiddenFile(m) => write!(f, "file type not allowed: {m}"),
            PackError::Unsigned => write!(f, "pack is not signed"),
            PackError::BadSignature(m) => write!(f, "bad signature: {m}"),
            PackError::KeyMismatch(m) => write!(f, "signing key mismatch: {m}"),
        }
    }
}

impl std::error::Error for PackError {}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Assets and a starting table; players enforce the rules themselves.
    Sandbox,
    /// Plus `rules.wasm` or `rules.js`.
    #[default]
    Scripted,
    /// Plus a renderer built into the host (named by `renderer`).
    Native,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Players {
    pub min: u32,
    pub max: u32,
}

/// `manifest.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Lower-case letters, digits and dashes; unique per author key.
    pub id: String,
    pub name: String,
    /// `MAJOR.MINOR.PATCH`.
    pub version: String,
    pub players: Players,
    /// The rules API version the pack was built against.
    pub api: u32,
    #[serde(default)]
    pub level: Level,
    /// `rules.wasm` or `rules.js`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<String>,
    /// `table.json`, the objects on the table at start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    /// Files phones may download (images for cards and choices).
    #[serde(default)]
    pub phone_assets: Vec<String>,
    /// The author's ed25519 public key, base64. Filled in when signing.
    #[serde(default)]
    pub author_key: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    /// For native packs, which built-in renderer draws the table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renderer: Option<String>,
}

impl Manifest {
    pub fn validate(&self, files: &BTreeMap<String, Vec<u8>>) -> Result<(), PackError> {
        let bad = |m: String| Err(PackError::BadManifest(m));
        let id_ok = (2..=40).contains(&self.id.len())
            && self
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && !self.id.starts_with('-');
        if !id_ok {
            return bad(format!("id {:?} must be 2-40 of a-z, 0-9 and -", self.id));
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 60 {
            return bad("name must be 1-60 characters".into());
        }
        let parts: Vec<&str> = self.version.split('.').collect();
        if parts.len() != 3
            || parts
                .iter()
                .any(|p| p.is_empty() || p.parse::<u32>().is_err())
        {
            return bad(format!("version {:?} must look like 1.2.3", self.version));
        }
        if self.players.min == 0 || self.players.min > self.players.max || self.players.max > 64 {
            return bad("players must be 1 <= min <= max <= 64".into());
        }
        if self.api != hostable_rules_api::RULES_API_VERSION {
            return bad(format!(
                "api {} isn't supported (this host speaks {})",
                self.api,
                hostable_rules_api::RULES_API_VERSION
            ));
        }
        if self.description.chars().count() > 2000 || self.author.chars().count() > 100 {
            return bad("description or author too long".into());
        }
        match (&self.level, &self.rules, &self.table) {
            (Level::Sandbox, None, Some(_)) => {}
            (Level::Sandbox, _, _) => return bad("a sandbox pack has a table and no rules".into()),
            (_, Some(r), _) if r == "rules.wasm" || r == "rules.js" => {}
            (_, Some(r), _) => {
                return bad(format!("rules must be rules.wasm or rules.js, not {r}"))
            }
            (_, None, _) => return bad("a scripted or native pack needs rules".into()),
        }
        if self.level == Level::Native && self.renderer.is_none() {
            return bad("a native pack names its renderer".into());
        }
        for name in self.rules.iter().chain(self.table.iter()) {
            if !files.contains_key(name) {
                return bad(format!("{name} is listed but missing"));
            }
        }
        for asset in &self.phone_assets {
            if !asset.starts_with("assets/") || !files.contains_key(asset) {
                return bad(format!("phone asset {asset} must be a file under assets/"));
            }
        }
        Ok(())
    }
}

/// Which files a pack may hold.
pub fn allowed_file(path: &str) -> bool {
    const ASSET_TYPES: &[&str] = &[
        "png", "jpg", "jpeg", "webp", "gif", "ogg", "mp3", "wav", "woff2", "ttf", "otf", "gltf",
        "glb", "bin", "json", "txt",
    ];
    match path {
        MANIFEST_FILE | SIGNATURE_FILE | "rules.wasm" | "rules.js" | "table.json" | "LICENSE"
        | "LICENSE.txt" | "README.md" => true,
        _ => {
            let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
            let ext = ext.as_deref().unwrap_or("");
            if let Some(rest) = path.strip_prefix("assets/") {
                !rest.is_empty() && ASSET_TYPES.contains(&ext)
            } else if path.starts_with("ui/") || path.starts_with("locales/") {
                ext == "json"
            } else {
                false
            }
        }
    }
}

fn path_ok(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 200
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains('\0')
        && path
            .split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

/// A pack in memory: every file by path.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pack {
    pub files: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureFile {
    /// The signer's public key, base64.
    pub key: String,
    /// ed25519 over [`Pack::digest`], base64.
    pub signature: String,
}

impl Pack {
    /// Read a `.gamepack`, refusing anything over the size cap (counted
    /// while decompressing, so a zip bomb stops early).
    pub fn read(bytes: &[u8]) -> Result<Self, PackError> {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
            .map_err(|e| PackError::Unreadable(e.to_string()))?;
        if zip.len() > MAX_FILES {
            return Err(PackError::Unreadable(format!("{} files", zip.len())));
        }
        let mut files = BTreeMap::new();
        let mut total: u64 = 0;
        for i in 0..zip.len() {
            let mut entry = zip
                .by_index(i)
                .map_err(|e| PackError::Unreadable(e.to_string()))?;
            if entry.is_dir() {
                continue;
            }
            let name = entry
                .name()
                .map_err(|e| PackError::Unreadable(e.to_string()))?
                .to_string();
            if !path_ok(&name) {
                return Err(PackError::Unreadable(format!("bad path {name:?}")));
            }
            total = total.saturating_add(entry.size());
            if total > MAX_PACK_BYTES {
                return Err(PackError::TooLarge(total));
            }
            let room = MAX_PACK_BYTES - (total - entry.size());
            let mut data = Vec::new();
            (&mut entry)
                .take(room + 1)
                .read_to_end(&mut data)
                .map_err(|e| PackError::Unreadable(e.to_string()))?;
            if data.len() as u64 > room {
                return Err(PackError::TooLarge(MAX_PACK_BYTES + 1));
            }
            if files.insert(name.clone(), data).is_some() {
                return Err(PackError::Unreadable(format!("{name} appears twice")));
            }
        }
        Ok(Self { files })
    }

    pub fn write(&self) -> Result<Vec<u8>, PackError> {
        let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in &self.files {
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .large_file(data.len() as u64 >= u32::MAX as u64);
            out.start_file(name.as_str(), options)
                .map_err(|e| PackError::Unreadable(e.to_string()))?;
            out.write_all(data)
                .map_err(|e| PackError::Unreadable(e.to_string()))?;
        }
        let cursor = out
            .finish()
            .map_err(|e| PackError::Unreadable(e.to_string()))?;
        Ok(cursor.into_inner())
    }

    pub fn manifest(&self) -> Result<Manifest, PackError> {
        let bytes = self
            .files
            .get(MANIFEST_FILE)
            .ok_or_else(|| PackError::BadManifest("no manifest.json".into()))?;
        let manifest: Manifest =
            serde_json::from_slice(bytes).map_err(|e| PackError::BadManifest(e.to_string()))?;
        manifest.validate(&self.files)?;
        Ok(manifest)
    }

    /// Every file must be an allowed type.
    pub fn check_files(&self) -> Result<(), PackError> {
        for name in self.files.keys() {
            if !allowed_file(name) {
                return Err(PackError::ForbiddenFile(name.clone()));
            }
        }
        let total: u64 = self.files.values().map(|d| d.len() as u64).sum();
        if total > MAX_PACK_BYTES {
            return Err(PackError::TooLarge(total));
        }
        Ok(())
    }

    /// SHA-256 over every file except the signature: one line per file,
    /// sorted by path, `path NUL sha256-hex LF`.
    pub fn digest(&self) -> [u8; 32] {
        let mut listing = Sha256::new();
        for (name, data) in &self.files {
            if name == SIGNATURE_FILE {
                continue;
            }
            let file_hash = Sha256::digest(data);
            listing.update(name.as_bytes());
            listing.update([0u8]);
            listing.update(hex(&file_hash).as_bytes());
            listing.update(b"\n");
        }
        listing.finalize().into()
    }

    /// Put the signer's key into the manifest and sign every file.
    pub fn sign(&mut self, key: &SigningKey) -> Result<(), PackError> {
        let public = B64.encode(key.verifying_key().to_bytes());
        let bytes = self
            .files
            .get(MANIFEST_FILE)
            .ok_or_else(|| PackError::BadManifest("no manifest.json".into()))?;
        let mut manifest: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|e| PackError::BadManifest(e.to_string()))?;
        manifest["author_key"] = serde_json::Value::String(public.clone());
        self.files.insert(
            MANIFEST_FILE.into(),
            serde_json::to_vec_pretty(&manifest).expect("json"),
        );
        self.files.remove(SIGNATURE_FILE);
        let signature = key.sign(&self.digest());
        let sig = SignatureFile {
            key: public,
            signature: B64.encode(signature.to_bytes()),
        };
        self.files.insert(
            SIGNATURE_FILE.into(),
            serde_json::to_vec_pretty(&sig).expect("json"),
        );
        Ok(())
    }

    /// The signature must be valid and made with the manifest's author key.
    /// Returns that key (base64).
    pub fn verify(&self, manifest: &Manifest) -> Result<String, PackError> {
        let bytes = self.files.get(SIGNATURE_FILE).ok_or(PackError::Unsigned)?;
        let sig: SignatureFile = serde_json::from_slice(bytes)
            .map_err(|e| PackError::BadSignature(format!("{SIGNATURE_FILE}: {e}")))?;
        if manifest.author_key.is_empty() {
            return Err(PackError::Unsigned);
        }
        if sig.key != manifest.author_key {
            return Err(PackError::KeyMismatch(
                "signed with a different key than the manifest's author_key".into(),
            ));
        }
        let key = parse_public_key(&sig.key)?;
        let raw = B64
            .decode(sig.signature.as_bytes())
            .map_err(|e| PackError::BadSignature(e.to_string()))?;
        let signature =
            Signature::from_slice(&raw).map_err(|e| PackError::BadSignature(e.to_string()))?;
        key.verify_strict(&self.digest(), &signature)
            .map_err(|_| PackError::BadSignature("does not match the files".into()))?;
        Ok(sig.key)
    }
}

pub fn parse_public_key(b64: &str) -> Result<VerifyingKey, PackError> {
    let raw = B64
        .decode(b64.as_bytes())
        .map_err(|e| PackError::BadSignature(format!("key: {e}")))?;
    let bytes: [u8; 32] = raw
        .try_into()
        .map_err(|_| PackError::BadSignature("key must be 32 bytes".into()))?;
    VerifyingKey::from_bytes(&bytes).map_err(|e| PackError::BadSignature(e.to_string()))
}

/// A new signing key from the OS random source.
pub fn generate_key() -> SigningKey {
    use rand::RngCore;
    let mut secret = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut secret);
    SigningKey::from_bytes(&secret)
}

/// Key files hold the secret as base64 on one line after a comment.
pub fn key_to_file(key: &SigningKey) -> String {
    format!(
        "# Hostable pack signing key. Keep this file secret; never commit it.\n# public: {}\n{}\n",
        B64.encode(key.verifying_key().to_bytes()),
        B64.encode(key.to_bytes())
    )
}

pub fn key_from_file(text: &str) -> Result<SigningKey, PackError> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .ok_or_else(|| PackError::BadSignature("empty key file".into()))?;
    let raw = B64
        .decode(line.as_bytes())
        .map_err(|e| PackError::BadSignature(format!("key file: {e}")))?;
    let secret: [u8; 32] = raw
        .try_into()
        .map_err(|_| PackError::BadSignature("key file must hold 32 bytes".into()))?;
    Ok(SigningKey::from_bytes(&secret))
}

pub fn public_key_b64(key: &SigningKey) -> String {
    B64.encode(key.verifying_key().to_bytes())
}

/// Pack ids pinned to the key that first published them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pins {
    pub pins: BTreeMap<String, String>,
}

impl Pins {
    /// A new version must be signed by the key that signed the first one.
    pub fn check(&self, id: &str, key: &str) -> Result<(), PackError> {
        match self.pins.get(id) {
            Some(pinned) if pinned != key => Err(PackError::KeyMismatch(format!(
                "pack {id} is pinned to a different author key"
            ))),
            _ => Ok(()),
        }
    }

    /// Check, then pin if new. Returns whether it was newly pinned.
    pub fn pin(&mut self, id: &str, key: &str) -> Result<bool, PackError> {
        self.check(id, key)?;
        Ok(self.pins.insert(id.into(), key.into()).is_none())
    }
}

/// A content hash for asset caches (phones fetch assets by hash).
pub fn content_hash(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests;
