//! Picking the game to host from the command line: the built-in buzzer, a
//! single `.gamepack`, a directory of packs, or a saved game to resume.

use std::path::Path;

use hostable_buzzer::Buzzer;
use hostable_host_core::Room;
use hostable_host_runtime::{now_ms, Catalog, GameRoom, Options, SaveTo, Saved};
use hostable_rules_api::{DynRules, Json};
use hostable_sandbox::Limits;

use crate::cli::Cli;

/// The built-in game's id.
pub const BUZZER: &str = "buzzer";

/// Everything needed to start [`hostable_host_runtime::run`].
pub struct GameSetup {
    pub room: GameRoom,
    pub reclaim: Option<(String, String)>,
    pub options: Options,
    pub name: String,
    /// The pack's native renderer, if it names one (`wizards-duel`).
    pub renderer: Option<String>,
}

struct Chosen {
    rules: Box<DynRules>,
    name: String,
    renderer: Option<String>,
}

fn load_catalog(cli: &Cli) -> Result<(Catalog, Option<String>), String> {
    let mut catalog = match &cli.packs {
        Some(dir) => {
            if !dir.is_dir() {
                return Err(format!("--packs {} is not a directory", dir.display()));
            }
            Catalog::from_dir(dir)
        }
        None => Catalog::default(),
    };
    for (name, why) in &catalog.rejected {
        eprintln!("hostable: skipped pack {name}: {why}");
    }
    let mut from_file = None;
    if let Some(path) = &cli.pack {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let id = catalog
            .add(&bytes)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        from_file = Some(id);
    }
    Ok((catalog, from_file))
}

fn choose(catalog: &Catalog, id: &str) -> Result<Chosen, String> {
    if let Some(entry) = catalog.get(id) {
        return Ok(Chosen {
            rules: entry.rules(Limits::DEFAULT)?,
            name: entry.manifest.name.clone(),
            renderer: entry.manifest.renderer.clone(),
        });
    }
    if id == BUZZER {
        return Ok(Chosen {
            rules: Box::new(Json(Buzzer)),
            name: "Buzzer".into(),
            renderer: None,
        });
    }
    let mut known: Vec<&str> = catalog.packs.keys().map(String::as_str).collect();
    known.push(BUZZER);
    Err(format!(
        "no game {id} (have: {}); pass --pack or --packs",
        known.join(", ")
    ))
}

fn read_save(path: &Path) -> Result<Saved, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("{}: not a save: {e}", path.display()))
}

/// Work out the game from the options.
pub fn prepare(cli: &Cli) -> Result<GameSetup, String> {
    let (catalog, from_file) = load_catalog(cli)?;
    let save_to = match &cli.save {
        Some(p) => SaveTo::File(p.clone()),
        None => SaveTo::Nowhere,
    };
    let max_reconnects = if cli.smoke_test { Some(3) } else { None };
    if let Some(path) = &cli.resume {
        let saved = read_save(path)?;
        let chosen = choose(&catalog, &saved.pack)?;
        let room = saved.restore(chosen.rules)?.with_name(chosen.name.clone());
        return Ok(GameSetup {
            room,
            reclaim: Some((saved.code.clone(), saved.host_token.clone())),
            options: Options {
                relay: cli.relay.clone(),
                pack: saved.pack,
                save_to,
                max_reconnects,
            },
            name: chosen.name,
            renderer: chosen.renderer,
        });
    }
    let id = cli
        .game
        .clone()
        .or(from_file)
        .unwrap_or_else(|| BUZZER.to_string());
    let chosen = choose(&catalog, &id)?;
    let room = Room::new(chosen.rules, now_ms()).with_name(chosen.name.clone());
    Ok(GameSetup {
        room,
        reclaim: None,
        options: Options {
            relay: cli.relay.clone(),
            pack: id,
            save_to,
            max_reconnects,
        },
        name: chosen.name,
        renderer: chosen.renderer,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_buzzer_is_built_in() {
        let setup = prepare(&Cli::default()).unwrap();
        assert_eq!(setup.options.pack, "buzzer");
        assert_eq!(setup.name, "Buzzer");
        assert_eq!(setup.room.rules().seats(), (2, 12));
        assert!(setup.reclaim.is_none());
    }

    #[test]
    fn unknown_games_and_missing_files_are_errors() {
        let cli = Cli {
            game: Some("nope".into()),
            ..Cli::default()
        };
        assert!(prepare(&cli).err().unwrap().contains("no game nope"));
        let cli = Cli {
            pack: Some("/does/not/exist.gamepack".into()),
            ..Cli::default()
        };
        assert!(prepare(&cli).is_err());
    }
}
