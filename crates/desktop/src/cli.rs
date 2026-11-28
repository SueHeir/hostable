//! Command-line options.

use std::path::PathBuf;

use crate::config;

pub const USAGE: &str = "\
hostable-desktop [options]

  --relay URL       relay base URL (default wss://dinkydelvers.com/play)
  --join-url URL    web client URL phones open (default https://dinkydelvers.com/play)
  --game ID         game to host: buzzer (built in) or a pack id from --packs
  --pack FILE       host this .gamepack
  --packs DIR       load .gamepack files from DIR
  --cloud URL       cloud host root, enables \"Move to cloud\"
  --save FILE       save the game here after every change
  --resume FILE     reopen a saved game (and reclaim its room)
  --smoke-test      no window: play a game with simulated phones and exit
  --players N       simulated phones in --smoke-test (default 3)
  -h, --help        show this
";

#[derive(Debug, Clone, PartialEq)]
pub struct Cli {
    pub relay: String,
    pub join_url: String,
    pub game: Option<String>,
    pub pack: Option<PathBuf>,
    pub packs: Option<PathBuf>,
    pub cloud: Option<String>,
    pub save: Option<PathBuf>,
    pub resume: Option<PathBuf>,
    pub smoke_test: bool,
    pub players: usize,
    /// `--relay` was given (smoke tests must not use the public relay).
    pub relay_given: bool,
    pub help: bool,
}

impl Default for Cli {
    fn default() -> Self {
        Cli {
            relay: config::default_relay(),
            join_url: config::default_join_url(),
            game: None,
            pack: None,
            packs: None,
            cloud: None,
            save: None,
            resume: None,
            smoke_test: false,
            players: 3,
            relay_given: false,
            help: false,
        }
    }
}

impl Cli {
    /// Parse arguments (without the program name).
    pub fn parse<I, S>(args: I) -> Result<Cli, String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut cli = Cli::default();
        let mut args = args.into_iter().map(Into::into);
        while let Some(arg) = args.next() {
            // Accept `--flag=value` as well as `--flag value`.
            let (flag, inline) = match arg.split_once('=') {
                Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
                _ => (arg.clone(), None),
            };
            let mut value = |name: &str| -> Result<String, String> {
                inline
                    .clone()
                    .or_else(|| args.next())
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| format!("{name} needs a value"))
            };
            match flag.as_str() {
                "--relay" => {
                    cli.relay = value("--relay")?;
                    cli.relay_given = true;
                }
                "--join-url" => cli.join_url = value("--join-url")?,
                "--game" => cli.game = Some(value("--game")?),
                "--pack" => cli.pack = Some(value("--pack")?.into()),
                "--packs" => cli.packs = Some(value("--packs")?.into()),
                "--cloud" => cli.cloud = Some(value("--cloud")?),
                "--save" => cli.save = Some(value("--save")?.into()),
                "--resume" => cli.resume = Some(value("--resume")?.into()),
                "--smoke-test" => cli.smoke_test = true,
                "--players" => {
                    let v = value("--players")?;
                    cli.players = v
                        .parse()
                        .ok()
                        .filter(|n| (1..=64).contains(n))
                        .ok_or_else(|| format!("--players wants a number from 1 to 64, not {v}"))?;
                }
                "-h" | "--help" => cli.help = true,
                other => return Err(format!("unknown option {other}")),
            }
        }
        for (name, url) in [("--relay", &cli.relay)] {
            if !(url.starts_with("ws://") || url.starts_with("wss://")) {
                return Err(format!("{name} must start with ws:// or wss://"));
            }
        }
        for (name, url) in [
            ("--join-url", Some(&cli.join_url)),
            ("--cloud", cli.cloud.as_ref()),
        ] {
            if let Some(url) = url {
                if !(url.starts_with("http://") || url.starts_with("https://")) {
                    return Err(format!("{name} must start with http:// or https://"));
                }
            }
        }
        if cli.smoke_test && !cli.relay_given {
            return Err("--smoke-test needs --relay ws://HOST:PORT".into());
        }
        Ok(cli)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_use_the_configured_domain() {
        let cli = Cli::parse(Vec::<String>::new()).unwrap();
        assert_eq!(cli.relay, "wss://dinkydelvers.com/play");
        assert_eq!(cli.join_url, "https://dinkydelvers.com/play");
        assert_eq!(cli.players, 3);
        assert!(!cli.smoke_test);
    }

    #[test]
    fn all_options_parse() {
        let cli = Cli::parse([
            "--relay",
            "ws://127.0.0.1:9000",
            "--pack",
            "duel.gamepack",
            "--packs=packs",
            "--game",
            "wizards-duel",
            "--cloud",
            "https://example.test/play",
            "--save",
            "game.json",
            "--resume",
            "old.json",
            "--join-url",
            "http://localhost:5173",
        ])
        .unwrap();
        assert_eq!(cli.relay, "ws://127.0.0.1:9000");
        assert_eq!(cli.pack, Some(PathBuf::from("duel.gamepack")));
        assert_eq!(cli.packs, Some(PathBuf::from("packs")));
        assert_eq!(cli.game.as_deref(), Some("wizards-duel"));
        assert_eq!(cli.cloud.as_deref(), Some("https://example.test/play"));
        assert_eq!(cli.save, Some(PathBuf::from("game.json")));
        assert_eq!(cli.resume, Some(PathBuf::from("old.json")));
        assert_eq!(cli.join_url, "http://localhost:5173");
    }

    #[test]
    fn smoke_test_options() {
        let cli = Cli::parse([
            "--smoke-test",
            "--relay",
            "ws://127.0.0.1:1234",
            "--players",
            "5",
        ])
        .unwrap();
        assert!(cli.smoke_test);
        assert_eq!(cli.players, 5);
        assert!(Cli::parse(["--smoke-test"])
            .unwrap_err()
            .contains("--relay"));
    }

    #[test]
    fn bad_input_is_refused() {
        assert!(Cli::parse(["--bogus"]).is_err());
        assert!(Cli::parse(["--relay"]).is_err());
        assert!(Cli::parse(["--relay", "http://x"]).is_err());
        assert!(Cli::parse(["--players", "zero"]).is_err());
        assert!(Cli::parse(["--players", "0"]).is_err());
        assert!(Cli::parse(["--cloud", "ftp://x"]).is_err());
        assert!(Cli::parse(["--help"]).unwrap().help);
    }
}
