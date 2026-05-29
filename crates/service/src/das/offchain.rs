//! Off-chain metadata enrichment.
//!
//! An NFT's on-chain account carries only a `name` and a `uri`. The
//! rich metadata — image, description, attributes, extra files — lives
//! in a JSON document at that URI (Arweave, IPFS, a web server, or a
//! local `file://` during dev). Real Helius `getAsset` fetches that
//! document and folds it into the response; a DAS answer without the
//! image is a half-answer that forces every consumer to re-fetch the
//! URI themselves.
//!
//! This module does that fold. It is deliberately **fail-soft**: any
//! fetch or parse error leaves the on-chain fields intact and returns
//! quietly. A blocked network (CI sandbox), a slow gateway, or a
//! garbage URI degrades a `getAsset` to its on-chain half rather than
//! failing the call. The actual fetch (with timeout + size cap +
//! `file://` support) lives in the `UpstreamClient::fetch_uri` impl;
//! this module only orchestrates and maps the JSON.

use serde_json::Value;

use crate::das::types::{DasAsset, DasAttribute, DasFile};
use crate::upstream::UpstreamClient;

/// Fetch `asset.content.json_uri` and merge the off-chain JSON into the
/// asset's content. No-op when the URI is empty, the fetch returns
/// `None` (disabled / unreachable / timed out), or the body isn't JSON.
pub async fn enrich_offchain_metadata<U>(upstream: &U, asset: &mut DasAsset)
where
    U: UpstreamClient + ?Sized,
{
    let uri = asset.content.json_uri.trim();
    if uri.is_empty() {
        return;
    }
    let Some(bytes) = upstream.fetch_uri(uri).await else {
        return; // disabled, unreachable, timed out — keep on-chain fields
    };
    let Ok(json) = serde_json::from_slice::<Value>(&bytes) else {
        return; // not JSON — leave the asset as-is
    };
    merge_offchain_json(asset, &json);
}

/// Pure mapping from a Metaplex off-chain JSON document onto a
/// `DasAsset`'s content. Separated from the fetch so it's testable
/// without any upstream.
///
/// Only fills fields that the off-chain doc actually carries; never
/// clobbers a non-empty on-chain value with an empty off-chain one.
pub fn merge_offchain_json(asset: &mut DasAsset, json: &Value) {
    let content = &mut asset.content;

    // description: on-chain MplCore/metadata has none, so off-chain wins.
    if let Some(desc) = json.get("description").and_then(Value::as_str) {
        if !desc.is_empty() {
            content.metadata.description = desc.to_string();
        }
    }

    // name: only fill if the on-chain decoder left it empty (on-chain
    // name is authoritative when present).
    if content.metadata.name.is_empty() {
        if let Some(name) = json.get("name").and_then(Value::as_str) {
            content.metadata.name = name.to_string();
        }
    }

    // symbol: same rule.
    if content.metadata.symbol.is_empty() {
        if let Some(symbol) = json.get("symbol").and_then(Value::as_str) {
            content.metadata.symbol = symbol.to_string();
        }
    }

    // links.
    if let Some(image) = json.get("image").and_then(Value::as_str) {
        content.links.image = Some(image.to_string());
    }
    if let Some(anim) = json.get("animation_url").and_then(Value::as_str) {
        content.links.animation_url = Some(anim.to_string());
    }
    if let Some(ext) = json.get("external_url").and_then(Value::as_str) {
        content.links.external_url = Some(ext.to_string());
    }

    // attributes (top-level array of {trait_type, value}).
    if let Some(attrs) = json.get("attributes").and_then(Value::as_array) {
        let mapped: Vec<DasAttribute> = attrs
            .iter()
            .filter_map(|a| {
                let trait_type = a.get("trait_type").and_then(Value::as_str)?.to_string();
                let value = a.get("value").cloned().unwrap_or(Value::Null);
                Some(DasAttribute { trait_type, value })
            })
            .collect();
        if !mapped.is_empty() {
            content.metadata.attributes = Some(mapped);
        }
    }

    // files + category from `properties`.
    if let Some(props) = json.get("properties") {
        if let Some(files) = props.get("files").and_then(Value::as_array) {
            let mapped: Vec<DasFile> = files
                .iter()
                .filter_map(|f| {
                    let uri = f.get("uri").and_then(Value::as_str)?.to_string();
                    // off-chain uses `type`; DAS exposes it as `mime`.
                    let mime = f
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    Some(DasFile {
                        uri,
                        mime,
                        ..Default::default()
                    })
                })
                .collect();
            if !mapped.is_empty() {
                content.files = mapped;
            }
        }
        if let Some(cat) = props.get("category").and_then(Value::as_str) {
            content.category = Some(cat.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::das::types::{DasContent, DasMetadata};

    fn asset_with_uri(uri: &str, name: &str) -> DasAsset {
        DasAsset {
            content: DasContent {
                json_uri: uri.to_string(),
                metadata: DasMetadata {
                    name: name.to_string(),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn merge_populates_all_fields() {
        let mut asset = asset_with_uri("https://x/meta.json", "On-chain Name");
        let json = serde_json::json!({
            "name": "Off-chain Name",
            "description": "A test asset",
            "image": "https://x/img.png",
            "animation_url": "https://x/vid.mp4",
            "external_url": "https://x",
            "attributes": [
                {"trait_type": "Color", "value": "Blue"},
                {"trait_type": "Level", "value": 7}
            ],
            "properties": {
                "files": [{"uri": "https://x/img.png", "type": "image/png"}],
                "category": "image"
            }
        });
        merge_offchain_json(&mut asset, &json);

        // on-chain name preserved (not clobbered by off-chain)
        assert_eq!(asset.content.metadata.name, "On-chain Name");
        assert_eq!(asset.content.metadata.description, "A test asset");
        assert_eq!(
            asset.content.links.image.as_deref(),
            Some("https://x/img.png")
        );
        assert_eq!(
            asset.content.links.animation_url.as_deref(),
            Some("https://x/vid.mp4")
        );
        assert_eq!(
            asset.content.links.external_url.as_deref(),
            Some("https://x")
        );
        let attrs = asset.content.metadata.attributes.as_ref().unwrap();
        assert_eq!(attrs.len(), 2);
        assert_eq!(attrs[0].trait_type, "Color");
        assert_eq!(asset.content.files.len(), 1);
        assert_eq!(asset.content.files[0].mime, "image/png");
        assert_eq!(asset.content.category.as_deref(), Some("image"));
    }

    #[test]
    fn merge_fills_name_when_onchain_empty() {
        let mut asset = asset_with_uri("https://x/meta.json", "");
        let json = serde_json::json!({ "name": "Off-chain Name" });
        merge_offchain_json(&mut asset, &json);
        assert_eq!(asset.content.metadata.name, "Off-chain Name");
    }

    #[test]
    fn merge_is_noop_on_empty_json() {
        let mut asset = asset_with_uri("https://x/meta.json", "Keep Me");
        merge_offchain_json(&mut asset, &serde_json::json!({}));
        assert_eq!(asset.content.metadata.name, "Keep Me");
        assert!(asset.content.links.image.is_none());
        assert!(asset.content.metadata.attributes.is_none());
    }

    #[tokio::test]
    async fn enrich_via_fixture_upstream() {
        use crate::upstream::FixtureUpstream;
        let body = serde_json::to_vec(&serde_json::json!({
            "description": "from fixture",
            "image": "file:///tmp/img.png"
        }))
        .unwrap();
        let upstream = FixtureUpstream::new().with_offchain("file:///tmp/meta.json", body);
        let mut asset = asset_with_uri("file:///tmp/meta.json", "Name");
        enrich_offchain_metadata(&upstream, &mut asset).await;
        assert_eq!(asset.content.metadata.description, "from fixture");
        assert_eq!(
            asset.content.links.image.as_deref(),
            Some("file:///tmp/img.png")
        );
    }

    #[tokio::test]
    async fn enrich_failsoft_when_uri_unregistered() {
        use crate::upstream::FixtureUpstream;
        let upstream = FixtureUpstream::new(); // no offchain registered
        let mut asset = asset_with_uri("https://unreachable/meta.json", "Name");
        enrich_offchain_metadata(&upstream, &mut asset).await;
        // unchanged — fail-soft
        assert_eq!(asset.content.metadata.name, "Name");
        assert!(asset.content.links.image.is_none());
    }
}
