use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use stellar_xdr::curr::{Limits, ReadXdr, ScSpecEntry};
use wasmparser::{Parser, Payload};

use crate::cli::StorageDiffArgs;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Compatibility {
    BackwardCompatible,
    CompatibleWithReview,
    PotentiallyBreaking,
    Breaking,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructField {
    pub name: String,
    pub type_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumVariant {
    pub name: String,
    pub discriminant: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtractedSpec {
    pub structs: BTreeMap<String, Vec<StructField>>,
    pub enums: BTreeMap<String, Vec<EnumVariant>>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct ChangeSummary {
    pub added_struct_fields: BTreeMap<String, Vec<String>>,
    pub deleted_struct_fields: BTreeMap<String, Vec<String>>,
    pub changed_struct_fields: BTreeMap<String, BTreeMap<String, String>>,
    pub reordered_struct_fields: Vec<String>,
    pub added_enum_variants: BTreeMap<String, Vec<String>>,
    pub deleted_enum_variants: BTreeMap<String, Vec<String>>,
    pub changed_enum_discriminants: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StorageDiffJson {
    pub schema_version: &'static str,
    pub source_artifact: String,
    pub target_artifact: String,
    pub storage_keys_verified: bool,
    pub added_storage_keys: Vec<String>,
    pub deleted_storage_keys: Vec<String>,
    pub struct_changes: ChangeSummary,
    pub compatibility: Compatibility,
    pub warnings: Vec<String>,
    pub verification_limitations: Vec<String>,
    pub extraction_errors: Vec<String>,
}

fn type_name(type_: &stellar_xdr::curr::ScSpecTypeDef) -> String {
    match type_ {
        stellar_xdr::curr::ScSpecTypeDef::Udt(v) => {
            format!("udt:{}", v.name.to_utf8_string_lossy())
        }
        stellar_xdr::curr::ScSpecTypeDef::Option(v) => {
            format!("option<{}>", type_name(&v.value_type))
        }
        stellar_xdr::curr::ScSpecTypeDef::Vec(v) => format!("vec<{}>", type_name(&v.element_type)),
        stellar_xdr::curr::ScSpecTypeDef::Map(v) => format!(
            "map<{},{}>",
            type_name(&v.key_type),
            type_name(&v.value_type)
        ),
        stellar_xdr::curr::ScSpecTypeDef::BytesN(v) => format!("bytesN<{}>", v.n),
        other => format!("{other:?}")
            .split(['(', '{', ' '])
            .next()
            .unwrap_or("unknown")
            .to_owned(),
    }
}

pub fn extract_spec_from_wasm(wasm: &[u8]) -> Result<ExtractedSpec> {
    let mut data = None;
    for payload in Parser::new(0).parse_all(wasm) {
        let payload = payload.map_err(|error| anyhow::anyhow!("invalid WASM: {error}"))?;
        if let Payload::CustomSection(section) = payload {
            if section.name() == "contractspecv0" {
                data = Some(section.data().to_vec());
                break;
            }
        }
    }
    let data = data.ok_or_else(|| anyhow::anyhow!("missing `contractspecv0` custom section"))?;
    let mut limited = stellar_xdr::curr::Limited::new(
        Cursor::new(data),
        Limits {
            depth: 500,
            len: 0x1000000,
        },
    );
    let entries: Vec<ScSpecEntry> = ScSpecEntry::read_xdr_iter(&mut limited)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| anyhow::anyhow!("malformed contract specification XDR: {error}"))?;
    let mut result = ExtractedSpec::default();
    for entry in entries {
        match entry {
            ScSpecEntry::UdtStructV0(spec) => {
                let fields = spec
                    .fields
                    .iter()
                    .map(|field| StructField {
                        name: field.name.to_utf8_string_lossy(),
                        type_name: type_name(&field.type_),
                    })
                    .collect();
                result
                    .structs
                    .insert(spec.name.to_utf8_string_lossy(), fields);
            }
            ScSpecEntry::UdtEnumV0(spec) => {
                let variants = spec
                    .cases
                    .iter()
                    .map(|variant| EnumVariant {
                        name: variant.name.to_utf8_string_lossy(),
                        discriminant: variant.value,
                    })
                    .collect();
                result
                    .enums
                    .insert(spec.name.to_utf8_string_lossy(), variants);
            }
            ScSpecEntry::UdtErrorEnumV0(spec) => {
                let variants = spec
                    .cases
                    .iter()
                    .map(|variant| EnumVariant {
                        name: variant.name.to_utf8_string_lossy(),
                        discriminant: variant.value,
                    })
                    .collect();
                result
                    .enums
                    .insert(spec.name.to_utf8_string_lossy(), variants);
            }
            _ => {}
        }
    }
    Ok(result)
}

pub fn compare_specs(source: &ExtractedSpec, target: &ExtractedSpec) -> ChangeSummary {
    let mut changes = ChangeSummary::default();
    for name in source
        .structs
        .keys()
        .chain(target.structs.keys())
        .collect::<BTreeSet<_>>()
    {
        let source_fields = source.structs.get(name).map(Vec::as_slice).unwrap_or(&[]);
        let target_fields = target.structs.get(name).map(Vec::as_slice).unwrap_or(&[]);
        let source_names: BTreeSet<&str> = source_fields
            .iter()
            .map(|field| field.name.as_str())
            .collect();
        let target_names: BTreeSet<&str> = target_fields
            .iter()
            .map(|field| field.name.as_str())
            .collect();
        let deleted: Vec<String> = source_fields
            .iter()
            .filter(|field| !target_names.contains(field.name.as_str()))
            .map(|field| field.name.clone())
            .collect();
        if !deleted.is_empty() {
            changes.deleted_struct_fields.insert(name.clone(), deleted);
        }
        let added: Vec<String> = target_fields
            .iter()
            .filter(|field| !source_names.contains(field.name.as_str()))
            .map(|field| field.name.clone())
            .collect();
        if !added.is_empty() {
            changes.added_struct_fields.insert(name.clone(), added);
        }
        let mut changed = BTreeMap::new();
        for target_field in target_fields {
            if let Some(source_field) = source_fields
                .iter()
                .find(|field| field.name == target_field.name)
            {
                if source_field.type_name != target_field.type_name {
                    changed.insert(
                        target_field.name.clone(),
                        format!("{} -> {}", source_field.type_name, target_field.type_name),
                    );
                }
            }
        }
        if !changed.is_empty() {
            changes.changed_struct_fields.insert(name.clone(), changed);
        }
        let common_source: Vec<&str> = source_fields
            .iter()
            .map(|field| field.name.as_str())
            .filter(|field| target_names.contains(field))
            .collect();
        let common_target: Vec<&str> = target_fields
            .iter()
            .map(|field| field.name.as_str())
            .filter(|field| source_names.contains(field))
            .collect();
        if common_source != common_target {
            changes.reordered_struct_fields.push(name.clone());
        }
    }
    changes.reordered_struct_fields.sort();
    for name in source
        .enums
        .keys()
        .chain(target.enums.keys())
        .collect::<BTreeSet<_>>()
    {
        let source_variants = source.enums.get(name).map(Vec::as_slice).unwrap_or(&[]);
        let target_variants = target.enums.get(name).map(Vec::as_slice).unwrap_or(&[]);
        let source_names: BTreeSet<&str> = source_variants
            .iter()
            .map(|variant| variant.name.as_str())
            .collect();
        let target_names: BTreeSet<&str> = target_variants
            .iter()
            .map(|variant| variant.name.as_str())
            .collect();
        let deleted: Vec<String> = source_variants
            .iter()
            .filter(|variant| !target_names.contains(variant.name.as_str()))
            .map(|variant| variant.name.clone())
            .collect();
        if !deleted.is_empty() {
            changes.deleted_enum_variants.insert(name.clone(), deleted);
        }
        let added: Vec<String> = target_variants
            .iter()
            .filter(|variant| !source_names.contains(variant.name.as_str()))
            .map(|variant| variant.name.clone())
            .collect();
        if !added.is_empty() {
            changes.added_enum_variants.insert(name.clone(), added);
        }
        let mut changed = Vec::new();
        for target_variant in target_variants {
            if let Some(source_variant) = source_variants
                .iter()
                .find(|variant| variant.name == target_variant.name)
            {
                if source_variant.discriminant != target_variant.discriminant {
                    changed.push(format!(
                        "{}: {} -> {}",
                        target_variant.name,
                        source_variant.discriminant,
                        target_variant.discriminant
                    ));
                }
            }
        }
        if !changed.is_empty() {
            changes
                .changed_enum_discriminants
                .insert(name.clone(), changed);
        }
    }
    changes
}

pub fn classify(changes: &ChangeSummary) -> Compatibility {
    if !changes.deleted_struct_fields.is_empty() || !changes.deleted_enum_variants.is_empty() {
        Compatibility::Breaking
    } else if !changes.changed_struct_fields.is_empty()
        || !changes.changed_enum_discriminants.is_empty()
        || !changes.reordered_struct_fields.is_empty()
    {
        Compatibility::PotentiallyBreaking
    } else if !changes.added_struct_fields.is_empty() || !changes.added_enum_variants.is_empty() {
        Compatibility::CompatibleWithReview
    } else {
        Compatibility::BackwardCompatible
    }
}

pub fn run(args: StorageDiffArgs) -> Result<()> {
    if !args.source.exists() {
        bail!("source WASM artifact not found: {}", args.source.display());
    }
    if !args.target.exists() {
        bail!("target WASM artifact not found: {}", args.target.display());
    }
    let source = extract_spec_from_wasm(
        &std::fs::read(&args.source)
            .with_context(|| format!("failed to read source WASM: {}", args.source.display()))?,
    )
    .with_context(|| {
        format!(
            "failed to extract source contract specification: {}",
            args.source.display()
        )
    })?;
    let target = extract_spec_from_wasm(
        &std::fs::read(&args.target)
            .with_context(|| format!("failed to read target WASM: {}", args.target.display()))?,
    )
    .with_context(|| {
        format!(
            "failed to extract target contract specification: {}",
            args.target.display()
        )
    })?;
    let changes = compare_specs(&source, &target);
    let compatibility = classify(&changes);
    let mut warnings = vec!["contractspecv0 does not prove complete instance/persistent storage layout; manual review remains required.".to_string()];
    if compatibility == Compatibility::Breaking {
        warnings.insert(
            0,
            "Breaking storage schema change detected; migration review is required before upgrade."
                .to_string(),
        );
    }
    if compatibility == Compatibility::PotentiallyBreaking {
        warnings.insert(
            0,
            "Potential storage compatibility risk detected; migration review is required."
                .to_string(),
        );
    }
    let output = StorageDiffJson {
        schema_version: "soroban-forge/storage-diff@1",
        source_artifact: args.source.to_string_lossy().into_owned(),
        target_artifact: args.target.to_string_lossy().into_owned(),
        storage_keys_verified: false,
        added_storage_keys: Vec::new(),
        deleted_storage_keys: Vec::new(),
        struct_changes: changes,
        compatibility,
        warnings,
        verification_limitations: vec![
            "Concrete storage key names and namespaces remain unknown.".to_string(),
            "Existing persistent and instance entries require manual upgrade validation."
                .to_string(),
        ],
        extraction_errors: Vec::new(),
    };
    if args.json {
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!(
            "Storage diff: {} -> {}",
            output.source_artifact, output.target_artifact
        );
        println!("Compatibility: {:?}", output.compatibility);
        for warning in &output.warnings {
            println!("Warning: {warning}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(fields: &[(&str, &str)], variants: &[(&str, u32)]) -> ExtractedSpec {
        let mut result = ExtractedSpec::default();
        if !fields.is_empty() {
            result.structs.insert(
                "Record".into(),
                fields
                    .iter()
                    .map(|(name, type_name)| StructField {
                        name: (*name).into(),
                        type_name: (*type_name).into(),
                    })
                    .collect(),
            );
        }
        if !variants.is_empty() {
            result.enums.insert(
                "State".into(),
                variants
                    .iter()
                    .map(|(name, discriminant)| EnumVariant {
                        name: (*name).into(),
                        discriminant: *discriminant,
                    })
                    .collect(),
            );
        }
        result
    }

    #[test]
    fn identical_schema_is_backward_compatible() {
        let source = spec(&[("id", "U64")], &[("Open", 0)]);
        let target = spec(&[("id", "U64")], &[("Open", 0)]);
        assert_eq!(compare_specs(&source, &target), ChangeSummary::default());
        assert_eq!(
            classify(&ChangeSummary::default()),
            Compatibility::BackwardCompatible
        );
    }

    #[test]
    fn detects_field_enum_reorder_and_type_changes() {
        let source = spec(
            &[("id", "U64"), ("owner", "Address")],
            &[("Open", 0), ("Closed", 1)],
        );
        let target = spec(
            &[("owner", "Address"), ("id", "U128"), ("active", "Bool")],
            &[("Open", 1), ("Paused", 2)],
        );
        let changes = compare_specs(&source, &target);
        assert_eq!(
            changes.added_struct_fields["Record"],
            vec!["active".to_string()]
        );
        assert!(changes.changed_struct_fields.contains_key("Record"));
        assert_eq!(changes.reordered_struct_fields, vec!["Record".to_string()]);
        assert_eq!(
            changes.deleted_enum_variants["State"],
            vec!["Closed".to_string()]
        );
        assert_eq!(
            changes.added_enum_variants["State"],
            vec!["Paused".to_string()]
        );
        assert_eq!(classify(&changes), Compatibility::Breaking);
    }

    #[test]
    fn added_members_require_review() {
        let source = spec(&[("id", "U64")], &[("Open", 0)]);
        let target = spec(
            &[("id", "U64"), ("name", "Symbol")],
            &[("Open", 0), ("Paused", 1)],
        );
        assert_eq!(
            classify(&compare_specs(&source, &target)),
            Compatibility::CompatibleWithReview
        );
    }

    fn wasm(name: Option<&str>, data: &[u8]) -> Vec<u8> {
        let mut bytes = b"\0asm\x01\0\0\0".to_vec();
        if let Some(name) = name {
            let mut section = vec![name.len() as u8];
            section.extend_from_slice(name.as_bytes());
            section.extend_from_slice(data);
            bytes.push(0);
            bytes.push(section.len() as u8);
            bytes.extend_from_slice(&section);
        }
        bytes
    }

    #[test]
    fn invalid_missing_and_malformed_sections() {
        assert!(extract_spec_from_wasm(b"not wasm").is_err());
        assert!(extract_spec_from_wasm(&wasm(Some("other"), b"x"))
            .unwrap_err()
            .to_string()
            .contains("contractspecv0"));
        assert!(
            extract_spec_from_wasm(&wasm(Some("contractspecv0"), &[255, 255, 255]))
                .unwrap_err()
                .to_string()
                .contains("XDR")
        );
    }

    #[test]
    fn real_contractspec_xdr_extracts_struct_type() {
        use stellar_xdr::curr::{
            ScSpecEntry, ScSpecTypeDef, ScSpecUdtStructFieldV0, ScSpecUdtStructV0, StringM,
            WriteXdr,
        };
        let field = ScSpecUdtStructFieldV0 {
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
        let mut section = vec!["contractspecv0".len() as u8];
        section.extend_from_slice(b"contractspecv0");
        section.extend_from_slice(&xdr);
        let mut wasm = b"\0asm\x01\0\0\0".to_vec();
        wasm.push(0);
        wasm.push(section.len() as u8);
        wasm.extend_from_slice(&section);
        let extracted = extract_spec_from_wasm(&wasm).unwrap();
        assert_eq!(extracted.structs["Record"][0].name, "id");
        assert_eq!(extracted.structs["Record"][0].type_name, "U64");
    }

    #[test]
    fn json_is_valid_and_deterministic() {
        let changes = compare_specs(
            &spec(&[("id", "U64")], &[("Open", 0)]),
            &spec(&[("id", "U128")], &[("Open", 1)]),
        );
        let output = StorageDiffJson {
            schema_version: "soroban-forge/storage-diff@1",
            source_artifact: "a.wasm".into(),
            target_artifact: "b.wasm".into(),
            storage_keys_verified: false,
            added_storage_keys: vec![],
            deleted_storage_keys: vec![],
            struct_changes: changes,
            compatibility: Compatibility::PotentiallyBreaking,
            warnings: vec![],
            verification_limitations: vec![],
            extraction_errors: vec![],
        };
        let first = serde_json::to_string(&output).unwrap();
        let second = serde_json::to_string(&output).unwrap();
        assert_eq!(first, second);
        assert!(serde_json::from_str::<serde_json::Value>(&first).is_ok());
    }
}
