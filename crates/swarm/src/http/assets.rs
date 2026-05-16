//! Embedded frontend bundle.
//!
//! `build.rs` walks `src/http/web/dist/` after `npm run build`, embeds each
//! file, and emits a match-based lookup function.  This module exposes a
//! single `lookup` helper for the static-asset handler.

include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));

/// Look up an embedded asset by URL path.  Returns `(bytes, content-type)`
/// or `None`.  `/` resolves to `index.html`.
pub fn lookup(path: &str) -> Option<(&'static [u8], &'static str)> {
    let key = if path == "/" {
        "index.html"
    } else {
        path.trim_start_matches('/')
    };
    lookup_asset(key)
}

#[cfg(test)]
mod tests {
    use super::lookup;

    #[test]
    fn root_normalizes_to_index_html() {
        let root = lookup("/").expect("root asset");
        let index = lookup("index.html").expect("index asset");
        assert_eq!(root.0.as_ptr(), index.0.as_ptr());
        assert_eq!(root.1, index.1);
    }

    #[test]
    fn missing_asset_returns_none() {
        assert!(lookup("/does-not-exist").is_none());
    }
}
