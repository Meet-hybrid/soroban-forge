//! Compare the storage schemas of two compiled Soroban contracts.
//!
//! The `contractspecv0` custom section describes the UDTs a contract uses.
//! Structs model stored value layouts and enums/unions model key enums, so a
//! change to either can invalidate data written by a previous contract version.
//! This module extracts those schemas from two WASM artifacts and reports how
//! their fields and variants changed.

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use soroban_spec::read;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use stellar_xdr_28::{ScSpecEntry, ScSpecTypeDef, ScSpecUdtUnionCaseV0};

use crate::cli::StorageDiffArgs;
use crate::commands::inspect::{infer_package_from_wasm_name, sha256_hex_file};

/// Contract specs do not expose the full internal persistent/instance storage
/// layout of a contract. This warning is always included in reports.
const LIMITATIONS: &str = "Contract specs do not necessarily expose the full internal persistent or instance storage layout. Absence of detected spec differences does NOT guarantee a safe upgrade; review persisted key encodings, XDR wire changes, migrations, and TTL behavior manually.";

/// What evidence supports a reported change.
///
/// The contract spec is not a complete description of internal storage, so the
/// report deliberately separates facts from inferred risks and items that a
/// human must review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Classification {
    /// Established directly by comparing the two `contractspecv0` entries.
    #[serde(rename = "spec-established")]
    SpecEstablished,
    /// Potential storage compatibility risk inferred from a spec change.
    #[serde(rename = "potential-risk")]
    PotentialRisk,
    /// The spec cannot determine storage safety; manual review is required.
    #[serde(rename = "manual-review-required")]
    ManualReview,
}

impl std::fmt::Display for Classification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::SpecEstablished => "spec-established",
            Self::PotentialRisk => "potential-risk",
            Self::ManualReview => "manual-review-required",
        };
        f.write_str(label)
    }
}

/// How safe a schema change is for data written by the source contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Compatibility {
    /// Existing stored values remain readable.
    BackwardCompatible,
    /// Readability depends on how the contract uses the changed schema.
    PotentiallyBreaking,
    /// Existing stored values can no longer be read correctly.
    Breaking,
}

impl std::fmt::Display for Compatibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::BackwardCompatible => "backward-compatible",
            Self::PotentiallyBreaking => "potentially-breaking",
            Self::Breaking => "breaking",
        };
        f.write_str(label)
    }
}

/// A single detected schema change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    /// Stable machine-readable change kind, e.g. `deleted-struct-field`.
    pub kind: &'static str,
    /// Name of the struct/enum the change belongs to.
    pub schema_type: String,
    /// Name of the added, removed or changed field/variant/key.
    pub name: String,
    /// Human-readable detail such as the old and new type signatures.
    pub detail: String,
    /// Evidence category for this change.
    pub classification: Classification,
    /// Severity of the change for previously stored values.
    pub compatibility: Compatibility,
}

impl Change {
    fn new(
        kind: &'static str,
        classification: Classification,
        compatibility: Compatibility,
        schema_type: impl Into<String>,
        name: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            schema_type: schema_type.into(),
            name: name.into(),
            detail: detail.into(),
            classification,
            compatibility,
        }
    }
}

/// All detected changes, grouped by the schema concept they affect.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct ChangeSummary {
    pub added_keys: Vec<Change>,
    pub deleted_keys: Vec<Change>,
    pub added_struct_fields: Vec<Change>,
    pub deleted_struct_fields: Vec<Change>,
    pub changed_struct_fields: Vec<Change>,
    pub reordered_struct_fields: Vec<Change>,
    pub added_enum_variants: Vec<Change>,
    pub deleted_enum_variants: Vec<Change>,
    pub changed_enum_variants: Vec<Change>,
    pub changed_enum_discriminants: Vec<Change>,
}

impl ChangeSummary {
    /// Returns every change in a stable category order.
    pub fn all(&self) -> Vec<&Change> {
        self.added_keys
            .iter()
            .chain(&self.deleted_keys)
            .chain(&self.added_struct_fields)
            .chain(&self.deleted_struct_fields)
            .chain(&self.changed_struct_fields)
            .chain(&self.reordered_struct_fields)
            .chain(&self.added_enum_variants)
            .chain(&self.deleted_enum_variants)
            .chain(&self.changed_enum_variants)
            .chain(&self.changed_enum_discriminants)
            .collect()
    }

    /// The most severe compatibility impact across all detected changes.
    pub fn compatibility(&self) -> Compatibility {
        self.all()
            .into_iter()
            .map(|change| change.compatibility)
            .max()
            .unwrap_or(Compatibility::BackwardCompatible)
    }

    /// Changes backed by a specific evidence category, in stable order.
    pub fn by_classification(&self, classification: Classification) -> Vec<&Change> {
        self.all()
            .into_iter()
            .filter(|change| change.classification == classification)
            .collect()
    }

    /// Changes that can invalidate values written by the source contract.
    pub fn breaking(&self) -> Vec<&Change> {
        self.all()
            .into_iter()
            .filter(|change| change.compatibility == Compatibility::Breaking)
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.added_keys.is_empty()
            && self.deleted_keys.is_empty()
            && self.added_struct_fields.is_empty()
            && self.deleted_struct_fields.is_empty()
            && self.changed_struct_fields.is_empty()
            && self.reordered_struct_fields.is_empty()
            && self.added_enum_variants.is_empty()
            && self.deleted_enum_variants.is_empty()
            && self.changed_enum_variants.is_empty()
            && self.changed_enum_discriminants.is_empty()
    }
}

/// A single field of a stored struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StructField {
    pub name: String,
    /// Full type signature, including nested types and UDT names.
    #[serde(rename = "type")]
    pub type_signature: String,
    /// Numeric XDR type discriminant, used to distinguish same-named UDTs.
    pub type_discriminant: i32,
}

/// A stored struct layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StructSchema {
    pub name: String,
    /// Fields in declaration order, which is also their XDR encoding order.
    pub fields: Vec<StructField>,
}

/// A variant of a key enum or union.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Variant {
    pub name: String,
    /// Explicit XDR discriminant for enums; unions have no numeric discriminant.
    pub discriminant: Option<i32>,
    /// Tuple payload signature for union cases.
    #[serde(rename = "type")]
    pub type_signature: Option<String>,
}

/// A key enum (`UdtEnumV0`) or tagged union (`UdtUnionV0`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnumSchema {
    pub name: String,
    /// `enum` or `union`.
    pub kind: &'static str,
    pub variants: Vec<Variant>,
}

/// The storage-relevant subset of a contract spec.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct StorageSchema {
    pub structs: BTreeMap<String, StructSchema>,
    pub enums: BTreeMap<String, EnumSchema>,
}

/// Full report emitted by the command.
#[derive(Debug, Serialize)]
struct StorageDiffReport {
    /// Stable JSON schema version for CI consumers.
    schema_version: u8,
    source: String,
    target: String,
    source_package: Option<String>,
    target_package: Option<String>,
    source_sha256: String,
    target_sha256: String,
    compatibility: Compatibility,
    change_count: usize,
    /// Counts keyed by the stable classification labels, always all present.
    classification_counts: BTreeMap<&'static str, usize>,
    changes: ChangeSummary,
    /// Explicit warnings for changes that can invalidate stored values.
    migration_warnings: Vec<String>,
    /// Always-present limitation note about spec-based storage inspection.
    limitations: &'static str,
}

impl StorageDiffReport {
    fn new(args: &StorageDiffArgs, changes: ChangeSummary) -> Result<Self> {
        let migration_warnings = changes
            .breaking()
            .into_iter()
            .map(|change| {
                format!(
                    "existing stored values for {}.{} may no longer decode ({})",
                    change.schema_type, change.name, change.kind
                )
            })
            .collect();
        let classification_counts = [
            "spec-established",
            "potential-risk",
            "manual-review-required",
        ]
        .into_iter()
        .map(|label| {
            let count = changes
                .all()
                .into_iter()
                .filter(|change| change.classification.to_string() == label)
                .count();
            (label, count)
        })
        .collect();

        Ok(Self {
            schema_version: 1,
            source: args.source.display().to_string(),
            target: args.target.display().to_string(),
            source_package: infer_package_from_wasm_name(&args.source).ok(),
            target_package: infer_package_from_wasm_name(&args.target).ok(),
            source_sha256: sha256_hex_file(&args.source)
                .with_context(|| format!("failed to hash {}", args.source.display()))?,
            target_sha256: sha256_hex_file(&args.target)
                .with_context(|| format!("failed to hash {}", args.target.display()))?,
            compatibility: changes.compatibility(),
            change_count: changes.all().len(),
            classification_counts,
            changes,
            migration_warnings,
            limitations: LIMITATIONS,
        })
    }
}

/// Runs the command. Exits nonzero when a breaking change is detected, matching
/// the `soroban-forge compare` convention; the report is always printed first.
pub fn run(args: StorageDiffArgs) -> Result<()> {
    let source_bytes = read_wasm(&args.source)?;
    let target_bytes = read_wasm(&args.target)?;

    let source_schema = extract_schema(&source_bytes).with_context(|| {
        format!(
            "failed to extract storage schema from {}",
            args.source.display()
        )
    })?;
    let target_schema = extract_schema(&target_bytes).with_context(|| {
        format!(
            "failed to extract storage schema from {}",
            args.target.display()
        )
    })?;

    let report = StorageDiffReport::new(&args, compare(&source_schema, &target_schema))?;

    let stdout = if args.json {
        serde_json::to_string_pretty(&report)?
    } else {
        render_human(&report)
    };
    println!("{stdout}");

    if args.json {
        for line in diagnostics(&report) {
            eprintln!("{line}");
        }
    }

    if !report.migration_warnings.is_empty() {
        bail!(
            "storage-diff: {} breaking storage change(s) detected; manual migration review required",
            report.migration_warnings.len()
        );
    }
    Ok(())
}

/// Reads and validates a WASM artifact path.
fn read_wasm(path: &Path) -> Result<Vec<u8>> {
    if !path.exists() {
        bail!("WASM artifact not found: {}", path.display());
    }
    if !path.is_file() {
        bail!("WASM artifact path is not a file: {}", path.display());
    }
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    if bytes.len() < 8 || !bytes.starts_with(b"\0asm") {
        bail!(
            "invalid WASM artifact: missing WASM header ({})",
            path.display()
        );
    }
    if bytes[4..8] != [1, 0, 0, 0] {
        bail!(
            "invalid WASM artifact: unsupported WASM version ({})",
            path.display()
        );
    }
    Ok(bytes)
}

/// Extracts storage-relevant schema types from a WASM artifact.
pub fn extract_schema(wasm: &[u8]) -> Result<StorageSchema> {
    let entries = read::from_wasm(wasm).map_err(|error| match error {
        read::FromWasmError::Read(error) => anyhow!("invalid WASM artifact: {error}"),
        read::FromWasmError::Parse(error) => anyhow!("malformed contract spec XDR: {error}"),
        read::FromWasmError::NotFound => anyhow!("missing contractspecv0 custom section"),
    })?;
    Ok(schema_from_entries(&entries))
}

fn schema_from_entries(entries: &[ScSpecEntry]) -> StorageSchema {
    let mut schema = StorageSchema::default();
    for entry in entries {
        match entry {
            ScSpecEntry::UdtStructV0(udt) => {
                let fields = udt
                    .fields
                    .iter()
                    .map(|field| StructField {
                        name: field.name.to_string(),
                        type_signature: type_signature(&field.type_),
                        type_discriminant: field.type_.discriminant() as i32,
                    })
                    .collect();
                schema.structs.insert(
                    udt.name.to_string(),
                    StructSchema {
                        name: udt.name.to_string(),
                        fields,
                    },
                );
            }
            ScSpecEntry::UdtEnumV0(udt) => {
                let variants = udt
                    .cases
                    .iter()
                    .map(|case| Variant {
                        name: case.name.to_string(),
                        discriminant: Some(case.value as i32),
                        type_signature: None,
                    })
                    .collect();
                schema.enums.insert(
                    udt.name.to_string(),
                    EnumSchema {
                        name: udt.name.to_string(),
                        kind: "enum",
                        variants,
                    },
                );
            }
            ScSpecEntry::UdtUnionV0(udt) => {
                let variants = udt
                    .cases
                    .iter()
                    .map(|case| match case {
                        ScSpecUdtUnionCaseV0::VoidV0(case) => Variant {
                            name: case.name.to_string(),
                            discriminant: None,
                            type_signature: None,
                        },
                        ScSpecUdtUnionCaseV0::TupleV0(case) => Variant {
                            name: case.name.to_string(),
                            discriminant: None,
                            type_signature: Some(
                                case.type_
                                    .iter()
                                    .map(type_signature)
                                    .collect::<Vec<_>>()
                                    .join(","),
                            ),
                        },
                    })
                    .collect();
                schema.enums.insert(
                    udt.name.to_string(),
                    EnumSchema {
                        name: udt.name.to_string(),
                        kind: "union",
                        variants,
                    },
                );
            }
            ScSpecEntry::FunctionV0(_)
            | ScSpecEntry::UdtErrorEnumV0(_)
            | ScSpecEntry::EventV0(_) => {}
        }
    }
    schema
}

fn type_signature(ty: &ScSpecTypeDef) -> String {
    match ty {
        ScSpecTypeDef::Option(value) => format!("Option<{}>", type_signature(&value.value_type)),
        ScSpecTypeDef::Result(value) => format!(
            "Result<{},{}>",
            type_signature(&value.ok_type),
            type_signature(&value.error_type)
        ),
        ScSpecTypeDef::Vec(value) => format!("Vec<{}>", type_signature(&value.element_type)),
        ScSpecTypeDef::Map(value) => format!(
            "Map<{},{}>",
            type_signature(&value.key_type),
            type_signature(&value.value_type)
        ),
        ScSpecTypeDef::Tuple(value) => format!(
            "Tuple<{}>",
            value
                .value_types
                .iter()
                .map(type_signature)
                .collect::<Vec<_>>()
                .join(",")
        ),
        ScSpecTypeDef::BytesN(value) => format!("BytesN<{}>", value.n),
        ScSpecTypeDef::Udt(value) => value.name.to_string(),
        _ => ty.name().to_owned(),
    }
}

/// Compares two storage schemas and groups the resulting changes.
pub fn compare(source: &StorageSchema, target: &StorageSchema) -> ChangeSummary {
    let mut changes = ChangeSummary::default();
    compare_keys(source, target, &mut changes);
    compare_structs(source, target, &mut changes);
    compare_enums(source, target, &mut changes);
    changes
}

fn compare_keys(source: &StorageSchema, target: &StorageSchema, changes: &mut ChangeSummary) {
    let source_keys: BTreeSet<&str> = source
        .structs
        .keys()
        .chain(source.enums.keys())
        .map(String::as_str)
        .collect();
    let target_keys: BTreeSet<&str> = target
        .structs
        .keys()
        .chain(target.enums.keys())
        .map(String::as_str)
        .collect();

    for key in source_keys.difference(&target_keys) {
        let kind = if source.structs.contains_key(*key) {
            "struct"
        } else {
            "enum"
        };
        changes.deleted_keys.push(Change::new(
            "deleted-key",
            Classification::SpecEstablished,
            Compatibility::Breaking,
            kind,
            *key,
            format!("{kind} schema removed"),
        ));
    }
    for key in target_keys.difference(&source_keys) {
        let kind = if target.structs.contains_key(*key) {
            "struct"
        } else {
            "enum"
        };
        changes.added_keys.push(Change::new(
            "added-key",
            Classification::ManualReview,
            Compatibility::BackwardCompatible,
            kind,
            *key,
            format!("{kind} schema added"),
        ));
    }
}

fn compare_structs(source: &StorageSchema, target: &StorageSchema, changes: &mut ChangeSummary) {
    for (name, source_struct) in &source.structs {
        let Some(target_struct) = target.structs.get(name) else {
            continue;
        };
        let source_fields: BTreeMap<&str, &StructField> = source_struct
            .fields
            .iter()
            .map(|field| (field.name.as_str(), field))
            .collect();
        let target_fields: BTreeMap<&str, &StructField> = target_struct
            .fields
            .iter()
            .map(|field| (field.name.as_str(), field))
            .collect();

        for (field_name, source_field) in &source_fields {
            match target_fields.get(field_name) {
                None => changes.deleted_struct_fields.push(Change::new(
                    "deleted-struct-field",
                    Classification::SpecEstablished,
                    Compatibility::Breaking,
                    name,
                    *field_name,
                    source_field.type_signature.clone(),
                )),
                Some(target_field) if **source_field != **target_field => {
                    changes.changed_struct_fields.push(Change::new(
                        "changed-struct-field",
                        Classification::PotentialRisk,
                        Compatibility::PotentiallyBreaking,
                        name,
                        *field_name,
                        format!(
                            "{} ({}) -> {} ({})",
                            source_field.type_signature,
                            source_field.type_discriminant,
                            target_field.type_signature,
                            target_field.type_discriminant
                        ),
                    ));
                }
                Some(_) => {}
            }
        }
        for field_name in target_fields.keys() {
            if !source_fields.contains_key(field_name) {
                let signature = &target_fields[field_name].type_signature;
                changes.added_struct_fields.push(Change::new(
                    "added-struct-field",
                    Classification::ManualReview,
                    Compatibility::Breaking,
                    name,
                    *field_name,
                    signature.clone(),
                ));
            }
        }

        // Only compare the relative order of fields present in both schemas, so
        // adding or removing a field does not also produce a reorder change.
        let shared_source: Vec<&str> = source_struct
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .filter(|field_name| target_fields.contains_key(field_name))
            .collect();
        let shared_target: Vec<&str> = target_struct
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .filter(|field_name| source_fields.contains_key(field_name))
            .collect();
        if shared_source != shared_target {
            changes.reordered_struct_fields.push(Change::new(
                "reordered-struct-fields",
                Classification::PotentialRisk,
                Compatibility::PotentiallyBreaking,
                name,
                "fields",
                format!("{} -> {}", shared_source.join(","), shared_target.join(",")),
            ));
        }
    }
}

fn compare_enums(source: &StorageSchema, target: &StorageSchema, changes: &mut ChangeSummary) {
    for (name, source_enum) in &source.enums {
        let Some(target_enum) = target.enums.get(name) else {
            continue;
        };
        let source_variants: BTreeMap<&str, &Variant> = source_enum
            .variants
            .iter()
            .map(|variant| (variant.name.as_str(), variant))
            .collect();
        let target_variants: BTreeMap<&str, &Variant> = target_enum
            .variants
            .iter()
            .map(|variant| (variant.name.as_str(), variant))
            .collect();

        for (variant_name, source_variant) in &source_variants {
            match target_variants.get(variant_name) {
                None => changes.deleted_enum_variants.push(Change::new(
                    "deleted-enum-variant",
                    Classification::SpecEstablished,
                    Compatibility::Breaking,
                    name,
                    *variant_name,
                    format_discriminant(source_variant.discriminant),
                )),
                Some(target_variant) => {
                    if source_variant.discriminant != target_variant.discriminant {
                        changes.changed_enum_discriminants.push(Change::new(
                            "changed-enum-discriminant",
                            Classification::SpecEstablished,
                            Compatibility::PotentiallyBreaking,
                            name,
                            *variant_name,
                            format!(
                                "{} -> {}",
                                format_discriminant(source_variant.discriminant),
                                format_discriminant(target_variant.discriminant)
                            ),
                        ));
                    }
                    if source_variant.type_signature != target_variant.type_signature {
                        changes.changed_enum_variants.push(Change::new(
                            "changed-enum-variant",
                            Classification::PotentialRisk,
                            Compatibility::PotentiallyBreaking,
                            name,
                            *variant_name,
                            format!(
                                "{} -> {}",
                                source_variant.type_signature.as_deref().unwrap_or("void"),
                                target_variant.type_signature.as_deref().unwrap_or("void")
                            ),
                        ));
                    }
                }
            }
        }
        for variant_name in target_variants.keys() {
            if !source_variants.contains_key(variant_name) {
                let variant = &target_variants[variant_name];
                changes.added_enum_variants.push(Change::new(
                    "added-enum-variant",
                    Classification::ManualReview,
                    Compatibility::PotentiallyBreaking,
                    name,
                    *variant_name,
                    format_discriminant(variant.discriminant),
                ));
            }
        }
    }
}

fn format_discriminant(value: Option<i32>) -> String {
    value
        .map(|value| format!("discriminant={value}"))
        .unwrap_or_else(|| "no discriminant".to_owned())
}

fn render_human(report: &StorageDiffReport) -> String {
    let mut lines = vec![
        "Storage schema diff".to_owned(),
        format!("  source: {}", report.source),
        format!("  target: {}", report.target),
        format!(
            "  packages: {} -> {}",
            report.source_package.as_deref().unwrap_or("unknown"),
            report.target_package.as_deref().unwrap_or("unknown")
        ),
        format!(
            "  sha256: {} -> {}",
            truncate_hash(&report.source_sha256),
            truncate_hash(&report.target_sha256)
        ),
        format!("  compatibility: {}", report.compatibility),
        format!("  changes: {}", report.change_count),
        format!(
            "  classifications: spec-established={} potential-risk={} manual-review-required={}",
            report.classification_counts["spec-established"],
            report.classification_counts["potential-risk"],
            report.classification_counts["manual-review-required"]
        ),
    ];

    if report.changes.is_empty() {
        lines.push("  no schema changes detected".to_owned());
    } else {
        for (title, classification) in [
            ("Spec-established changes", Classification::SpecEstablished),
            (
                "Potential storage compatibility risks",
                Classification::PotentialRisk,
            ),
            (
                "Changes requiring manual migration review",
                Classification::ManualReview,
            ),
        ] {
            let changes = report.changes.by_classification(classification);
            if changes.is_empty() {
                continue;
            }
            lines.push(String::new());
            lines.push(format!("{title} ({}):", changes.len()));
            for change in changes {
                lines.push(format!(
                    "  - [{}] {}.{}: {} ({})",
                    change.compatibility,
                    change.schema_type,
                    change.name,
                    change.detail,
                    change.kind
                ));
            }
        }
    }

    if report.migration_warnings.is_empty() {
        lines.push(String::new());
        lines.push("No breaking storage changes detected by spec comparison.".to_owned());
    } else {
        lines.push(String::new());
        lines.push("MIGRATION WARNINGS:".to_owned());
        for warning in &report.migration_warnings {
            lines.push(format!("  MIGRATION WARNING: {warning}"));
        }
    }
    lines.push(String::new());
    lines.push(format!("LIMITATIONS: {}", report.limitations));
    lines.join("\n")
}

fn truncate_hash(hash: &str) -> &str {
    &hash[..hash.len().min(12)]
}

/// Diagnostic lines emitted on stderr when JSON output is requested.
fn diagnostics(report: &StorageDiffReport) -> Vec<String> {
    let mut lines = vec![format!(
        "storage-diff: compatibility={} changes={}",
        report.compatibility, report.change_count
    )];
    for warning in &report.migration_warnings {
        lines.push(format!("storage-diff: MIGRATION WARNING: {warning}"));
    }
    lines.push(format!("storage-diff: LIMITATIONS: {}", report.limitations));
    for change in report.changes.all() {
        lines.push(format!(
            "storage-diff: {} [{}] {} {}.{}: {}",
            change.classification,
            change.compatibility,
            change.kind,
            change.schema_type,
            change.name,
            change.detail
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_xdr_28::{
        Limited, Limits, ScSpecUdtEnumCaseV0, ScSpecUdtEnumV0, ScSpecUdtStructFieldV0,
        ScSpecUdtStructV0, StringM, WriteXdr,
    };

    fn field(name: &str, ty: ScSpecTypeDef) -> ScSpecUdtStructFieldV0 {
        ScSpecUdtStructFieldV0 {
            doc: StringM::default(),
            name: name.try_into().unwrap(),
            type_: ty,
        }
    }

    fn struct_type(name: &str, fields: Vec<ScSpecUdtStructFieldV0>) -> ScSpecEntry {
        ScSpecEntry::UdtStructV0(ScSpecUdtStructV0 {
            doc: StringM::default(),
            lib: StringM::default(),
            name: name.try_into().unwrap(),
            fields: fields.try_into().unwrap(),
        })
    }

    fn variant(name: &str, value: u32) -> ScSpecUdtEnumCaseV0 {
        ScSpecUdtEnumCaseV0 {
            doc: StringM::default(),
            name: name.try_into().unwrap(),
            value,
        }
    }

    fn enum_type(name: &str, variants: Vec<ScSpecUdtEnumCaseV0>) -> ScSpecEntry {
        ScSpecEntry::UdtEnumV0(ScSpecUdtEnumV0 {
            doc: StringM::default(),
            lib: StringM::default(),
            name: name.try_into().unwrap(),
            cases: variants.try_into().unwrap(),
        })
    }

    fn push_uleb128(bytes: &mut Vec<u8>, mut value: usize) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            bytes.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    fn wasm_with_raw_spec(spec: &[u8]) -> Vec<u8> {
        let mut wasm = b"\0asm\x01\0\0\0".to_vec();
        let name = b"contractspecv0";
        let mut payload = vec![name.len() as u8];
        payload.extend_from_slice(name);
        payload.extend_from_slice(spec);
        wasm.push(0x00); // custom section id
        push_uleb128(&mut wasm, payload.len());
        wasm.extend_from_slice(&payload);
        wasm
    }

    fn wasm_with_entries(entries: &[ScSpecEntry], include_spec: bool) -> Vec<u8> {
        if !include_spec {
            return b"\0asm\x01\0\0\0".to_vec();
        }
        let mut spec = Vec::new();
        let mut limited = Limited::new(&mut spec, Limits::none());
        for entry in entries {
            entry.write_xdr(&mut limited).unwrap();
        }
        wasm_with_raw_spec(&spec)
    }

    fn source_schema() -> StorageSchema {
        schema_from_entries(&[
            struct_type(
                "Position",
                vec![
                    field("x", ScSpecTypeDef::U32),
                    field("y", ScSpecTypeDef::U32),
                ],
            ),
            enum_type("DataKey", vec![variant("Admin", 0), variant("Balance", 1)]),
        ])
    }

    #[test]
    fn extraction_reads_struct_fields_and_enum_variants() {
        let wasm = wasm_with_entries(
            &[
                struct_type("Position", vec![field("x", ScSpecTypeDef::U32)]),
                enum_type("DataKey", vec![variant("Admin", 7)]),
            ],
            true,
        );
        let schema = extract_schema(&wasm).unwrap();
        let position = &schema.structs["Position"];
        assert_eq!(position.fields.len(), 1);
        assert_eq!(position.fields[0].name, "x");
        assert_eq!(position.fields[0].type_signature, "U32");
        assert_eq!(
            position.fields[0].type_discriminant,
            ScSpecTypeDef::U32.discriminant() as i32
        );
        let key = &schema.enums["DataKey"];
        assert_eq!(key.kind, "enum");
        assert_eq!(key.variants[0].name, "Admin");
        assert_eq!(key.variants[0].discriminant, Some(7));
    }

    #[test]
    fn extraction_reports_missing_malformed_and_invalid_sections() {
        let no_section = extract_schema(&wasm_with_entries(&[], false)).unwrap_err();
        assert!(no_section.to_string().contains("contractspecv0"));

        // A custom section with an invalid XDR discriminant.
        let malformed = extract_schema(&wasm_with_raw_spec(&[0xff; 8])).unwrap_err();
        assert!(malformed.to_string().contains("XDR"));

        let invalid = extract_schema(b"not a wasm file").unwrap_err();
        assert!(invalid.to_string().contains("invalid WASM"));
    }

    #[test]
    fn read_wasm_distinguishes_missing_non_file_and_bad_header() {
        let dir = tempfile::tempdir().unwrap();
        let missing = read_wasm(&dir.path().join("missing.wasm")).unwrap_err();
        assert!(missing.to_string().contains("not found"));

        let bad = dir.path().join("bad.wasm");
        fs::write(&bad, b"nope").unwrap();
        assert!(read_wasm(&bad)
            .unwrap_err()
            .to_string()
            .contains("invalid WASM"));
    }

    #[test]
    fn identical_schemas_are_backward_compatible() {
        let schema = source_schema();
        let changes = compare(&schema, &schema);
        assert!(changes.is_empty());
        assert_eq!(changes.compatibility(), Compatibility::BackwardCompatible);
    }

    #[test]
    fn added_and_deleted_keys_are_classified() {
        let source = source_schema();
        let mut target = source.clone();
        target.structs.remove("Position");
        target.enums.insert(
            "Extra".to_owned(),
            EnumSchema {
                name: "Extra".to_owned(),
                kind: "enum",
                variants: Vec::new(),
            },
        );

        let changes = compare(&source, &target);
        assert_eq!(changes.deleted_keys.len(), 1);
        assert_eq!(changes.deleted_keys[0].name, "Position");
        assert_eq!(
            changes.deleted_keys[0].compatibility,
            Compatibility::Breaking
        );
        assert_eq!(changes.added_keys.len(), 1);
        assert_eq!(changes.added_keys[0].name, "Extra");
        assert_eq!(
            changes.added_keys[0].compatibility,
            Compatibility::BackwardCompatible
        );
        assert_eq!(changes.compatibility(), Compatibility::Breaking);
    }

    #[test]
    fn struct_field_additions_deletions_and_type_changes_are_classified() {
        let source = source_schema();
        let mut target = source.clone();
        {
            let position = target.structs.get_mut("Position").unwrap();
            position.fields[0].type_signature = "U64".to_owned();
            position.fields[0].type_discriminant = ScSpecTypeDef::U64.discriminant() as i32;
            position.fields.pop();
            position.fields.push(StructField {
                name: "z".to_owned(),
                type_signature: "Bool".to_owned(),
                type_discriminant: ScSpecTypeDef::Bool.discriminant() as i32,
            });
        }

        let changes = compare(&source, &target);
        assert_eq!(changes.changed_struct_fields.len(), 1);
        assert_eq!(changes.changed_struct_fields[0].name, "x");
        assert_eq!(
            changes.changed_struct_fields[0].compatibility,
            Compatibility::PotentiallyBreaking
        );
        assert_eq!(changes.deleted_struct_fields.len(), 1);
        assert_eq!(changes.deleted_struct_fields[0].name, "y");
        assert_eq!(changes.added_struct_fields.len(), 1);
        assert_eq!(changes.added_struct_fields[0].name, "z");
        assert_eq!(changes.compatibility(), Compatibility::Breaking);
    }

    #[test]
    fn reordered_struct_fields_are_detected_without_add_delete_noise() {
        let source = source_schema();
        let mut target = source.clone();
        target.structs.get_mut("Position").unwrap().fields.reverse();

        let changes = compare(&source, &target);
        assert_eq!(changes.reordered_struct_fields.len(), 1);
        assert_eq!(changes.reordered_struct_fields[0].schema_type, "Position");
        assert_eq!(
            changes.reordered_struct_fields[0].compatibility,
            Compatibility::PotentiallyBreaking
        );
        assert!(changes.added_struct_fields.is_empty());
        assert!(changes.deleted_struct_fields.is_empty());
    }

    #[test]
    fn enum_variant_and_discriminant_changes_are_classified() {
        let source = source_schema();
        let mut target = source.clone();
        target.enums.get_mut("DataKey").unwrap().variants = vec![
            Variant {
                name: "Admin".to_owned(),
                discriminant: Some(9),
                type_signature: None,
            },
            Variant {
                name: "NewKey".to_owned(),
                discriminant: Some(2),
                type_signature: None,
            },
        ];

        let changes = compare(&source, &target);
        assert_eq!(changes.deleted_enum_variants.len(), 1);
        assert_eq!(changes.deleted_enum_variants[0].name, "Balance");
        assert_eq!(changes.added_enum_variants.len(), 1);
        assert_eq!(changes.added_enum_variants[0].name, "NewKey");
        assert_eq!(
            changes.added_enum_variants[0].compatibility,
            Compatibility::PotentiallyBreaking
        );
        assert_eq!(changes.changed_enum_discriminants.len(), 1);
        assert_eq!(changes.changed_enum_discriminants[0].name, "Admin");
        assert_eq!(changes.compatibility(), Compatibility::Breaking);
    }

    #[test]
    fn union_payload_changes_are_reported_as_variant_changes() {
        let source =
            schema_from_entries(&[ScSpecEntry::UdtUnionV0(stellar_xdr_28::ScSpecUdtUnionV0 {
                doc: StringM::default(),
                lib: StringM::default(),
                name: "Key".try_into().unwrap(),
                cases: vec![ScSpecUdtUnionCaseV0::TupleV0(
                    stellar_xdr_28::ScSpecUdtUnionCaseTupleV0 {
                        doc: StringM::default(),
                        name: "Balance".try_into().unwrap(),
                        type_: vec![ScSpecTypeDef::Address].try_into().unwrap(),
                    },
                )]
                .try_into()
                .unwrap(),
            })]);
        let mut target = source.clone();
        target.enums.get_mut("Key").unwrap().variants[0].type_signature = Some("U32".to_owned());

        let changes = compare(&source, &target);
        assert_eq!(changes.changed_enum_variants.len(), 1);
        assert!(changes.changed_enum_variants[0]
            .detail
            .contains("Address -> U32"));
    }

    fn report_for(changes: ChangeSummary) -> StorageDiffReport {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.wasm");
        let target = dir.path().join("target.wasm");
        fs::write(&source, b"source-artifact").unwrap();
        fs::write(&target, b"target-artifact").unwrap();
        let args = StorageDiffArgs {
            source,
            target,
            json: false,
        };
        StorageDiffReport::new(&args, changes).unwrap()
    }

    #[test]
    fn classifications_distinguish_facts_risks_and_manual_review() {
        let source = source_schema();
        let mut target = source.clone();
        target.structs.get_mut("Position").unwrap().fields[0].type_signature = "U64".to_owned();
        target
            .enums
            .get_mut("DataKey")
            .unwrap()
            .variants
            .push(Variant {
                name: "Extra".to_owned(),
                discriminant: Some(2),
                type_signature: None,
            });
        let changes = compare(&source, &target);

        assert_eq!(
            changes
                .by_classification(Classification::SpecEstablished)
                .len(),
            0
        );
        assert!(changes
            .by_classification(Classification::PotentialRisk)
            .iter()
            .any(|change| change.kind == "changed-struct-field"));
        assert!(changes
            .by_classification(Classification::ManualReview)
            .iter()
            .any(|change| change.kind == "added-enum-variant"));
        assert_eq!(changes.compatibility(), Compatibility::PotentiallyBreaking);
    }

    #[test]
    fn json_report_and_diagnostics_are_structured_and_deterministic() {
        let source = source_schema();
        let mut target = source.clone();
        target.structs.get_mut("Position").unwrap().fields.remove(1);
        let report = report_for(compare(&source, &target));

        let first = serde_json::to_string_pretty(&report).unwrap();
        let second = serde_json::to_string_pretty(&report).unwrap();
        assert_eq!(
            first, second,
            "JSON output must be byte-for-byte deterministic"
        );

        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["schema_version"], 1);
        assert!(json["source"].as_str().unwrap().ends_with("source.wasm"));
        assert_eq!(json["compatibility"], "breaking");
        assert_eq!(json["change_count"], 1);
        assert_eq!(json["classification_counts"]["spec-established"], 1);
        assert_eq!(json["changes"]["deleted_struct_fields"][0]["name"], "y");
        assert_eq!(
            json["changes"]["deleted_struct_fields"][0]["classification"],
            "spec-established"
        );
        assert_eq!(
            json["changes"]["deleted_struct_fields"][0]["compatibility"],
            "breaking"
        );
        assert_eq!(json["migration_warnings"].as_array().unwrap().len(), 1);
        assert!(json["limitations"]
            .as_str()
            .unwrap()
            .contains("does NOT guarantee"));

        let diagnostics = diagnostics(&report);
        assert!(diagnostics[0].contains("compatibility=breaking"));
        assert!(diagnostics
            .iter()
            .any(|line| line.contains("MIGRATION WARNING")));
        assert!(diagnostics.iter().any(|line| line.contains("LIMITATIONS")));
        assert!(diagnostics
            .iter()
            .any(|line| line.contains("deleted-struct-field")));

        let human = render_human(&report);
        assert!(human.contains("Storage schema diff"));
        assert!(human.contains("Spec-established changes"));
        assert!(human.contains("MIGRATION WARNING"));
        assert!(human.contains("Position.y"));
    }

    #[test]
    fn identical_schemas_have_no_warnings_but_still_state_limitations() {
        let schema = source_schema();
        let report = report_for(compare(&schema, &schema));
        assert!(report.migration_warnings.is_empty());
        assert!(render_human(&report).contains("does NOT guarantee a safe upgrade"));
    }

    #[test]
    fn run_rejects_breaking_storage_changes_after_reporting() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.wasm");
        let target = dir.path().join("target.wasm");
        let source_entries = [
            struct_type(
                "Position",
                vec![
                    field("x", ScSpecTypeDef::U32),
                    field("y", ScSpecTypeDef::U32),
                ],
            ),
            enum_type("DataKey", vec![variant("Admin", 0), variant("Balance", 1)]),
        ];
        let target_entries = [struct_type(
            "Position",
            vec![field("x", ScSpecTypeDef::U32)],
        )];
        fs::write(&source, wasm_with_entries(&source_entries, true)).unwrap();
        fs::write(&target, wasm_with_entries(&target_entries, true)).unwrap();

        let error = run(StorageDiffArgs {
            source,
            target,
            json: true,
        })
        .unwrap_err();
        assert!(
            error.to_string().contains("breaking storage change"),
            "unexpected error: {error}"
        );
    }
}
