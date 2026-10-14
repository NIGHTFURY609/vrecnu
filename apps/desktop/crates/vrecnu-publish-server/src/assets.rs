//! The publish page, embedded at compile time by `build.rs`.

use std::borrow::Cow;

use crate::PublishError;

include!(concat!(env!("OUT_DIR"), "/publish_assets.rs"));

/// One file of the publish page.
#[derive(Debug, Clone)]
pub struct Asset {
    /// URL path relative to the site root, e.g. `assets/index-a1b2c3.js`. No leading slash.
    pub path: String,
    /// The `Content-Type` to serve it with.
    pub mime: String,
    /// The bytes.
    pub bytes: Cow<'static, [u8]>,
}

/// The whole publish page as a lookup table.
///
/// Deliberately **not** a filesystem root. The server can only answer with something in this
/// table, so there is no path to traverse out of and no directory to enumerate — the
/// "never serve a directory" rule in ADR-002 is a property of the type rather than a check
/// somebody has to remember to write.
#[derive(Debug, Clone)]
pub struct PublishAssets {
    entries: Vec<Asset>,
}

impl PublishAssets {
    /// The assets this build embedded.
    ///
    /// # Errors
    ///
    /// [`PublishError::AssetsMissing`] when the crate was compiled without
    /// `packages/publish/dist` present, or when what was embedded has no `index.html` — a page
    /// with no entry point would serve a 404 at `/` and look like a port problem.
    pub fn embedded() -> Result<Self, PublishError> {
        let entries: Vec<Asset> = EMBEDDED
            .iter()
            .map(|(path, mime, bytes)| Asset {
                path: (*path).to_owned(),
                mime: (*mime).to_owned(),
                bytes: Cow::Borrowed(*bytes),
            })
            .collect();
        let assets = Self { entries };
        if assets.get("index.html").is_none() {
            return Err(PublishError::AssetsMissing);
        }
        Ok(assets)
    }

    /// Builds a table from parts. The desktop shell does not need this; tests do.
    #[must_use]
    pub fn from_entries(entries: Vec<Asset>) -> Self {
        Self { entries }
    }

    /// Looks up one file by its URL path, with no leading slash.
    #[must_use]
    pub fn get(&self, path: &str) -> Option<&Asset> {
        self.entries.iter().find(|asset| asset.path == path)
    }

    /// How many files are embedded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is embedded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Maps a request URL path to an asset key.
///
/// `/` means `index.html`. Everything else is taken literally, **without** an SPA fallback:
/// the publish page is one route, and a catch-all would answer `/r/<stale-token>` with HTML
/// where the page expects video bytes, turning an expired session into a decode error instead
/// of a 404 the page can explain.
#[must_use]
pub fn asset_key_for(url_path: &str) -> Option<String> {
    let trimmed = url_path.trim_start_matches('/');
    if trimmed.is_empty() {
        return Some("index.html".to_owned());
    }
    // Reject anything that could climb out of the table even though the table has no
    // filesystem under it. Cheap, and it keeps the invariant true if that ever changes.
    if trimmed.contains("..") || trimmed.contains('\\') {
        return None;
    }
    Some(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(path: &str) -> Asset {
        Asset {
            path: path.to_owned(),
            mime: "text/plain".to_owned(),
            bytes: Cow::Owned(b"x".to_vec()),
        }
    }

    #[test]
    fn the_site_root_resolves_to_the_entry_point() {
        assert_eq!(asset_key_for("/").as_deref(), Some("index.html"));
        assert_eq!(asset_key_for("").as_deref(), Some("index.html"));
    }

    #[test]
    fn a_nested_asset_keeps_its_path() {
        assert_eq!(
            asset_key_for("/assets/index-a1b2c3.js").as_deref(),
            Some("assets/index-a1b2c3.js")
        );
    }

    #[test]
    fn traversal_attempts_resolve_to_nothing() {
        assert_eq!(asset_key_for("/../../secrets.txt"), None);
        assert_eq!(asset_key_for("/assets/..%2fsecrets"), None);
        assert_eq!(asset_key_for("/assets\\windows"), None);
    }

    #[test]
    fn a_table_without_an_index_is_a_missing_page_not_an_empty_one() {
        // The failure mode this catches: a partial build serves 404 at `/` and the user
        // reports "the port is broken".
        let assets = PublishAssets::from_entries(vec![asset("assets/app.js")]);
        assert!(assets.get("index.html").is_none());
    }

    #[test]
    fn lookup_is_exact_rather_than_prefixed() {
        let assets = PublishAssets::from_entries(vec![asset("index.html")]);
        assert!(assets.get("index.html").is_some());
        assert!(assets.get("index.htm").is_none());
        assert!(assets.get("/index.html").is_none());
    }
}
