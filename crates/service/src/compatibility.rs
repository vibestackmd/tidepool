//! Upstream compatibility pins — the versions of Surfpool, helius-sdk,
//! Solana, etc. that this release of Tidepool was tested against.
//!
//! Source of truth is the repo-root `compatibility.toml`. A symlink
//! at `crates/service/compatibility.toml` points there; `include_str!`
//! reads through it in dev and reads the dereferenced copy that
//! `cargo publish` bakes into the package tarball. This keeps a
//! single source of truth without duplicating the file or needing a
//! build script.
//!
//! Release preflight (`scripts/preflight.sh`) asserts that every
//! version bump either confirms these pins are still accurate or
//! updates them. Forcing a "yes, we re-verified" step per release is
//! the whole point — no silent compatibility drift between releases.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Embedded `compatibility.toml`. Parsed on first access + cached.
const COMPATIBILITY_TOML: &str = include_str!("../compatibility.toml");

/// One pin — a SemVer range plus optional context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pin {
    /// SemVer range in cargo/npm syntax. `"any"` for intentionally
    /// unpinned entries (with a `note` explaining why).
    pub version: String,
    /// Upstream source URL. Helps tooling link back to the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Free-form rationale. Read by humans reviewing a release PR;
    /// surfaced verbatim in `tidepool_info`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Parsed shape of `compatibility.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Compatibility {
    /// Upstream project → pin. Names are caller-chosen strings
    /// (`"surfpool"`, `"helius-sdk"`, etc.); we don't impose a schema
    /// because the set will evolve faster than any enum would.
    #[serde(rename = "tested-against", default)]
    pub tested_against: BTreeMap<String, Pin>,
    /// Runtime constraints (Node version, Python version, etc. as we
    /// add language bindings). Distinct from `tested_against` because
    /// the interpretation is different: "you need this" vs. "we
    /// verified against this".
    #[serde(default)]
    pub runtime: BTreeMap<String, Pin>,
}

/// Parse the embedded TOML once. Panics at startup if the file is
/// malformed — that's a build-time guarantee we want, not a runtime
/// error users have to handle.
#[must_use]
pub fn compatibility() -> &'static Compatibility {
    // OnceLock-style lazy init without the std::sync::OnceLock
    // borrow dance — toml parsing is cheap enough that we can just
    // do it on every call if we ever drop the cache. Today we cache.
    use std::sync::OnceLock;
    static CACHE: OnceLock<Compatibility> = OnceLock::new();
    CACHE.get_or_init(|| {
        toml::from_str(COMPATIBILITY_TOML)
            .expect("compatibility.toml is malformed — this is a build-time bug")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_compatibility_parses() {
        let c = compatibility();
        assert!(
            !c.tested_against.is_empty(),
            "tested-against section must have at least one pin"
        );
        // Surfpool is the one upstream we never ship without.
        assert!(
            c.tested_against.contains_key("surfpool"),
            "surfpool pin is mandatory"
        );
        // Rust MSRV is captured here too; cross-check it with the
        // workspace Cargo.toml at release-preflight time.
        assert!(c.tested_against.contains_key("rust"));
    }

    #[test]
    fn every_pin_has_a_version() {
        let c = compatibility();
        for (name, pin) in &c.tested_against {
            assert!(
                !pin.version.is_empty(),
                "pin {name} has empty version string"
            );
        }
    }
}
