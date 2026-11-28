//! Turning a checked pack into rules a host can run.

use std::collections::BTreeMap;

use hostable_rules_api::DynRules;
use hostable_sandbox::{Limits, WasmRules};

use crate::{Level, Manifest, Pack, PackError};

/// A pack ready to play.
pub struct LoadedGame {
    pub manifest: Manifest,
    pub rules: Box<DynRules>,
    /// Files phones may fetch, by path.
    pub phone_assets: BTreeMap<String, Vec<u8>>,
    /// The signer's key (base64).
    pub author_key: String,
}

/// Check the files, manifest and signature, then load the rules into the
/// sandbox. Doesn't play any games; see [`crate::check_pack`] for that.
pub fn load_rules(pack: &Pack, limits: Limits) -> Result<LoadedGame, PackError> {
    pack.check_files()?;
    let manifest = pack.manifest()?;
    let author_key = pack.verify(&manifest)?;
    let rules: Box<DynRules> = match (manifest.level, manifest.rules.as_deref()) {
        (Level::Sandbox, _) => {
            let table = manifest
                .table
                .as_ref()
                .and_then(|t| pack.files.get(t))
                .ok_or_else(|| PackError::BadManifest("no table.json".into()))?;
            let rules = hostable_tabletop::Tabletop::from_json(
                table,
                (manifest.players.min, manifest.players.max),
            )
            .map_err(|e| PackError::BadManifest(format!("table.json: {e}")))?;
            Box::new(hostable_rules_api::Json(rules))
        }
        (_, Some("rules.wasm")) => Box::new(
            WasmRules::new(&pack.files["rules.wasm"], limits)
                .map_err(|e| PackError::BadManifest(format!("rules.wasm: {e}")))?,
        ),
        (_, Some("rules.js")) => Box::new(
            WasmRules::javascript(&pack.files["rules.js"], limits)
                .map_err(|e| PackError::BadManifest(format!("rules.js: {e}")))?,
        ),
        _ => return Err(PackError::BadManifest("no rules".into())),
    };
    let (min, max) = rules.seats();
    if (min, max) != (manifest.players.min, manifest.players.max) {
        return Err(PackError::BadManifest(format!(
            "manifest says {}-{} players, the rules say {min}-{max}",
            manifest.players.min, manifest.players.max
        )));
    }
    let phone_assets = manifest
        .phone_assets
        .iter()
        .map(|p| (p.clone(), pack.files[p].clone()))
        .collect();
    Ok(LoadedGame {
        manifest,
        rules,
        phone_assets,
        author_key,
    })
}
