use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use stellar_xdr::curr::ReadXdr;
use wasmparser::{Parser, Payload};

type EnumDiscriminantChanges = BTreeMap<String, BTreeMap<(String, u32), (String, u32)>>;

#[derive(Clone, Debug, PartialEq)]
pub enum Compatibility {
    BackwardCompatible,
    CompatibleWithReview,
    PotentiallyBreaking,
    Breaking,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructField {
    pub name: String,
    pub type_discriminant: u32,
    pub type_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumVariant {
    pub name: String,
    pub discriminant: u32,
}

#[derive(Clone, Debug)]
pub struct ExtractedSpec {
    pub structs: BTreeMap<String, Vec<StructField>>,
    pub enums: BTreeMap<String, Vec<EnumVariant>>,
}

fn extract_spec_from_wasm(wasm: &[u8]) -> Result<ExtractedSpec, anyhow::Error> {
    let parser = Parser::new(0);
    let mut spec_data = None;
    for payload in parser.parse_all(wasm) {
        let payload = payload.map_err(|e| anyhow::anyhow!("failed to parse WASM: {e}"))?;
        if let Payload::CustomSection(custom) = &payload {
            if custom.name() == "contractspecv0" {
                spec_data = Some(custom.data().to_vec());
                break;
            }
        }
    }
    let spec_data = match spec_data {
        Some(data) => data,
        None => {
            return Err(anyhow::anyhow!(
                "WASM artifact does not contain a `contractspecv0` custom section"
            ))
        }
    };
    let cursor = Cursor::new(&spec_data);
    let entries_vec: Vec<stellar_xdr::curr::ScSpecEntry> =
        match stellar_xdr::curr::ScSpecEntry::read_xdr_iter(&mut stellar_xdr::curr::Limited::new(
            cursor,
            stellar_xdr::curr::Limits {
                depth: 500,
                len: 0x1000000,
            },
        ))
        .collect()
        {
            Ok(v) => v,
            Err(e) => bail!("failed to parse XDR spec: {}", e),
        };
    let mut structs: BTreeMap<String, Vec<StructField>> = BTreeMap::new();
    let mut enums: BTreeMap<String, Vec<EnumVariant>> = BTreeMap::new();
    for entry in &entries_vec {
        match entry {
            stellar_xdr::curr::ScSpecEntry::UdtStructV0(spec) => {
                let name = spec.name.to_utf8_string_lossy();
                let fields: Vec<StructField> = spec
                    .fields
                    .iter()
                    .map(|field| StructField {
                        name: field.name.to_utf8_string_lossy(),
                        type_discriminant: field.type_.discriminant() as u32,
                        type_name: normalized_type_name(&field.type_),
                    })
                    .collect();
                structs.insert(name, fields);
            }
            stellar_xdr::curr::ScSpecEntry::UdtEnumV0(spec) => {
                let name = spec.name.to_utf8_string_lossy();
                let variants = spec
                    .cases
                    .iter()
                    .map(|variant| EnumVariant {
                        name: variant.name.to_utf8_string_lossy(),
                        discriminant: variant.value,
                    })
                    .collect();
                enums.insert(name, variants);
            }
            stellar_xdr::curr::ScSpecEntry::UdtErrorEnumV0(spec) => {
                let name = spec.name.to_utf8_string_lossy();
                let variants = spec
                    .cases
                    .iter()
                    .map(|variant| EnumVariant {
                        name: variant.name.to_utf8_string_lossy(),
                        discriminant: variant.value,
                    })
                    .collect();
                enums.insert(name, variants);
            }
            _ => {}
        }
    }
    Ok(ExtractedSpec { structs, enums })
}

fn assess_compatibility(summary: &ChangeSummary) -> Compatibility {
    if !summary.deleted_struct_fields.is_empty() || !summary.deleted_enum_variants.is_empty() {
        Compatibility::Breaking
    } else if !summary.changed_struct_fields.is_empty()
        || !summary.changed_enum_discriminants.is_empty()
        || !summary.reordered_struct_fields.is_empty()
    {
        Compatibility::PotentiallyBreaking
    } else if !summary.added_struct_fields.is_empty() || !summary.added_enum_variants.is_empty() {
        Compatibility::CompatibleWithReview
    } else {
        Compatibility::BackwardCompatible
    }
}

fn compare_specs(source: &ExtractedSpec, target: &ExtractedSpec) -> ChangeSummary {
    let mut added_struct_fields: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut deleted_struct_fields: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut changed_struct_fields: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut reordered_struct_fields: BTreeMap<String, bool> = BTreeMap::new();
    let mut added_enum_variants: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut deleted_enum_variants: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut changed_enum_discriminants: EnumDiscriminantChanges = BTreeMap::new();
    let all_struct_names: BTreeSet<String> = {
        let a = source.structs.keys().cloned().collect::<Vec<String>>();
        let b = target.structs.keys().cloned().collect::<Vec<String>>();
        [a, b].concat().into_iter().collect()
    };
    for name in &all_struct_names {
        let source_fields: &[StructField] = source.structs.get(name).map_or(&[], |v| v.as_slice());
        let target_fields: &[StructField] = target.structs.get(name).map_or(&[], |v| v.as_slice());
        if source_fields.is_empty() && !target_fields.is_empty() {
            for field in target_fields {
                added_struct_fields
                    .entry(name.clone())
                    .or_default()
                    .insert(field.name.clone());
            }
        } else if !source_fields.is_empty() && target_fields.is_empty() {
            for field in source_fields {
                deleted_struct_fields
                    .entry(name.clone())
                    .or_default()
                    .insert(field.name.clone());
            }
        } else if !source_fields.is_empty() && !target_fields.is_empty() {
            let source_names: BTreeSet<&str> =
                source_fields.iter().map(|f| f.name.as_str()).collect();
            let target_names: BTreeSet<&str> =
                target_fields.iter().map(|f| f.name.as_str()).collect();
            for field in target_fields {
                if !source_names.contains(&field.name.as_str()) {
                    added_struct_fields
                        .entry(name.clone())
                        .or_default()
                        .insert(field.name.clone());
                }
            }
            for field in source_fields {
                if !target_names.contains(&field.name.as_str()) {
                    deleted_struct_fields
                        .entry(name.clone())
                        .or_default()
                        .insert(field.name.clone());
                }
            }
            for field in target_fields {
                if let Some(source_field) = source_fields.iter().find(|f| f.name == field.name) {
                    if field.type_discriminant != source_field.type_discriminant
                        || field.type_name != source_field.type_name
                    {
                        changed_struct_fields
                            .entry(name.clone())
                            .or_default()
                            .insert(
                                field.name.clone(),
                                format!("{} -> {}", source_field.type_name, field.type_name),
                            );
                    }
                }
            }
            let common_source: Vec<&str> = source_fields
                .iter()
                .map(|f| f.name.as_str())
                .filter(|name| target_names.contains(name))
                .collect();
            let common_target: Vec<&str> = target_fields
                .iter()
                .map(|f| f.name.as_str())
                .filter(|name| source_names.contains(name))
                .collect();
            if common_source != common_target {
                reordered_struct_fields.entry(name.clone()).or_insert(true);
            }
        }
    }
    let all_enum_names: BTreeSet<String> = {
        let a = source.enums.keys().cloned().collect::<Vec<String>>();
        let b = target.enums.keys().cloned().collect::<Vec<String>>();
        [a, b].concat().into_iter().collect()
    };
    for name in &all_enum_names {
        let source_variants = source.enums.get(name).cloned().unwrap_or_default();
        let target_variants = target.enums.get(name).cloned().unwrap_or_default();
        if source_variants.is_empty() && !target_variants.is_empty() {
            for variant in &target_variants {
                added_enum_variants
                    .entry(name.clone())
                    .or_default()
                    .insert(variant.name.clone());
            }
        } else if !source_variants.is_empty() && target_variants.is_empty() {
            for variant in &source_variants {
                deleted_enum_variants
                    .entry(name.clone())
                    .or_default()
                    .insert(variant.name.clone());
            }
        } else if !source_variants.is_empty() && !target_variants.is_empty() {
            let source_names: BTreeSet<&str> =
                source_variants.iter().map(|v| v.name.as_str()).collect();
            let target_names: BTreeSet<&str> =
                target_variants.iter().map(|v| v.name.as_str()).collect();
            for variant in &target_variants {
                if !source_names.contains(&variant.name.as_str()) {
                    added_enum_variants
                        .entry(name.clone())
                        .or_default()
                        .insert(variant.name.clone());
                }
            }
            for variant in &source_variants {
                if !target_names.contains(&variant.name.as_str()) {
                    deleted_enum_variants
                        .entry(name.clone())
                        .or_default()
                        .insert(variant.name.clone());
                }
            }
            for variant in &target_variants {
                if let Some(source_v) = source_variants.iter().find(|v| v.name == variant.name) {
                    if variant.discriminant != source_v.discriminant {
                        changed_enum_discriminants
                            .entry(name.clone())
                            .or_default()
                            .insert(
                                (variant.name.clone(), variant.discriminant),
                                (source_v.name.clone(), source_v.discriminant),
                            );
                    }
                }
            }
        }
    }
    let has_changes = !added_struct_fields.is_empty()
        || !deleted_struct_fields.is_empty()
        || !changed_struct_fields.is_empty()
        || !reordered_struct_fields.is_empty()
        || !added_enum_variants.is_empty()
        || !deleted_enum_variants.is_empty()
        || !changed_enum_discriminants.is_empty();
    ChangeSummary {
        added_struct_fields,
        deleted_struct_fields,
        changed_struct_fields,
        reordered_struct_fields,
        added_enum_variants,
        deleted_enum_variants,
        changed_enum_discriminants,
        has_changes,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ChangeSummary {
    pub added_struct_fields: BTreeMap<String, BTreeSet<String>>,
    pub deleted_struct_fields: BTreeMap<String, BTreeSet<String>>,
    pub changed_struct_fields: BTreeMap<String, BTreeMap<String, String>>,
    pub reordered_struct_fields: BTreeMap<String, bool>,
    pub added_enum_variants: BTreeMap<String, BTreeSet<String>>,
    pub deleted_enum_variants: BTreeMap<String, BTreeSet<String>>,
    pub changed_enum_discriminants: EnumDiscriminantChanges,
    pub has_changes: bool,
}

fn format_field(field: &StructField) -> String {
    format!("{} ({})", field.name, field.type_name)
}
fn format_variant(variant: &EnumVariant) -> String {
    format!("{} (discriminant {})", variant.name, variant.discriminant)
}

fn render_human_readable(
    source_artifact: &str,
    target_artifact: &str,
    summary: &ChangeSummary,
    compatibility: &Compatibility,
) -> String {
    let mut output = String::new();
    output.push_str(&format!("Source artifact: {}\n", source_artifact));
    output.push_str(&format!("Target artifact: {}\n\n", target_artifact));
    let compatibility_str = match compatibility {
        Compatibility::BackwardCompatible => {
            "Backward-compatible (no migration needed)".to_string()
        }
        Compatibility::CompatibleWithReview => {
            "Compatible with review (migration may be needed)".to_string()
        }
        Compatibility::PotentiallyBreaking => {
            "Potentially breaking changes (migration recommended)".to_string()
        }
        Compatibility::Breaking => "Breaking changes (migration required)".to_string(),
    };
    output.push_str(&format!("Compatibility: {}\n\n", compatibility_str));
    let change_count = summary
        .added_struct_fields
        .values()
        .map(|v| v.len())
        .sum::<usize>()
        + summary
            .deleted_struct_fields
            .values()
            .map(|v| v.len())
            .sum::<usize>()
        + summary
            .changed_struct_fields
            .values()
            .map(|v| v.len())
            .sum::<usize>()
        + summary.reordered_struct_fields.len()
        + summary
            .added_enum_variants
            .values()
            .map(|v| v.len())
            .sum::<usize>()
        + summary
            .deleted_enum_variants
            .values()
            .map(|v| v.len())
            .sum::<usize>()
        + summary
            .changed_enum_discriminants
            .values()
            .map(|v| v.len())
            .sum::<usize>();
    output.push_str(&format!(
        "Summary: {change_count} detected schema change(s)\n"
    ));
    if !summary.added_struct_fields.is_empty() || !summary.deleted_struct_fields.is_empty() {
        output.push_str("Struct field changes:\n");
        if !summary.added_struct_fields.is_empty() {
            output.push_str("  Added fields:\n");
            for (struct_name, fields) in &summary.added_struct_fields {
                output.push_str(&format!("    {}:\n", struct_name));
                for field in fields {
                    output.push_str(&format!(
                        "      - {}\n",
                        format_field(&StructField {
                            name: field.clone(),
                            type_discriminant: 0,
                            type_name: "unknown".to_string()
                        })
                    ));
                }
            }
        }
        if !summary.deleted_struct_fields.is_empty() {
            output.push_str("  Deleted fields:\n");
            for (struct_name, fields) in &summary.deleted_struct_fields {
                output.push_str(&format!("    {}:\n", struct_name));
                for field in fields {
                    output.push_str(&format!(
                        "      - {}\n",
                        format_field(&StructField {
                            name: field.clone(),
                            type_discriminant: 0,
                            type_name: "unknown".to_string()
                        })
                    ));
                }
            }
        }
        output.push('\n');
    }
    if !summary.changed_struct_fields.is_empty() {
        output.push_str("Changed struct field types:\n");
        for (struct_name, changes) in &summary.changed_struct_fields {
            output.push_str(&format!("  {}:\n", struct_name));
            for (field_name, change) in changes {
                output.push_str(&format!("    {}: {}\n", field_name, change));
            }
        }
        output.push('\n');
    }
    if !summary.reordered_struct_fields.is_empty() {
        output.push_str("Reordered struct fields:\n");
        for struct_name in summary.reordered_struct_fields.keys() {
            output.push_str(&format!("  {}: fields reordered\n", struct_name));
        }
        output.push('\n');
    }
    if !summary.added_enum_variants.is_empty() || !summary.deleted_enum_variants.is_empty() {
        output.push_str("Enum variant changes:\n");
        if !summary.added_enum_variants.is_empty() {
            output.push_str("  Added variants:\n");
            for (enum_name, variants) in &summary.added_enum_variants {
                output.push_str(&format!("    {}:\n", enum_name));
                for variant in variants {
                    output.push_str(&format!(
                        "      - {}\n",
                        format_variant(&EnumVariant {
                            name: variant.clone(),
                            discriminant: 0
                        })
                    ));
                }
            }
        }
        if !summary.deleted_enum_variants.is_empty() {
            output.push_str("  Deleted variants:\n");
            for (enum_name, variants) in &summary.deleted_enum_variants {
                output.push_str(&format!("    {}:\n", enum_name));
                for variant in variants {
                    output.push_str(&format!(
                        "      - {}\n",
                        format_variant(&EnumVariant {
                            name: variant.clone(),
                            discriminant: 0
                        })
                    ));
                }
            }
        }
        output.push('\n');
    }
    if !summary.changed_enum_discriminants.is_empty() {
        output.push_str("Changed enum discriminants:\n");
        for (enum_name, changes) in &summary.changed_enum_discriminants {
            output.push_str(&format!("  {}:\n", enum_name));
            for ((variant_name, new_d), (_, old_d)) in changes {
                output.push_str(&format!(
                    "    {}: discriminant changed from {} to {}\n",
                    variant_name, old_d, new_d
                ));
            }
        }
        output.push('\n');
    }
    if change_count == 0 {
        output.push_str("  No differences established by the available contract specifications.\n");
        output.push_str(
            "  Actual on-chain persistent/instance storage compatibility remains unknown.\n\n",
        );
    }
    output
}

fn render_json(
    source_artifact: &str,
    target_artifact: &str,
    summary: &ChangeSummary,
    compatibility: &Compatibility,
) -> Result<String> {
    let compatibility_str = match compatibility {
        Compatibility::BackwardCompatible => "backward-compatible",
        Compatibility::CompatibleWithReview => "compatible-with-review",
        Compatibility::PotentiallyBreaking => "potentially-breaking",
        Compatibility::Breaking => "breaking",
    };
    let mut warnings = Vec::new();
    if !summary.deleted_struct_fields.is_empty() || !summary.deleted_enum_variants.is_empty() {
        warnings.push("Removed schema members may invalidate existing serialized persistent or instance values.".to_string());
    }
    if !summary.changed_struct_fields.is_empty() || !summary.changed_enum_discriminants.is_empty() {
        warnings.push("Changed field types or enum discriminants may invalidate existing encoded values; review every stored record before upgrade.".to_string());
    }
    if !summary.reordered_struct_fields.is_empty() {
        warnings.push("Reordered struct fields may change positional or map decoding assumptions; migration review is required.".to_string());
    }
    if !summary.added_struct_fields.is_empty() || !summary.added_enum_variants.is_empty() {
        warnings.push("Added schema members require migration review; additions may affect application fallback logic and are not proof of safe on-chain storage compatibility.".to_string());
    }
    let output = StorageDiffJson {
        schema_version: "soroban-forge/storage-diff@1",
        source_artifact: source_artifact.to_string(),
        target_artifact: target_artifact.to_string(),
        storage_keys_verified: false,
        added_storage_keys: Vec::new(),
        deleted_storage_keys: Vec::new(),
        added_struct_fields: summary.added_struct_fields.iter().map(|(k, v)| (k.clone(), v.iter().cloned().collect())).collect(),
        deleted_struct_fields: summary.deleted_struct_fields.iter().map(|(k, v)| (k.clone(), v.iter().cloned().collect())).collect(),
        changed_struct_fields: summary.changed_struct_fields.clone(),
        reordered_struct_fields: summary.reordered_struct_fields.iter().map(|(k, v)| (k.clone(), vec![v.to_string()])).collect(),
        added_enum_variants: summary.added_enum_variants.iter().map(|(k, v)| (k.clone(), v.iter().cloned().collect())).collect(),
        deleted_enum_variants: summary.deleted_enum_variants.iter().map(|(k, v)| (k.clone(), v.iter().cloned().collect())).collect(),
        changed_enum_discriminants: summary.changed_enum_discriminants.iter().map(|(k, changes)| (k.clone(), changes.iter().map(|((name, new), (_, old))| format!("{name}:{old}->{new}")).collect())).collect(),
        compatibility: compatibility_str,
        warnings,
        verification_limitations: vec![
            "The contract specification does not prove concrete instance or persistent storage key names, key namespaces, or on-chain layout.".to_string(),
            "The command verifies decoded UDT and enum metadata only; actual on-chain storage compatibility remains unknown until existing keys and values are inspected.".to_string(),
        ],
        extraction_errors: Vec::new(),
    };
    Ok(serde_json::to_string(&output)?)
}

#[derive(Clone, Debug, Serialize)]
pub struct StorageDiffJson {
    pub schema_version: &'static str,
    pub source_artifact: String,
    pub target_artifact: String,
    pub storage_keys_verified: bool,
    pub added_storage_keys: Vec<String>,
    pub deleted_storage_keys: Vec<String>,
    pub added_struct_fields: BTreeMap<String, Vec<String>>,
    pub deleted_struct_fields: BTreeMap<String, Vec<String>>,
    pub changed_struct_fields: BTreeMap<String, BTreeMap<String, String>>,
    pub reordered_struct_fields: BTreeMap<String, Vec<String>>,
    pub added_enum_variants: BTreeMap<String, Vec<String>>,
    pub deleted_enum_variants: BTreeMap<String, Vec<String>>,
    pub changed_enum_discriminants: BTreeMap<String, Vec<String>>,
    pub compatibility: &'static str,
    pub warnings: Vec<String>,
    pub verification_limitations: Vec<String>,
    pub extraction_errors: Vec<String>,
}

pub fn run(args: crate::cli::StorageDiffArgs) -> Result<()> {
    let source_wasm = std::fs::read(&args.source)
        .with_context(|| format!("failed to read source WASM: {}", args.source.display()))?;
    if source_wasm.is_empty() {
        bail!("Source WASM artifact is empty: {}", args.source.display());
    }
    let target_wasm = std::fs::read(&args.target)
        .with_context(|| format!("failed to read target WASM: {}", args.target.display()))?;
    if target_wasm.is_empty() {
        bail!("Target WASM artifact is empty: {}", args.target.display());
    }
    let source_spec = extract_spec_from_wasm(&source_wasm).with_context(|| {
        format!(
            "failed to extract spec from source WASM: {}",
            args.source.display()
        )
    })?;
    let target_spec = extract_spec_from_wasm(&target_wasm).with_context(|| {
        format!(
            "failed to extract spec from target WASM: {}",
            args.target.display()
        )
    })?;
    let summary: ChangeSummary = compare_specs(&source_spec, &target_spec);
    let compatibility = assess_compatibility(&summary);
    if args.json {
        let json_output = render_json(
            &args.source.to_string_lossy(),
            &args.target.to_string_lossy(),
            &summary,
            &compatibility,
        )?;
        println!("{}", json_output);
    } else {
        let source_name = args
            .source
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        let target_name = args
            .target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        println!(
            "{}",
            render_human_readable(source_name, target_name, &summary, &compatibility)
        );
    }
    Ok(())
}

fn normalized_type_name(type_: &stellar_xdr::curr::ScSpecTypeDef) -> String {
    match type_ {
        stellar_xdr::curr::ScSpecTypeDef::Udt(udt) => {
            format!("udt:{}", udt.name.to_utf8_string_lossy())
        }
        stellar_xdr::curr::ScSpecTypeDef::Option(inner) => {
            format!("option<{}>", normalized_type_name(&inner.value_type))
        }
        stellar_xdr::curr::ScSpecTypeDef::Result(inner) => format!(
            "result<{},{}>",
            normalized_type_name(&inner.ok_type),
            normalized_type_name(&inner.error_type)
        ),
        stellar_xdr::curr::ScSpecTypeDef::Vec(inner) => {
            format!("vec<{}>", normalized_type_name(&inner.element_type))
        }
        stellar_xdr::curr::ScSpecTypeDef::Map(inner) => format!(
            "map<{},{}>",
            normalized_type_name(&inner.key_type),
            normalized_type_name(&inner.value_type)
        ),
        stellar_xdr::curr::ScSpecTypeDef::BytesN(inner) => format!("bytesN<{}>", inner.n),
        stellar_xdr::curr::ScSpecTypeDef::Tuple(inner) => {
            let types: Vec<String> = inner.value_types.iter().map(normalized_type_name).collect();
            format!("tuple<{}>", types.join(","))
        }
        other => format!("{other:?}")
            .split(['(', '{', ' '])
            .next()
            .unwrap_or("unknown")
            .to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(struct_fields: &[(&str, &str)], enums: &[(&str, u32)]) -> ExtractedSpec {
        let mut structs = BTreeMap::new();
        if !struct_fields.is_empty() {
            structs.insert(
                "Record".to_string(),
                struct_fields
                    .iter()
                    .map(|(name, type_name)| StructField {
                        name: (*name).to_string(),
                        type_discriminant: 1,
                        type_name: (*type_name).to_string(),
                    })
                    .collect(),
            );
        }
        let mut enum_map = BTreeMap::new();
        if !enums.is_empty() {
            enum_map.insert(
                "State".to_string(),
                enums
                    .iter()
                    .map(|(name, discriminant)| EnumVariant {
                        name: (*name).to_string(),
                        discriminant: *discriminant,
                    })
                    .collect(),
            );
        }
        ExtractedSpec {
            structs,
            enums: enum_map,
        }
    }

    fn leb128(mut value: usize) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                break;
            }
        }
        out
    }

    fn minimal_wasm(custom_name: Option<&str>, custom_data: &[u8]) -> Vec<u8> {
        let mut wasm = b"\0asm\x01\0\0\0".to_vec();
        if let Some(name) = custom_name {
            let mut payload = Vec::new();
            payload.extend(leb128(name.len()));
            payload.extend_from_slice(name.as_bytes());
            payload.extend_from_slice(custom_data);
            wasm.push(0);
            wasm.extend(leb128(payload.len()));
            wasm.extend_from_slice(&payload);
        }
        wasm
    }

    #[test]
    fn identical_schemas_are_backward_compatible() {
        let source = spec(&[("id", "U64")], &[("Open", 0), ("Closed", 1)]);
        let target = spec(&[("id", "U64")], &[("Open", 0), ("Closed", 1)]);
        let summary = compare_specs(&source, &target);
        assert!(!summary.has_changes);
        assert_eq!(
            assess_compatibility(&summary),
            Compatibility::BackwardCompatible
        );
    }

    #[test]
    fn detects_added_removed_reordered_and_changed_fields() {
        let source = spec(&[("id", "U64"), ("owner", "Address")], &[]);
        let target = spec(
            &[("owner", "Address"), ("id", "U128"), ("active", "Bool")],
            &[],
        );
        let summary = compare_specs(&source, &target);
        assert!(summary.deleted_struct_fields.is_empty());
        assert_eq!(
            summary.added_struct_fields["Record"]
                .iter()
                .next()
                .map(String::as_str),
            Some("active")
        );
        assert!(summary.changed_struct_fields["Record"].contains_key("id"));
        assert!(summary.reordered_struct_fields.contains_key("Record"));
    }

    #[test]
    fn detects_added_removed_enum_variants_and_discriminants() {
        let source = spec(&[], &[("Open", 0), ("Closed", 1)]);
        let target = spec(&[], &[("Open", 1), ("Paused", 2)]);
        let summary = compare_specs(&source, &target);
        assert!(summary.deleted_enum_variants["State"].contains("Closed"));
        assert!(summary.added_enum_variants["State"].contains("Paused"));
        assert!(!summary.changed_enum_discriminants["State"].is_empty());
        assert_eq!(assess_compatibility(&summary), Compatibility::Breaking);
    }

    #[test]
    fn added_enum_variant_requires_review_not_unconditional_safety() {
        let source = spec(&[], &[("Open", 0)]);
        let target = spec(&[], &[("Open", 0), ("Paused", 1)]);
        let summary = compare_specs(&source, &target);
        assert_eq!(
            assess_compatibility(&summary),
            Compatibility::CompatibleWithReview
        );
    }

    #[test]
    fn multiple_changes_are_reported_together() {
        let source = spec(&[("id", "U64")], &[("Open", 0)]);
        let target = spec(&[("name", "Symbol")], &[("Open", 2), ("Paused", 3)]);
        let summary = compare_specs(&source, &target);
        assert!(summary.deleted_struct_fields.contains_key("Record"));
        assert!(summary.added_struct_fields.contains_key("Record"));
        assert!(summary.changed_enum_discriminants.contains_key("State"));
        assert!(summary.added_enum_variants.contains_key("State"));
        assert_eq!(assess_compatibility(&summary), Compatibility::Breaking);
    }

    #[test]
    fn extraction_rejects_invalid_wasm() {
        let err = extract_spec_from_wasm(b"not wasm").unwrap_err();
        assert!(
            err.to_string().contains("failed to parse WASM") || err.to_string().contains("WASM")
        );
    }

    #[test]
    fn extraction_reports_missing_contract_spec_section() {
        let wasm = minimal_wasm(Some("other"), b"data");
        let err = extract_spec_from_wasm(&wasm).unwrap_err();
        assert!(err.to_string().contains("contractspecv0"));
    }

    #[test]
    fn extraction_reports_malformed_contract_spec_xdr() {
        let wasm = minimal_wasm(Some("contractspecv0"), &[0xff, 0xff, 0xff]);
        let err = extract_spec_from_wasm(&wasm).unwrap_err();
        assert!(err.to_string().contains("XDR"));
    }

    #[test]
    fn json_output_is_valid_and_deterministic() {
        let source = spec(&[("id", "U64")], &[("Open", 0)]);
        let target = spec(
            &[("id", "U128"), ("name", "Symbol")],
            &[("Open", 0), ("Paused", 1)],
        );
        let summary = compare_specs(&source, &target);
        let compatibility = assess_compatibility(&summary);
        let first = render_json("source.wasm", "target.wasm", &summary, &compatibility).unwrap();
        let second = render_json("source.wasm", "target.wasm", &summary, &compatibility).unwrap();
        assert_eq!(first, second);
        let value: serde_json::Value = serde_json::from_str(&first).unwrap();
        assert_eq!(value["schema_version"], "soroban-forge/storage-diff@1");
        assert!(value["verification_limitations"].is_array());
        assert!(value["warnings"].is_array());
    }

    #[test]
    fn extracts_real_contract_spec_xdr_from_wasm_section() {
        use stellar_xdr::curr::{
            Limits, ScSpecEntry, ScSpecTypeDef, ScSpecUdtStructFieldV0, ScSpecUdtStructV0, StringM,
            WriteXdr,
        };

        let field: ScSpecUdtStructFieldV0 = ScSpecUdtStructFieldV0 {
            doc: StringM::default(),
            name: "id".try_into().unwrap(),
            type_: ScSpecTypeDef::U64,
        };
        let entry = ScSpecEntry::UdtStructV0(ScSpecUdtStructV0 {
            doc: StringM::default(),
            lib: StringM::default(),
            name: "Record".try_into().unwrap(),
            fields: vec![field].try_into().unwrap(),
        });
        let xdr = entry.to_xdr(Limits::none()).unwrap();
        let wasm = minimal_wasm(Some("contractspecv0"), &xdr);

        let extracted = extract_spec_from_wasm(&wasm).unwrap();
        assert_eq!(extracted.structs["Record"][0].name, "id");
        assert_eq!(extracted.structs["Record"][0].type_name, "U64");
    }
}
