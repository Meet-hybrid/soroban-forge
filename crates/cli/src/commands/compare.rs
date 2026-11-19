use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;

use crate::cli::CompareArgs;
use soroban_spec::read;
use stellar_xdr_28::{ScSpecEntry, ScSpecTypeDef};

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct AbiChange {
    pub category: String,
    pub classification: String,
    pub kind: String,
    pub name: String,
    pub detail: String,
}

#[derive(Serialize)]
struct AbiReport {
    old: String,
    new: String,
    identical: bool,
    changes: Vec<AbiChange>,
}

pub fn run(args: CompareArgs) -> Result<()> {
    let old_spec =
        extract(&args.old).with_context(|| format!("failed to inspect {}", args.old.display()))?;
    let new_spec =
        extract(&args.new).with_context(|| format!("failed to inspect {}", args.new.display()))?;
    let report = diff(
        &args.old.display().to_string(),
        &args.new.display().to_string(),
        &old_spec,
        &new_spec,
    );

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_human(&report);
    }

    if report.changes.iter().any(|change| {
        matches!(
            change.classification.as_str(),
            "source-breaking" | "wire/storage-risky"
        )
    }) {
        anyhow::bail!("ABI compatibility check failed: breaking or risky changes detected");
    }
    Ok(())
}

fn extract(path: &std::path::Path) -> Result<BTreeMap<String, String>> {
    let wasm = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let entries = read::from_wasm(&wasm).context("contract spec not found or invalid")?;
    let mut spec = BTreeMap::new();
    for entry in entries {
        match entry {
            ScSpecEntry::FunctionV0(v) => {
                let name = v.name.to_string();
                let inputs = v
                    .inputs
                    .iter()
                    .map(|p| format!("{}:{}", p.name, type_name(&p.type_)))
                    .collect::<Vec<_>>()
                    .join(",");
                let outputs = v
                    .outputs
                    .iter()
                    .map(type_name)
                    .collect::<Vec<_>>()
                    .join(",");
                spec.insert(format!("function:{name}"), format!("({inputs})->{outputs}"));
            }
            ScSpecEntry::UdtStructV0(v) => {
                let fields = v
                    .fields
                    .iter()
                    .map(|f| format!("{}:{}", f.name, type_name(&f.type_)))
                    .collect::<Vec<_>>()
                    .join(",");
                spec.insert(format!("type:struct:{}", v.name), format!("{{{fields}}}"));
            }
            ScSpecEntry::UdtUnionV0(v) => {
                let cases = v
                    .cases
                    .iter()
                    .map(|c| match c {
                        stellar_xdr_28::ScSpecUdtUnionCaseV0::VoidV0(x) => x.name.to_string(),
                        stellar_xdr_28::ScSpecUdtUnionCaseV0::TupleV0(x) => format!(
                            "{}({})",
                            x.name,
                            x.type_.iter().map(type_name).collect::<Vec<_>>().join(",")
                        ),
                    })
                    .collect::<Vec<_>>();
                spec.insert(
                    format!("type:union:{}", v.name),
                    format!("[{}]", cases.join(",")),
                );
            }
            ScSpecEntry::UdtEnumV0(v) => {
                let cases = v
                    .cases
                    .iter()
                    .map(|c| format!("{}={}", c.name, c.value))
                    .collect::<Vec<_>>();
                spec.insert(
                    format!("type:enum:{}", v.name),
                    format!("[{}]", cases.join(",")),
                );
            }
            ScSpecEntry::UdtErrorEnumV0(v) => {
                let cases = v
                    .cases
                    .iter()
                    .map(|c| format!("{}={}", c.name, c.value))
                    .collect::<Vec<_>>();
                spec.insert(
                    format!("type:error:{}", v.name),
                    format!("[{}]", cases.join(",")),
                );
            }
            ScSpecEntry::EventV0(v) => {
                let params = v
                    .params
                    .iter()
                    .map(|p| format!("{}:{}:{:?}", p.name, type_name(&p.type_), p.location))
                    .collect::<Vec<_>>()
                    .join(",");
                spec.insert(format!("event:{}", v.name.0), format!("{{{params}}}"));
            }
        }
    }
    Ok(spec)
}

fn type_name(t: &ScSpecTypeDef) -> String {
    match t {
        ScSpecTypeDef::Option(v) => format!("option<{}>", type_name(&v.value_type)),
        ScSpecTypeDef::Result(v) => format!(
            "result<{},{}>",
            type_name(&v.ok_type),
            type_name(&v.error_type)
        ),
        ScSpecTypeDef::Vec(v) => format!("vec<{}>", type_name(&v.element_type)),
        ScSpecTypeDef::Map(v) => format!(
            "map<{},{}>",
            type_name(&v.key_type),
            type_name(&v.value_type)
        ),
        ScSpecTypeDef::Tuple(v) => format!(
            "tuple<{}>",
            v.value_types
                .iter()
                .map(type_name)
                .collect::<Vec<_>>()
                .join(",")
        ),
        ScSpecTypeDef::BytesN(v) => format!("bytes_n<{}>", v.n),
        ScSpecTypeDef::Udt(v) => format!("udt<{}>", v.name),
        _ => t.name().to_owned(),
    }
}

fn diff(
    old: &str,
    new: &str,
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> AbiReport {
    let mut changes = Vec::new();

    for key in before.keys().filter(|k| !after.contains_key(*k)) {
        let (kind, name) = split_key(key);
        let detail = before[key].clone();
        let category = "removed";
        changes.push(change(category, "source-breaking", kind, name, detail));
    }
    for key in after.keys().filter(|k| !before.contains_key(*k)) {
        let (kind, name) = split_key(key);
        changes.push(change("added", "additive", kind, name, after[key].clone()));
    }

    for key in before.keys().filter(|k| after.contains_key(*k)) {
        if before[key] != after[key] {
            let (kind, name) = split_key(key);
            let classification = if kind == "type" || kind == "event" {
                "wire/storage-risky"
            } else {
                "source-breaking"
            };
            if kind == "function" && is_reordered_signature(&before[key], &after[key]) {
                changes.push(change(
                    "reordered",
                    classification,
                    kind,
                    name,
                    format!("{} -> {}", before[key], after[key]),
                ));
            } else {
                let detail = format!("{} -> {}", before[key], after[key]);
                changes.push(change("type-changed", classification, kind, name, detail));
            }
        }
    }

    // Detect signature-preserving renames only when one old and one new entry can be paired uniquely.
    let removed: Vec<_> = before
        .keys()
        .filter(|k| k.starts_with("function:") && !after.contains_key(*k))
        .cloned()
        .collect();
    let added: Vec<_> = after
        .keys()
        .filter(|k| k.starts_with("function:") && !before.contains_key(*k))
        .cloned()
        .collect();
    for old_key in &removed {
        if let Some(new_key) = added.iter().find(|key| before[old_key] == after[*key]) {
            let (_, old_name) = split_key(old_key);
            let (_, new_name) = split_key(new_key);
            changes.push(change(
                "renamed",
                "source-breaking",
                "function",
                old_name,
                format!("{old_name} -> {new_name}"),
            ));
            changes.retain(|c| {
                !((c.category == "removed" && c.name == old_name)
                    || (c.category == "added" && c.name == new_name))
            });
        }
    }

    changes.sort_by(|a, b| {
        (&a.kind, &a.name, &a.category, &a.detail).cmp(&(&b.kind, &b.name, &b.category, &b.detail))
    });
    AbiReport {
        old: old.to_owned(),
        new: new.to_owned(),
        identical: changes.is_empty(),
        changes,
    }
}

fn split_key(key: &str) -> (&str, &str) {
    let mut parts = key.splitn(2, ':');
    (parts.next().unwrap_or("type"), parts.next().unwrap_or(key))
}

fn change(
    category: &str,
    classification: &str,
    kind: &str,
    name: &str,
    detail: String,
) -> AbiChange {
    AbiChange {
        category: category.to_owned(),
        classification: classification.to_owned(),
        kind: kind.to_owned(),
        name: name.to_owned(),
        detail,
    }
}

fn print_human(report: &AbiReport) {
    println!("ABI compatibility report");
    println!("  old: {}", report.old);
    println!("  new: {}", report.new);
    if report.identical {
        println!("  identical: true");
        println!("  changes: none");
        return;
    }
    println!("  identical: false");
    for change in &report.changes {
        println!(
            "  {} | {} | {} {}: {}",
            change.category, change.classification, change.kind, change.name, change.detail
        );
    }
}

fn is_reordered_signature(old: &str, new: &str) -> bool {
    fn inputs(signature: &str) -> Option<Vec<&str>> {
        let inside = signature.strip_prefix('(')?.split_once(")->")?.0;
        if inside.is_empty() {
            return Some(Vec::new());
        }
        Some(inside.split(',').collect())
    }
    let (Some(mut old_inputs), Some(mut new_inputs)) = (inputs(old), inputs(new)) else {
        return false;
    };
    old_inputs.sort_unstable();
    new_inputs.sort_unstable();
    old_inputs == new_inputs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn identical_specs_are_empty() {
        let s = spec(&[("function:ping", "() -> U32")]);
        let report = diff("old", "new", &s, &s);
        assert!(report.identical);
        assert!(report.changes.is_empty());
    }

    #[test]
    fn added_removed_and_changed_are_classified() {
        let old = spec(&[("function:a", "() -> U32"), ("type:struct:Old", "{x:U32}")]);
        let new = spec(&[("function:a", "() -> U64"), ("event:Paid", "{amount:U128}")]);
        let report = diff("old", "new", &old, &new);
        assert!(!report.identical);
        assert!(report
            .changes
            .iter()
            .any(|c| c.category == "removed" && c.classification == "source-breaking"));
        assert!(report
            .changes
            .iter()
            .any(|c| c.category == "added" && c.classification == "additive"));
        assert!(report.changes.iter().any(|c| c.category == "type-changed"));
    }

    #[test]
    fn output_is_stably_sorted() {
        let old = spec(&[("function:z", "()"), ("function:a", "()")]);
        let new = spec(&[("function:z", "(x:U32)"), ("function:b", "()")]);
        let report = diff("old", "new", &old, &new);
        for pair in report.changes.windows(2) {
            assert!(
                format!(
                    "{}:{}:{}:{}",
                    pair[0].kind, pair[0].name, pair[0].category, pair[0].detail
                ) <= format!(
                    "{}:{}:{}:{}",
                    pair[1].kind, pair[1].name, pair[1].category, pair[1].detail
                )
            );
        }
    }
}
