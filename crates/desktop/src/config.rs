//! Where the desktop host connects by default. The test deployment lives
//! under one path on one domain; change [`DOMAIN`] (or pass `--relay` and
//! `--join-url`) when the platform moves.

/// The domain serving the relay and the web client.
pub const DOMAIN: &str = "dinkydelvers.com";
/// The path both live under on [`DOMAIN`].
pub const BASE_PATH: &str = "/play";

/// The relay base URL; the host runtime appends `/ws/host`.
pub fn default_relay() -> String {
    format!("wss://{DOMAIN}{BASE_PATH}")
}

/// The web client phones open; the QR code points at `<this>/j/<CODE>`.
pub fn default_join_url() -> String {
    format!("https://{DOMAIN}{BASE_PATH}")
}

/// The link a phone opens to join room `code`.
pub fn join_link(join_url: &str, code: &str) -> String {
    format!("{}/j/{}", join_url.trim_end_matches('/'), code)
}

/// The full share link for a game moved to the cloud. `path` is what the
/// cloud answered (`/g/<id>`), relative to its root.
pub fn share_link(cloud: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_string();
    }
    format!(
        "{}/{}",
        cloud.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_point_at_the_test_deployment() {
        assert_eq!(default_relay(), "wss://dinkydelvers.com/play");
        assert_eq!(default_join_url(), "https://dinkydelvers.com/play");
    }

    #[test]
    fn join_links_append_the_code() {
        assert_eq!(
            join_link("https://dinkydelvers.com/play", "BCDF"),
            "https://dinkydelvers.com/play/j/BCDF"
        );
        assert_eq!(
            join_link("http://localhost:5173/", "XYZW"),
            "http://localhost:5173/j/XYZW"
        );
    }

    #[test]
    fn share_links_join_the_cloud_root_and_path() {
        assert_eq!(
            share_link("https://dinkydelvers.com/play/", "/g/abc"),
            "https://dinkydelvers.com/play/g/abc"
        );
        assert_eq!(
            share_link("https://x.test", "https://y.test/g/1"),
            "https://y.test/g/1"
        );
    }
}
