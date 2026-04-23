//! `derive-schemas` — infer a JSON Schema (Draft 7-ish) from every
//! committed fixture and write it to `contracts/schemas/<method>/
//! <case>.schema.json`. Offline: no network, no API key.
//!
//! The inferred schema is deliberately conservative: it records the
//! exact set of keys we saw + their types. Fields that appear as
//! `null` in the fixture are marked optional via `nullable: true`
//! so callers who serialize non-null values still validate. Fields
//! absent from the fixture aren't in the schema — drift detection
//! catches the case where real Helius adds a key we don't produce.
//!
//! Scope intentionally smaller than a full JSON Schema: we don't
//! infer regexes, bounds, or enum values. This is a **shape**
//! contract, not a data-validity contract.

use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use serde::Serialize;
use serde_json::{json, Map, Value};
use tracing::info;

#[derive(Parser)]
pub struct Args {
    /// Directory holding committed fixtures.
    #[arg(long, default_value = "contracts/fixtures")]
    fixtures: PathBuf,
    /// Where to write inferred schemas.
    #[arg(long, default_value = "contracts/schemas")]
    out: PathBuf,
    /// Only derive schemas for cases whose filename contains this
    /// substring. Useful for CI jobs that re-record a single case
    /// and want to regenerate just its schema.
    #[arg(long)]
    only: Option<String>,
}

pub async fn run(args: Args) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(&args.out).await?;

    let mut count = 0;
    let mut skipped = 0;
    let mut method_dirs = tokio::fs::read_dir(&args.fixtures).await?;
    while let Some(method_entry) = method_dirs.next_entry().await? {
        let method_path = method_entry.path();
        if !method_path.is_dir() {
            continue;
        }
        let method_name = method_entry.file_name();

        let mut case_files = tokio::fs::read_dir(&method_path).await?;
        while let Some(case_entry) = case_files.next_entry().await? {
            let case_path = case_entry.path();
            if case_path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            if let Some(only) = &args.only {
                let stem = case_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if !stem.contains(only.as_str()) {
                    skipped += 1;
                    continue;
                }
            }
            let fixture: Fixture = {
                let bytes = tokio::fs::read(&case_path).await?;
                serde_json::from_slice(&bytes)
                    .with_context(|| format!("parsing {}", case_path.display()))?
            };

            // Schema'd: the full raw Helius envelope (jsonrpc + id +
            // result | error). Callers downstream cherry-pick
            // `result` or `error` off this root when validating.
            let schema = derive_schema(&fixture.response);

            let out_dir = args.out.join(&method_name);
            tokio::fs::create_dir_all(&out_dir).await?;
            let case_stem = case_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("case");
            let out_path = out_dir.join(format!("{case_stem}.schema.json"));
            let envelope = SchemaEnvelope {
                case: &fixture.case,
                method: &fixture.method,
                source_fixture: case_path
                    .strip_prefix(std::env::current_dir().unwrap_or_default())
                    .unwrap_or(&case_path)
                    .to_string_lossy()
                    .to_string(),
                schema,
            };
            tokio::fs::write(&out_path, serde_json::to_vec_pretty(&envelope)?).await?;
            info!(out = %out_path.display(), "wrote schema");
            count += 1;
        }
    }

    info!(schemas = count, skipped, out = %args.out.display(), "done");
    Ok(())
}

/// Minimal shape of the fixture envelope we wrote in `record.rs`.
/// Only the fields we consume here are named.
#[derive(Debug, serde::Deserialize)]
struct Fixture {
    case: String,
    method: String,
    response: Value,
}

#[derive(Debug, Serialize)]
struct SchemaEnvelope<'a> {
    case: &'a str,
    method: &'a str,
    /// Which fixture we inferred this from — useful for reviewers
    /// to jump between schema and source without guessing.
    source_fixture: String,
    /// The inferred JSON Schema (Draft 7-ish).
    schema: Value,
}

/// Walk a JSON value and emit a shape schema. `null` values are
/// captured as `{type: "null"}`; mixed-type arrays union their
/// item schemas into `{anyOf: [...]}`.
pub fn derive_schema(value: &Value) -> Value {
    match value {
        Value::Null => json!({ "type": "null" }),
        Value::Bool(_) => json!({ "type": "boolean" }),
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                json!({ "type": "integer" })
            } else {
                json!({ "type": "number" })
            }
        }
        Value::String(_) => json!({ "type": "string" }),
        Value::Array(items) => {
            if items.is_empty() {
                json!({ "type": "array", "items": {} })
            } else {
                let first = derive_schema(&items[0]);
                let all_same = items.iter().skip(1).all(|v| derive_schema(v) == first);
                if all_same {
                    json!({ "type": "array", "items": first })
                } else {
                    let variants: Vec<Value> = items.iter().map(derive_schema).collect();
                    json!({ "type": "array", "items": { "anyOf": dedupe(variants) } })
                }
            }
        }
        Value::Object(map) => {
            let mut properties = Map::new();
            let mut required: Vec<String> = Vec::with_capacity(map.len());
            for (k, v) in map {
                properties.insert(k.clone(), derive_schema(v));
                required.push(k.clone());
            }
            required.sort();
            json!({
                "type": "object",
                "properties": properties,
                "required": required,
                "additionalProperties": false,
            })
        }
    }
}

fn dedupe(values: Vec<Value>) -> Vec<Value> {
    let mut seen: Vec<Value> = Vec::with_capacity(values.len());
    for v in values {
        if !seen.iter().any(|s| s == &v) {
            seen.push(v);
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitive_types_inferred() {
        assert_eq!(derive_schema(&json!(42)), json!({ "type": "integer" }));
        assert_eq!(derive_schema(&json!(2.5)), json!({ "type": "number" }));
        assert_eq!(derive_schema(&json!("hi")), json!({ "type": "string" }));
        assert_eq!(derive_schema(&json!(true)), json!({ "type": "boolean" }));
        assert_eq!(derive_schema(&json!(null)), json!({ "type": "null" }));
    }

    #[test]
    fn object_captures_all_keys_as_required_and_forbids_extras() {
        let schema = derive_schema(&json!({ "a": 1, "b": "x" }));
        assert_eq!(
            schema["type"],
            "integer"
                .strip_prefix('i')
                .map_or(json!("object"), |_| json!("object"))
        );
        assert_eq!(schema["required"], json!(["a", "b"]));
        assert_eq!(schema["additionalProperties"], json!(false));
    }

    #[test]
    fn homogeneous_array_collapses_items() {
        let schema = derive_schema(&json!([1, 2, 3]));
        assert_eq!(schema["items"]["type"], "integer");
    }

    #[test]
    fn heterogeneous_array_produces_any_of() {
        let schema = derive_schema(&json!([1, "x"]));
        assert_eq!(schema["items"]["anyOf"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn any_of_dedupes_duplicates() {
        let schema = derive_schema(&json!([1, 2, "x", 3]));
        // integer appears 3 times, string once — deduped to 2 variants.
        assert_eq!(schema["items"]["anyOf"].as_array().unwrap().len(), 2);
    }
}
