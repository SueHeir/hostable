//! The packs a host can run, loaded from a directory of `.gamepack` files.
//! Every pack is fully checked (files, manifest, signature) when it is
//! loaded; each new game gets its own fresh sandbox instance.

use std::collections::BTreeMap;
use std::path::Path;

use hostable_pack::{load_rules, Manifest, Pack};
use hostable_rules_api::DynRules;
use hostable_sandbox::Limits;

#[derive(Clone)]
pub struct PackEntry {
    pub manifest: Manifest,
    pub pack: Pack,
    pub author_key: String,
}

impl PackEntry {
    /// A fresh copy of the rules for one game.
    pub fn rules(&self, limits: Limits) -> Result<Box<DynRules>, String> {
        load_rules(&self.pack, limits)
            .map(|g| g.rules)
            .map_err(|e| e.to_string())
    }
}

#[derive(Clone, Default)]
pub struct Catalog {
    pub packs: BTreeMap<String, PackEntry>,
    /// Packs that failed to load, with why.
    pub rejected: Vec<(String, String)>,
}

impl Catalog {
    pub fn from_dir(dir: &Path) -> Self {
        let mut catalog = Catalog::default();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return catalog;
        };
        let mut paths: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "gamepack"))
            .collect();
        paths.sort();
        for path in paths {
            let name = path.display().to_string();
            match std::fs::read(&path) {
                Ok(bytes) => {
                    if let Err(e) = catalog.add(&bytes) {
                        catalog.rejected.push((name, e));
                    }
                }
                Err(e) => catalog.rejected.push((name, e.to_string())),
            }
        }
        catalog
    }

    /// Add a pack from its bytes. A pack id already present must be signed
    /// by the same key (key pinning).
    pub fn add(&mut self, bytes: &[u8]) -> Result<String, String> {
        let pack = Pack::read(bytes).map_err(|e| e.to_string())?;
        let game = load_rules(&pack, Limits::DEFAULT).map_err(|e| e.to_string())?;
        let id = game.manifest.id.clone();
        if let Some(existing) = self.packs.get(&id) {
            if existing.author_key != game.author_key {
                return Err(format!("pack {id} is already published with another key"));
            }
        }
        self.packs.insert(
            id.clone(),
            PackEntry {
                manifest: game.manifest,
                pack,
                author_key: game.author_key,
            },
        );
        Ok(id)
    }

    pub fn get(&self, id: &str) -> Option<&PackEntry> {
        self.packs.get(id)
    }
}
