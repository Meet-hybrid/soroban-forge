use crate::cli::ValidatePlanArgs;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use stellar_strkey::{ed25519, Contract, Strkey};
use stellar_xdr::curr::{
    AccountId, Hash, Int128Parts, PublicKey, ScAddress, ScBytes, ScString, ScSymbol, ScVal,
    StringM, UInt128Parts, Uint256,
};

pub const PLAN_SCHEMA_V1: &str = "soroban-forge/transaction-plan@1";
pub const REPORT_SCHEMA_V1: &str = "soroban-forge/tx-plan-validation-report@1";

pub const EXIT_OK: i32 = 0;
pub const EXIT_INVALID_PLAN: i32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationCategory {
    MissingRequiredField,
    NetworkMismatch,
    InvalidContractId,
    UnsupportedMethod,
    MalformedArgument,
    SensitiveInputRejected,
    SchemaMismatch,
}

impl ValidationCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingRequiredField => "missing_required_field",
            Self::NetworkMismatch => "network_mismatch",
            Self::InvalidContractId => "invalid_contract_id",
            Self::UnsupportedMethod => "unsupported_method",
            Self::MalformedArgument => "malformed_argument",
            Self::SensitiveInputRejected => "sensitive_input_rejected",
            Self::SchemaMismatch => "schema_mismatch",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub category: ValidationCategory,
    pub field: String,
    pub message: String,
    pub hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncodedArgumentSummary {
    pub index: usize,
    pub name: Option<String>,
    pub kind: String,
    pub scval_variant: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub schema: String,
    pub valid: bool,
    pub exit_code: i32,
    pub network: Option<String>,
    pub contract: Option<String>,
    pub method: Option<String>,
    pub encoded_args: Vec<EncodedArgumentSummary>,
    pub error_count: usize,
    pub errors: Vec<ValidationIssue>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TransactionPlanInput {
    #[serde(default)]
    pub schema: Option<String>,
    #[serde(default)]
    pub network: Option<String>,
    #[serde(default)]
    pub network_passphrase: Option<String>,
    #[serde(default, alias = "contract_id")]
    pub contract: Option<String>,
    #[serde(default)]
    pub contract_type: Option<String>,
    #[serde(default, alias = "function")]
    pub method: Option<String>,
    #[serde(default, alias = "source_account")]
    pub source: Option<String>,
    #[serde(default)]
    pub secret: Option<String>,
    #[serde(default, alias = "secret_key")]
    pub private_key: Option<String>,
    #[serde(default)]
    pub args: Vec<RawPlanArg>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RawPlanArg {
    Spec(String),
    Structured(StructuredPlanArg),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StructuredPlanArg {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, alias = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
}

type ArgumentNames = &'static [(&'static str, &'static str)];

struct KnownMethodSpec {
    name: &'static str,
    arg_sets: &'static [ArgumentNames],
}

struct ContractValidation {
    display: Option<String>,
    alias: Option<&'static str>,
}

struct MethodValidation {
    display: Option<String>,
    spec: Option<&'static KnownMethodSpec>,
}

struct ArgumentValidation {
    encoded: Vec<EncodedArgumentSummary>,
    seen_names: BTreeSet<String>,
    provided_kinds: BTreeMap<String, String>,
    uses_named_args: bool,
}

struct NormalizedArg {
    name: Option<String>,
    kind: String,
    raw_value: String,
}

pub fn run(args: ValidatePlanArgs) -> Result<()> {
    let plan = load_plan_from_args(&args)?;
    let report = validate_plan(&plan, args.expected_network.as_deref());

    if args.json {
        println!("{}", render_json(&report)?);
    } else {
        print!("{}", render_text(&report));
    }

    if !report.valid {
        std::process::exit(report.exit_code);
    }

    Ok(())
}

pub fn load_plan_from_args(args: &ValidatePlanArgs) -> Result<TransactionPlanInput> {
    let path = args.plan.as_ref().or(args.file.as_ref());
    let mut plan = match path {
        Some(path) => load_plan_file(path)?,
        None => TransactionPlanInput::default(),
    };

    if let Some(network) = &args.network {
        plan.network = Some(network.clone());
    }
    if let Some(contract) = &args.contract {
        plan.contract = Some(contract.clone());
    }
    if let Some(method) = &args.method {
        plan.method = Some(method.clone());
    }
    if let Some(source) = &args.source {
        plan.source = Some(source.clone());
    }
    if !args.args.is_empty() {
        plan.args
            .extend(args.args.iter().cloned().map(RawPlanArg::Spec));
    }

    Ok(plan)
}

pub fn load_plan_file(path: &Path) -> Result<TransactionPlanInput> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read transaction plan at {}", path.display()))?;
    parse_plan_str(&raw, path.extension().and_then(|ext| ext.to_str()))
}

pub fn parse_plan_str(raw: &str, extension_hint: Option<&str>) -> Result<TransactionPlanInput> {
    let trimmed = raw.trim();
    let try_toml = matches!(extension_hint, Some("toml"))
        || (!trimmed.is_empty() && !trimmed.starts_with('{') && extension_hint != Some("json"));

    if try_toml {
        if let Ok(plan) = toml::from_str::<TransactionPlanInput>(raw) {
            return Ok(plan);
        }
    }

    serde_json::from_str::<TransactionPlanInput>(raw)
        .context("failed to parse transaction plan as JSON or TOML")
}

pub fn validate_plan(
    plan: &TransactionPlanInput,
    expected_network: Option<&str>,
) -> ValidationReport {
    let mut errors = Vec::new();

    validate_schema(plan, &mut errors);
    validate_secrets(plan, &mut errors);
    validate_source(plan, &mut errors);

    let network = validate_network(plan, expected_network, &mut errors);
    let contract = validate_contract(plan, &mut errors);
    let method = validate_method(plan, contract.alias, &mut errors);
    let arguments = validate_arguments(plan, &mut errors);

    if let Some(spec) = method.spec {
        validate_method_inputs(spec, &arguments, &mut errors);
    }

    let valid = errors.is_empty();
    ValidationReport {
        schema: REPORT_SCHEMA_V1.to_owned(),
        valid,
        exit_code: if valid { EXIT_OK } else { EXIT_INVALID_PLAN },
        network,
        contract: contract.display,
        method: method.display,
        encoded_args: arguments.encoded,
        error_count: errors.len(),
        errors,
    }
}

fn push_error(
    errors: &mut Vec<ValidationIssue>,
    category: ValidationCategory,
    field: &str,
    message: String,
    hint: String,
) {
    errors.push(ValidationIssue {
        category,
        field: field.to_owned(),
        message,
        hint,
    });
}

fn validate_schema(plan: &TransactionPlanInput, errors: &mut Vec<ValidationIssue>) {
    let Some(schema) = plan
        .schema
        .as_deref()
        .map(str::trim)
        .filter(|schema| !schema.is_empty())
    else {
        return;
    };

    if schema != PLAN_SCHEMA_V1 {
        push_error(
            errors,
            ValidationCategory::SchemaMismatch,
            "schema",
            format!(
                "unsupported transaction-plan schema `{}`",
                sanitize_display(schema)
            ),
            format!("set `schema` to `{PLAN_SCHEMA_V1}` or omit the field"),
        );
    }
}

fn validate_secrets(plan: &TransactionPlanInput, errors: &mut Vec<ValidationIssue>) {
    for (field, value, hint) in [
        (
            "secret",
            &plan.secret,
            "remove `secret` from the plan and pass signing identities at submission time",
        ),
        (
            "private_key",
            &plan.private_key,
            "remove `private_key` from the plan and use external key management for signing",
        ),
    ] {
        if value
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            push_error(
                errors,
                ValidationCategory::SensitiveInputRejected,
                field,
                format!("transaction plans must not embed `{field}` keys ([REDACTED])"),
                hint.to_owned(),
            );
        }
    }
}

fn validate_source(plan: &TransactionPlanInput, errors: &mut Vec<ValidationIssue>) {
    let Some(source) = plan
        .source
        .as_deref()
        .map(str::trim)
        .filter(|source| !source.is_empty())
    else {
        return;
    };

    if looks_like_secret_key(source) {
        push_error(
            errors,
            ValidationCategory::SensitiveInputRejected,
            "source",
            "source field contains a secret seed ([REDACTED]) instead of a public key".to_owned(),
            "provide a public `G...` account address in `source`, never an `S...` seed".to_owned(),
        );
    } else if ed25519::PublicKey::from_string(source).is_err() {
        push_error(
            errors,
            ValidationCategory::MalformedArgument,
            "source",
            format!(
                "invalid Stellar source public key `{}`",
                sanitize_display(source)
            ),
            "provide a valid 56-character `G...` Stellar account strkey".to_owned(),
        );
    }
}

fn validate_network(
    plan: &TransactionPlanInput,
    expected_network: Option<&str>,
    errors: &mut Vec<ValidationIssue>,
) -> Option<String> {
    let Some(network) = plan
        .network
        .as_deref()
        .map(str::trim)
        .filter(|network| !network.is_empty())
        .map(str::to_lowercase)
    else {
        push_error(
            errors,
            ValidationCategory::MissingRequiredField,
            "network",
            "required field `network` is missing or empty".to_owned(),
            "specify `network` as `testnet`, `futurenet`, `standalone`, or `mainnet`".to_owned(),
        );
        return None;
    };

    let Some(canonical_passphrase) = expected_passphrase_for_network(&network) else {
        push_error(
            errors,
            ValidationCategory::NetworkMismatch,
            "network",
            format!("unsupported Stellar network `{network}`"),
            "use one of: `testnet`, `futurenet`, `standalone`, `mainnet`".to_owned(),
        );
        return Some(sanitize_display(&network));
    };

    if let Some(expected) = expected_network
        .map(str::trim)
        .filter(|expected| !expected.is_empty())
        .map(str::to_lowercase)
    {
        if canonical_network_name(&network) != canonical_network_name(&expected) {
            push_error(
                errors,
                ValidationCategory::NetworkMismatch,
                "network",
                format!(
                    "plan targets network `{network}`, which does not match expected network `{expected}`"
                ),
                format!(
                    "update the plan `network` to `{expected}` or run against `{network}`"
                ),
            );
        }
    }

    if let Some(passphrase) = plan
        .network_passphrase
        .as_deref()
        .map(str::trim)
        .filter(|passphrase| !passphrase.is_empty())
    {
        if passphrase != canonical_passphrase {
            push_error(
                errors,
                ValidationCategory::NetworkMismatch,
                "network_passphrase",
                format!("network passphrase does not match network `{network}`"),
                format!(
                    "use `{canonical_passphrase}` for `{network}` or omit `network_passphrase`"
                ),
            );
        }
    }

    Some(sanitize_display(&network))
}

fn validate_contract(
    plan: &TransactionPlanInput,
    errors: &mut Vec<ValidationIssue>,
) -> ContractValidation {
    let raw = plan
        .contract
        .as_deref()
        .map(str::trim)
        .filter(|contract| !contract.is_empty());

    let mut alias = plan
        .contract_type
        .as_deref()
        .and_then(resolve_known_contract_alias);

    let display = match raw {
        None => {
            push_error(
                errors,
                ValidationCategory::MissingRequiredField,
                "contract",
                "required field `contract` is missing or empty".to_owned(),
                "provide a 56-character `C...` contract strkey or a known Soroban Forge contract alias".to_owned(),
            );
            None
        }
        Some(contract) if looks_like_secret_key(contract) => {
            push_error(
                errors,
                ValidationCategory::SensitiveInputRejected,
                "contract",
                "contract field contains a secret seed ([REDACTED])".to_owned(),
                "provide a public `C...` contract address, never a secret seed".to_owned(),
            );
            Some("[REDACTED]".to_owned())
        }
        Some(contract) => {
            if let Some(resolved) = resolve_known_contract_alias(contract) {
                alias = Some(resolved);
            } else if Contract::from_string(contract).is_err() {
                push_error(
                    errors,
                    ValidationCategory::InvalidContractId,
                    "contract",
                    format!(
                        "invalid contract identifier `{}`",
                        sanitize_display(contract)
                    ),
                    "use a valid 56-character `C...` Stellar contract strkey or a workspace alias (`escrow`, `vesting`, `multi-sig-wallet`, `dao-governance`, `subscription-payments`, `marketplace-royalties`)".to_owned(),
                );
            }
            Some(sanitize_display(contract))
        }
    };

    ContractValidation { display, alias }
}

fn validate_method(
    plan: &TransactionPlanInput,
    alias: Option<&'static str>,
    errors: &mut Vec<ValidationIssue>,
) -> MethodValidation {
    let Some(method) = plan
        .method
        .as_deref()
        .map(str::trim)
        .filter(|method| !method.is_empty())
    else {
        push_error(
            errors,
            ValidationCategory::MissingRequiredField,
            "method",
            "required field `method` is missing or empty".to_owned(),
            "specify the contract function name to invoke (for example `create_escrow`)".to_owned(),
        );
        return MethodValidation {
            display: None,
            spec: None,
        };
    };

    let mut spec = None;
    if !is_valid_soroban_symbol(method) {
        push_error(
            errors,
            ValidationCategory::UnsupportedMethod,
            "method",
            format!(
                "method name `{}` is not a valid Soroban symbol",
                sanitize_display(method)
            ),
            "method names must be 1..=32 ASCII characters (`a-z`, `A-Z`, `0-9`, `_`) and start with a letter or `_`".to_owned(),
        );
    } else if is_explicitly_unsupported_method(method) {
        push_error(
            errors,
            ValidationCategory::UnsupportedMethod,
            "method",
            format!("method `{method}` is not supported for Soroban contract invocation plans"),
            "choose a public exported contract function on the target contract".to_owned(),
        );
    } else if let Some(alias) = alias {
        let methods = known_methods_for_contract(alias);
        if let Some(found) = methods.iter().find(|candidate| candidate.name == method) {
            spec = Some(found);
        } else {
            let supported: Vec<&str> = methods.iter().map(|candidate| candidate.name).collect();
            push_error(
                errors,
                ValidationCategory::UnsupportedMethod,
                "method",
                format!("method `{method}` is not exported by the `{alias}` contract"),
                format!(
                    "supported `{alias}` methods include: {}",
                    supported.join(", ")
                ),
            );
        }
    }

    MethodValidation {
        display: Some(sanitize_display(method)),
        spec,
    }
}

fn validate_arguments(
    plan: &TransactionPlanInput,
    errors: &mut Vec<ValidationIssue>,
) -> ArgumentValidation {
    let mut encoded = Vec::new();
    let mut seen_names = BTreeSet::new();
    let mut provided_kinds = BTreeMap::new();
    let mut uses_named_args = false;

    for (index, raw_arg) in plan.args.iter().enumerate() {
        let field = format!("args[{index}]");
        let parsed = match normalize_raw_arg(raw_arg) {
            Ok(parsed) => parsed,
            Err(issue) => {
                errors.push(ValidationIssue { field, ..issue });
                continue;
            }
        };

        if parsed.name.is_some() {
            uses_named_args = true;
        }

        if let Some(name) = &parsed.name {
            if !is_valid_soroban_symbol(name) {
                push_error(
                    errors,
                    ValidationCategory::MalformedArgument,
                    &format!("{field}.name"),
                    format!(
                        "argument name `{}` is not a valid identifier",
                        sanitize_display(name)
                    ),
                    "use snake_case ASCII parameter names up to 32 characters".to_owned(),
                );
            } else if !seen_names.insert(name.clone()) {
                push_error(
                    errors,
                    ValidationCategory::MalformedArgument,
                    &format!("{field}.name"),
                    format!("duplicate argument `{name}`"),
                    "provide each named argument at most once".to_owned(),
                );
            }
        }

        if looks_like_secret_key(&parsed.raw_value)
            || parsed.name.as_deref().is_some_and(is_sensitive_field_name)
        {
            let target = parsed
                .name
                .as_ref()
                .map_or(field.clone(), |name| format!("args.{name}"));
            push_error(
                errors,
                ValidationCategory::SensitiveInputRejected,
                &target,
                "argument contains sensitive secret material ([REDACTED])".to_owned(),
                "never pass secret seeds or private keys as contract call arguments".to_owned(),
            );
            continue;
        }

        match encode_scval(&parsed.kind, &parsed.raw_value) {
            Ok(scval) => {
                let kind = canonical_kind(&parsed.kind);
                if let Some(name) = &parsed.name {
                    provided_kinds.insert(name.clone(), kind.clone());
                }
                encoded.push(EncodedArgumentSummary {
                    index,
                    name: parsed.name,
                    kind,
                    scval_variant: scval_variant_name(&scval).to_owned(),
                });
            }
            Err(message) => {
                let target = parsed
                    .name
                    .as_ref()
                    .map_or(field, |name| format!("args.{name}"));
                push_error(
                    errors,
                    ValidationCategory::MalformedArgument,
                    &target,
                    message,
                    hint_for_kind(&parsed.kind),
                );
            }
        }
    }

    ArgumentValidation {
        encoded,
        seen_names,
        provided_kinds,
        uses_named_args,
    }
}

fn validate_method_inputs(
    spec: &KnownMethodSpec,
    arguments: &ArgumentValidation,
    errors: &mut Vec<ValidationIssue>,
) {
    let Some(required) = spec.arg_sets.first() else {
        return;
    };

    let complete_set = spec
        .arg_sets
        .iter()
        .find(|candidate| {
            candidate
                .iter()
                .all(|(name, _)| arguments.seen_names.contains(*name))
        })
        .unwrap_or(required);

    if arguments.uses_named_args || arguments.encoded.is_empty() {
        for (name, kind) in *complete_set {
            if !arguments.seen_names.contains(*name) {
                push_error(
                    errors,
                    ValidationCategory::MissingRequiredField,
                    &format!("args.{name}"),
                    format!(
                        "missing required argument `{name}` (expected `{kind}`) for method `{}`",
                        spec.name
                    ),
                    format!(
                        "pass `--arg {name}={kind}:<value>` or include `{name}` in the plan `args` list"
                    ),
                );
            } else if let Some(actual) = arguments.provided_kinds.get(*name) {
                if actual != *kind {
                    push_error(
                        errors,
                        ValidationCategory::MalformedArgument,
                        &format!("args.{name}"),
                        format!(
                            "argument `{name}` for method `{}` expects type `{kind}`, got `{actual}`",
                            spec.name
                        ),
                        format!("encode `{name}` as `{kind}`"),
                    );
                }
            }
        }
    } else if arguments.encoded.len() < required.len() {
        for (name, kind) in &required[arguments.encoded.len()..] {
            push_error(
                errors,
                ValidationCategory::MissingRequiredField,
                &format!("args.{name}"),
                format!(
                    "missing required argument `{name}` (expected `{kind}`) for method `{}`",
                    spec.name
                ),
                format!("provide `{name}` of type `{kind}`"),
            );
        }
    }
}

pub fn render_json(report: &ValidationReport) -> Result<String> {
    serde_json::to_string_pretty(report).context("failed to serialize validation report as JSON")
}

pub fn render_text(report: &ValidationReport) -> String {
    let status = if report.valid { "VALID" } else { "INVALID" };
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Transaction Plan Validation: {status} (exit_code={})",
        report.exit_code
    );
    let _ = writeln!(
        out,
        "  Network : {}",
        report.network.as_deref().unwrap_or("<missing>")
    );
    let _ = writeln!(
        out,
        "  Contract: {}",
        report.contract.as_deref().unwrap_or("<missing>")
    );
    let _ = writeln!(
        out,
        "  Method  : {}",
        report.method.as_deref().unwrap_or("<missing>")
    );
    let _ = writeln!(out, "  Args    : {} encoded", report.encoded_args.len());

    if !report.encoded_args.is_empty() {
        let _ = writeln!(out, "\nEncoded Arguments:");
        for arg in &report.encoded_args {
            let label = arg.name.as_deref().unwrap_or("<positional>");
            let _ = writeln!(
                out,
                "  [{}] {label} ({} -> {})",
                arg.index, arg.kind, arg.scval_variant
            );
        }
    }

    if report.errors.is_empty() {
        let _ = writeln!(out, "\nResult: all transaction-plan checks passed.");
    } else {
        let _ = writeln!(out, "\nValidation Errors ({}):", report.error_count);
        for (index, issue) in report.errors.iter().enumerate() {
            let _ = writeln!(
                out,
                "  {}. [{}] {}: {}",
                index + 1,
                issue.category.as_str(),
                issue.field,
                issue.message
            );
            let _ = writeln!(out, "     hint: {}", issue.hint);
        }
    }

    out
}

fn normalize_raw_arg(raw: &RawPlanArg) -> std::result::Result<NormalizedArg, ValidationIssue> {
    match raw {
        RawPlanArg::Spec(spec) => parse_arg_spec(spec),
        RawPlanArg::Structured(structured) => {
            let name = structured
                .name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned);
            let kind = structured
                .kind
                .as_deref()
                .map(str::trim)
                .filter(|kind| !kind.is_empty())
                .ok_or_else(|| ValidationIssue {
                    category: ValidationCategory::MissingRequiredField,
                    field: "args".to_owned(),
                    message: "structured argument is missing required `kind` (or `type`) field"
                        .to_owned(),
                    hint: "specify `kind` as `address`, `i128`, `u64`, `u32`, `bool`, `symbol`, `string`, or `bytes`".to_owned(),
                })?
                .to_lowercase();

            let raw_value = match &structured.value {
                None | Some(serde_json::Value::Null) if kind != "void" => {
                    return Err(ValidationIssue {
                        category: ValidationCategory::MissingRequiredField,
                        field: "args".to_owned(),
                        message: format!(
                            "structured argument{} is missing required `value`",
                            name.as_ref()
                                .map_or(String::new(), |name| format!(" `{name}`"))
                        ),
                        hint: "provide a non-null `value` matching the declared ScVal type"
                            .to_owned(),
                    });
                }
                None | Some(serde_json::Value::Null) => String::new(),
                Some(serde_json::Value::String(value)) => value.clone(),
                Some(serde_json::Value::Number(value)) => value.to_string(),
                Some(serde_json::Value::Bool(value)) => value.to_string(),
                Some(_) => {
                    return Err(ValidationIssue {
                        category: ValidationCategory::MalformedArgument,
                        field: "args".to_owned(),
                        message: "unsupported JSON value representation for argument".to_owned(),
                        hint: "represent scalar and ScVal values as strings, numbers, or booleans"
                            .to_owned(),
                    });
                }
            };

            Ok(NormalizedArg {
                name,
                kind,
                raw_value,
            })
        }
    }
}

fn parse_arg_spec(spec: &str) -> std::result::Result<NormalizedArg, ValidationIssue> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Err(ValidationIssue {
            category: ValidationCategory::MissingRequiredField,
            field: "args".to_owned(),
            message: "argument specification cannot be empty".to_owned(),
            hint: "use `name=type:value` or `type:value` (for example `amount=i128:1000`)"
                .to_owned(),
        });
    }

    let (name, rhs) = if let Some((lhs, rhs)) = trimmed.split_once('=') {
        let name = lhs.trim();
        if name.is_empty() {
            return Err(ValidationIssue {
                category: ValidationCategory::MalformedArgument,
                field: "args".to_owned(),
                message: "argument name before `=` cannot be empty".to_owned(),
                hint: "use `name=type:value` (for example `depositor=address:G...`)".to_owned(),
            });
        }
        (Some(name.to_owned()), rhs.trim())
    } else {
        (None, trimmed)
    };

    let Some((kind_raw, value_raw)) = rhs.split_once(':') else {
        if looks_like_secret_key(rhs) {
            return Err(ValidationIssue {
                category: ValidationCategory::SensitiveInputRejected,
                field: "args".to_owned(),
                message: "argument contains a secret seed ([REDACTED])".to_owned(),
                hint: "never pass secret seeds as contract arguments".to_owned(),
            });
        }
        return Err(ValidationIssue {
            category: ValidationCategory::MalformedArgument,
            field: "args".to_owned(),
            message: format!(
                "argument `{}` is missing an explicit ScVal type prefix",
                sanitize_display(trimmed)
            ),
            hint: "format arguments as `name=type:value` or `type:value` (for example `amount=i128:500`)".to_owned(),
        });
    };

    let kind = kind_raw.trim().to_lowercase();
    let raw_value = value_raw.trim().to_owned();
    if raw_value.is_empty() && !matches!(kind.as_str(), "void" | "string" | "bytes") {
        return Err(ValidationIssue {
            category: ValidationCategory::MissingRequiredField,
            field: "args".to_owned(),
            message: format!(
                "argument{} of type `{kind}` has an empty value",
                name.as_ref()
                    .map_or(String::new(), |name| format!(" `{name}`"))
            ),
            hint: hint_for_kind(&kind),
        });
    }

    Ok(NormalizedArg {
        name,
        kind,
        raw_value,
    })
}

pub fn encode_scval(kind: &str, raw_value: &str) -> std::result::Result<ScVal, String> {
    match kind.trim().to_lowercase().as_str() {
        "void" | "unit" => {
            if !matches!(raw_value, "" | "null" | "()") {
                return Err(format!(
                    "`void` argument does not accept value `{}`",
                    sanitize_display(raw_value)
                ));
            }
            Ok(ScVal::Void)
        }
        "bool" => match raw_value {
            "true" => Ok(ScVal::Bool(true)),
            "false" => Ok(ScVal::Bool(false)),
            _ => Err(format!(
                "invalid `bool` value `{}` (expected `true` or `false`)",
                sanitize_display(raw_value)
            )),
        },
        "u32" => raw_value
            .parse::<u32>()
            .map(ScVal::U32)
            .map_err(|_| format!("invalid `u32` integer `{}`", sanitize_display(raw_value))),
        "i32" => raw_value
            .parse::<i32>()
            .map(ScVal::I32)
            .map_err(|_| format!("invalid `i32` integer `{}`", sanitize_display(raw_value))),
        "u64" => raw_value
            .parse::<u64>()
            .map(ScVal::U64)
            .map_err(|_| format!("invalid `u64` integer `{}`", sanitize_display(raw_value))),
        "i64" => raw_value
            .parse::<i64>()
            .map(ScVal::I64)
            .map_err(|_| format!("invalid `i64` integer `{}`", sanitize_display(raw_value))),
        "u128" => {
            let parsed = raw_value
                .parse::<u128>()
                .map_err(|_| format!("invalid `u128` integer `{}`", sanitize_display(raw_value)))?;
            Ok(ScVal::U128(UInt128Parts {
                hi: (parsed >> 64) as u64,
                lo: parsed as u64,
            }))
        }
        "i128" => {
            let parsed = raw_value
                .parse::<i128>()
                .map_err(|_| format!("invalid `i128` integer `{}`", sanitize_display(raw_value)))?;
            Ok(ScVal::I128(Int128Parts {
                hi: (parsed >> 64) as i64,
                lo: parsed as u64,
            }))
        }
        "symbol" | "sym" => {
            if !is_valid_soroban_symbol(raw_value) {
                return Err(format!(
                    "invalid Soroban symbol `{}` (must be 1..=32 ASCII alphanumeric/`_` characters)",
                    sanitize_display(raw_value)
                ));
            }
            let symbol: ScSymbol = raw_value.try_into().map_err(|_| {
                format!("failed to encode symbol `{}`", sanitize_display(raw_value))
            })?;
            Ok(ScVal::Symbol(symbol))
        }
        "string" | "str" => {
            let string: StringM = raw_value.try_into().map_err(|_| {
                format!(
                    "failed to encode ScVal string `{}`",
                    sanitize_display(raw_value)
                )
            })?;
            Ok(ScVal::String(ScString(string)))
        }
        "bytes" | "hex" => {
            let clean = raw_value.strip_prefix("0x").unwrap_or(raw_value);
            if clean.len() % 2 != 0 || !clean.chars().all(|ch| ch.is_ascii_hexdigit()) {
                return Err(format!(
                    "invalid hex byte string `{}` (expected an even number of hex digits)",
                    sanitize_display(raw_value)
                ));
            }
            let mut decoded = Vec::with_capacity(clean.len() / 2);
            for chunk in clean.as_bytes().chunks(2) {
                let pair = std::str::from_utf8(chunk).unwrap_or_default();
                let byte = u8::from_str_radix(pair, 16).map_err(|_| {
                    format!(
                        "invalid hex byte `{pair}` in `{}`",
                        sanitize_display(raw_value)
                    )
                })?;
                decoded.push(byte);
            }
            let bytes: ScBytes = decoded
                .try_into()
                .map_err(|_| "byte array exceeds maximum ScBytes length".to_owned())?;
            Ok(ScVal::Bytes(bytes))
        }
        "address" | "addr" => match Strkey::from_string(raw_value) {
            Ok(Strkey::PublicKeyEd25519(public_key)) => Ok(ScVal::Address(ScAddress::Account(
                AccountId(PublicKey::PublicKeyTypeEd25519(Uint256(public_key.0))),
            ))),
            Ok(Strkey::Contract(contract)) => Ok(ScVal::Address(ScAddress::Contract(
                Hash(contract.0),
            ))),
            _ => Err(format!(
                "invalid Stellar address `{}` (expected a `G...` account or `C...` contract strkey)",
                sanitize_display(raw_value)
            )),
        },
        other => Err(format!(
            "unsupported ScVal argument type `{}`",
            sanitize_display(other)
        )),
    }
}

fn canonical_kind(kind: &str) -> String {
    match kind.trim().to_lowercase().as_str() {
        "unit" => "void".to_owned(),
        "sym" => "symbol".to_owned(),
        "str" => "string".to_owned(),
        "hex" => "bytes".to_owned(),
        "addr" => "address".to_owned(),
        other => other.to_owned(),
    }
}

fn scval_variant_name(value: &ScVal) -> &'static str {
    match value {
        ScVal::Void => "ScVal::Void",
        ScVal::Bool(_) => "ScVal::Bool",
        ScVal::U32(_) => "ScVal::U32",
        ScVal::I32(_) => "ScVal::I32",
        ScVal::U64(_) => "ScVal::U64",
        ScVal::I64(_) => "ScVal::I64",
        ScVal::U128(_) => "ScVal::U128",
        ScVal::I128(_) => "ScVal::I128",
        ScVal::Symbol(_) => "ScVal::Symbol",
        ScVal::String(_) => "ScVal::String",
        ScVal::Bytes(_) => "ScVal::Bytes",
        ScVal::Address(_) => "ScVal::Address",
        _ => "ScVal::Other",
    }
}

fn hint_for_kind(kind: &str) -> String {
    match canonical_kind(kind).as_str() {
        "address" => "use a valid 56-character `G...` account or `C...` contract strkey".to_owned(),
        "u32" | "i32" | "u64" | "i64" | "u128" | "i128" => {
            format!("provide a valid base-10 `{}` integer within bounds", canonical_kind(kind))
        }
        "bool" => "provide `true` or `false`".to_owned(),
        "symbol" => "provide 1..=32 ASCII alphanumeric or `_` characters".to_owned(),
        "bytes" => "provide an even-length hexadecimal string (optional `0x` prefix)".to_owned(),
        "string" => "provide a valid UTF-8 string".to_owned(),
        _ => "use one of the supported ScVal types: `address`, `i128`, `u128`, `u64`, `i64`, `u32`, `i32`, `bool`, `symbol`, `string`, `bytes`, `void`".to_owned(),
    }
}

fn expected_passphrase_for_network(network: &str) -> Option<&'static str> {
    match network {
        "testnet" => Some("Test SDF Network ; September 2015"),
        "futurenet" => Some("Test SDF Future Network ; October 2022"),
        "standalone" | "local" => Some("Standalone Network ; February 2017"),
        "mainnet" | "public" => Some("Public Global Stellar Network ; September 2015"),
        _ => None,
    }
}

fn canonical_network_name(network: &str) -> &str {
    match network {
        "public" => "mainnet",
        "local" => "standalone",
        other => other,
    }
}

fn resolve_known_contract_alias(raw: &str) -> Option<&'static str> {
    match raw.trim().to_lowercase().as_str() {
        "escrow" | "soroban-forge-escrow" => Some("escrow"),
        "vesting" | "token-vesting" | "soroban-forge-vesting" => Some("vesting"),
        "multi-sig-wallet" | "multi-sig" | "multisig" | "soroban-forge-multi-sig-wallet" => {
            Some("multi-sig-wallet")
        }
        "dao-governance" | "dao" | "soroban-forge-dao-governance" => Some("dao-governance"),
        "subscription-payments" | "subscription" | "soroban-forge-subscription-payments" => {
            Some("subscription-payments")
        }
        "marketplace-royalties" | "marketplace" | "soroban-forge-marketplace-royalties" => {
            Some("marketplace-royalties")
        }
        _ => None,
    }
}

const CREATE_ESCROW_CURRENT: ArgumentNames = &[
    ("buyer", "address"),
    ("seller", "address"),
    ("arbiter", "address"),
    ("token", "address"),
    ("amount", "i128"),
    ("timeout", "u64"),
];

const CREATE_ESCROW_LEGACY: ArgumentNames = &[
    ("depositor", "address"),
    ("beneficiary", "address"),
    ("arbiter", "address"),
    ("token", "address"),
    ("amount", "i128"),
    ("timeout_duration", "u64"),
];

const CREATE_SCHEDULE_CURRENT: ArgumentNames = &[
    ("beneficiary", "address"),
    ("token", "address"),
    ("total_amount", "i128"),
    ("cliff", "u64"),
    ("duration", "u64"),
];

const CREATE_SCHEDULE_LEGACY: ArgumentNames = &[
    ("admin", "address"),
    ("beneficiary", "address"),
    ("token", "address"),
    ("total_amount", "i128"),
    ("start_time", "u64"),
    ("cliff_duration", "u64"),
    ("total_duration", "u64"),
];

const SUBSCRIBE_CURRENT: ArgumentNames = &[
    ("subscriber", "address"),
    ("provider", "address"),
    ("token", "address"),
    ("amount", "i128"),
    ("period", "u64"),
];

const SUBSCRIBE_LEGACY: ArgumentNames = &[
    ("merchant", "address"),
    ("token", "address"),
    ("amount", "i128"),
    ("interval", "u64"),
];

fn known_methods_for_contract(alias: &str) -> &'static [KnownMethodSpec] {
    match alias {
        "escrow" => &[
            KnownMethodSpec {
                name: "create_escrow",
                arg_sets: &[CREATE_ESCROW_CURRENT, CREATE_ESCROW_LEGACY],
            },
            KnownMethodSpec {
                name: "deposit",
                arg_sets: &[&[("escrow_id", "u64")]],
            },
            KnownMethodSpec {
                name: "release",
                arg_sets: &[&[("escrow_id", "u64")]],
            },
            KnownMethodSpec {
                name: "release_partial",
                arg_sets: &[&[("escrow_id", "u64"), ("amount", "i128")]],
            },
            KnownMethodSpec {
                name: "refund",
                arg_sets: &[&[("escrow_id", "u64")]],
            },
            KnownMethodSpec {
                name: "dispute",
                arg_sets: &[&[("escrow_id", "u64"), ("claimant", "address")]],
            },
            KnownMethodSpec {
                name: "resolve",
                arg_sets: &[&[("escrow_id", "u64"), ("in_favor_of_seller", "bool")]],
            },
            KnownMethodSpec {
                name: "cancel",
                arg_sets: &[&[("escrow_id", "u64")]],
            },
            KnownMethodSpec {
                name: "get_status",
                arg_sets: &[&[("escrow_id", "u64")]],
            },
            KnownMethodSpec {
                name: "get_escrow",
                arg_sets: &[&[("escrow_id", "u64")]],
            },
            KnownMethodSpec {
                name: "escrows_for_participant",
                arg_sets: &[],
            },
            KnownMethodSpec {
                name: "touch_ttl",
                arg_sets: &[&[("escrow_id", "u64")]],
            },
        ],
        "vesting" => &[
            KnownMethodSpec {
                name: "create_schedule",
                arg_sets: &[CREATE_SCHEDULE_CURRENT, CREATE_SCHEDULE_LEGACY],
            },
            KnownMethodSpec {
                name: "create_tranche_schedule",
                arg_sets: &[],
            },
            KnownMethodSpec {
                name: "get_tranche_schedule",
                arg_sets: &[&[("schedule_id", "u64")]],
            },
            KnownMethodSpec {
                name: "claim",
                arg_sets: &[&[("schedule_id", "u64")]],
            },
            KnownMethodSpec {
                name: "claimable",
                arg_sets: &[&[("schedule_id", "u64")]],
            },
            KnownMethodSpec {
                name: "get_status",
                arg_sets: &[&[("schedule_id", "u64")]],
            },
            KnownMethodSpec {
                name: "revoke",
                arg_sets: &[&[("schedule_id", "u64")]],
            },
            KnownMethodSpec {
                name: "get_schedule",
                arg_sets: &[&[("schedule_id", "u64")]],
            },
        ],
        "multi-sig-wallet" => &[
            KnownMethodSpec {
                name: "initialize",
                arg_sets: &[],
            },
            KnownMethodSpec {
                name: "submit",
                arg_sets: &[&[
                    ("submitter", "address"),
                    ("target", "address"),
                    ("tx", "bytes"),
                ]],
            },
            KnownMethodSpec {
                name: "submit_call",
                arg_sets: &[&[
                    ("submitter", "address"),
                    ("target", "address"),
                    ("fn_name", "symbol"),
                ]],
            },
            KnownMethodSpec {
                name: "submit_transaction",
                arg_sets: &[&[
                    ("proposer", "address"),
                    ("target_contract", "address"),
                    ("function", "symbol"),
                ]],
            },
            KnownMethodSpec {
                name: "confirm",
                arg_sets: &[&[("tx_id", "u64"), ("signer", "address")]],
            },
            KnownMethodSpec {
                name: "approve",
                arg_sets: &[&[("signer", "address"), ("tx_id", "u64")]],
            },
            KnownMethodSpec {
                name: "reject",
                arg_sets: &[&[("tx_id", "u64"), ("signer", "address")]],
            },
            KnownMethodSpec {
                name: "execute",
                arg_sets: &[&[("tx_id", "u64")]],
            },
            KnownMethodSpec {
                name: "get_tx",
                arg_sets: &[&[("tx_id", "u64")]],
            },
            KnownMethodSpec {
                name: "get_transaction",
                arg_sets: &[&[("tx_id", "u64")]],
            },
            KnownMethodSpec {
                name: "deposit",
                arg_sets: &[],
            },
            KnownMethodSpec {
                name: "submit_withdrawal",
                arg_sets: &[],
            },
            KnownMethodSpec {
                name: "set_withdrawal_limit",
                arg_sets: &[],
            },
            KnownMethodSpec {
                name: "remove_withdrawal_limit",
                arg_sets: &[],
            },
            KnownMethodSpec {
                name: "get_withdrawal_limit",
                arg_sets: &[&[("token", "address")]],
            },
            KnownMethodSpec {
                name: "check_withdrawal",
                arg_sets: &[&[("token", "address"), ("amount", "i128")]],
            },
            KnownMethodSpec {
                name: "balance",
                arg_sets: &[&[("token", "address")]],
            },
            KnownMethodSpec {
                name: "get_threshold",
                arg_sets: &[],
            },
            KnownMethodSpec {
                name: "get_owners",
                arg_sets: &[],
            },
            KnownMethodSpec {
                name: "is_owner",
                arg_sets: &[&[("address", "address")]],
            },
            KnownMethodSpec {
                name: "get_tx_count",
                arg_sets: &[],
            },
        ],
        "dao-governance" => &[
            KnownMethodSpec {
                name: "configure_bond",
                arg_sets: &[],
            },
            KnownMethodSpec {
                name: "get_bond_config",
                arg_sets: &[],
            },
            KnownMethodSpec {
                name: "propose",
                arg_sets: &[&[
                    ("proposer", "address"),
                    ("target", "address"),
                    ("action", "bytes"),
                    ("duration", "u64"),
                ]],
            },
            KnownMethodSpec {
                name: "create_proposal",
                arg_sets: &[&[("proposer", "address"), ("title", "symbol")]],
            },
            KnownMethodSpec {
                name: "vote",
                arg_sets: &[&[
                    ("proposal_id", "u64"),
                    ("voter", "address"),
                    ("support", "bool"),
                ]],
            },
            KnownMethodSpec {
                name: "execute",
                arg_sets: &[&[("proposal_id", "u64")]],
            },
            KnownMethodSpec {
                name: "finalize",
                arg_sets: &[&[("proposal_id", "u64")]],
            },
            KnownMethodSpec {
                name: "cancel_proposal",
                arg_sets: &[],
            },
            KnownMethodSpec {
                name: "get_proposal",
                arg_sets: &[&[("proposal_id", "u64")]],
            },
            KnownMethodSpec {
                name: "get_proposal_count",
                arg_sets: &[],
            },
            KnownMethodSpec {
                name: "has_voted",
                arg_sets: &[&[("proposal_id", "u64"), ("voter", "address")]],
            },
        ],
        "subscription-payments" => &[
            KnownMethodSpec {
                name: "subscribe",
                arg_sets: &[SUBSCRIBE_CURRENT, SUBSCRIBE_LEGACY],
            },
            KnownMethodSpec {
                name: "create_plan",
                arg_sets: &[SUBSCRIBE_LEGACY],
            },
            KnownMethodSpec {
                name: "authorize_provider",
                arg_sets: &[&[("subscriber", "address"), ("provider", "address")]],
            },
            KnownMethodSpec {
                name: "revoke_provider",
                arg_sets: &[&[("subscriber", "address"), ("provider", "address")]],
            },
            KnownMethodSpec {
                name: "is_provider_authorized",
                arg_sets: &[&[("subscriber", "address"), ("provider", "address")]],
            },
            KnownMethodSpec {
                name: "subscribe_on_behalf_of",
                arg_sets: &[&[
                    ("provider", "address"),
                    ("subscriber", "address"),
                    ("token", "address"),
                    ("amount", "i128"),
                    ("period", "u64"),
                ]],
            },
            KnownMethodSpec {
                name: "charge",
                arg_sets: &[&[("subscription_id", "u64")]],
            },
            KnownMethodSpec {
                name: "charge_catchup",
                arg_sets: &[],
            },
            KnownMethodSpec {
                name: "pause",
                arg_sets: &[&[("subscription_id", "u64")]],
            },
            KnownMethodSpec {
                name: "resume",
                arg_sets: &[&[("subscription_id", "u64")]],
            },
            KnownMethodSpec {
                name: "cancel",
                arg_sets: &[&[("subscription_id", "u64")]],
            },
            KnownMethodSpec {
                name: "get_subscription",
                arg_sets: &[&[("subscription_id", "u64")]],
            },
        ],
        "marketplace-royalties" => &[
            KnownMethodSpec {
                name: "set_royalty",
                arg_sets: &[&[
                    ("collection", "address"),
                    ("recipient", "address"),
                    ("bps", "u32"),
                ]],
            },
            KnownMethodSpec {
                name: "distribute",
                arg_sets: &[&[
                    ("collection", "address"),
                    ("token", "address"),
                    ("payer", "address"),
                    ("seller", "address"),
                    ("amount", "i128"),
                ]],
            },
            KnownMethodSpec {
                name: "settle_sale",
                arg_sets: &[&[
                    ("collection", "address"),
                    ("token", "address"),
                    ("payer", "address"),
                    ("seller", "address"),
                    ("amount", "i128"),
                ]],
            },
            KnownMethodSpec {
                name: "settle_sales",
                arg_sets: &[],
            },
            KnownMethodSpec {
                name: "create_listing",
                arg_sets: &[&[
                    ("seller", "address"),
                    ("token", "address"),
                    ("price", "i128"),
                ]],
            },
            KnownMethodSpec {
                name: "buy",
                arg_sets: &[&[("buyer", "address"), ("listing_id", "u64")]],
            },
            KnownMethodSpec {
                name: "cancel_listing",
                arg_sets: &[&[("listing_id", "u64")]],
            },
            KnownMethodSpec {
                name: "get_listing",
                arg_sets: &[&[("listing_id", "u64")]],
            },
            KnownMethodSpec {
                name: "get_royalty",
                arg_sets: &[&[("collection", "address")]],
            },
            KnownMethodSpec {
                name: "get_settlement_summary",
                arg_sets: &[&[("collection", "address")]],
            },
            KnownMethodSpec {
                name: "touch_ttl",
                arg_sets: &[&[("collection", "address")]],
            },
        ],
        _ => &[],
    }
}

fn is_valid_soroban_symbol(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 32 {
        return false;
    }
    if !(bytes[0].is_ascii_alphabetic() || bytes[0] == b'_') {
        return false;
    }
    bytes
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
}

fn is_explicitly_unsupported_method(method: &str) -> bool {
    matches!(
        method,
        "__constructor" | "__eval" | "sudo" | "raw_host_call" | "upgrade_unchecked"
    )
}

fn is_sensitive_field_name(name: &str) -> bool {
    matches!(
        name.trim().to_lowercase().as_str(),
        "secret" | "secret_key" | "private_key" | "seed" | "mnemonic" | "passphrase"
    )
}

fn looks_like_secret_key(value: &str) -> bool {
    let trimmed = value.trim();
    if ed25519::PrivateKey::from_string(trimmed).is_ok() {
        return true;
    }
    trimmed.len() == 56
        && trimmed.starts_with('S')
        && trimmed
            .chars()
            .all(|ch| matches!(ch, 'A'..='Z' | '2'..='7'))
}

fn sanitize_display(value: &str) -> String {
    let trimmed = value.trim();
    if looks_like_secret_key(trimmed) {
        return "[REDACTED]".to_owned();
    }
    for token in trimmed.split(['=', ':', ' ', ',', '"', '\'']) {
        if looks_like_secret_key(token) {
            return "[REDACTED]".to_owned();
        }
    }
    trimmed.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_CONTRACT_ID: &str = "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC";
    const SAMPLE_ACCOUNT_A: &str = "GAAZI4TCR3TY5OJHCTJC2A4QSY6CJWJH5IAJTGKIN2ER7LBNVKOCCWN7";
    const SAMPLE_ACCOUNT_B: &str = "GBDEVU63Y6NTHJQQZIKVTC23NWLQVP3WJ2RI2OTSJTNYOIGICST6DUXR";
    const SAMPLE_SECRET_SEED: &str = "SB7WL5N4J7QGPX2Z2X2Z2X2Z2X2Z2X2Z2X2Z2X2Z2X2Z2X2Z2X2Z2X2Z";

    fn valid_escrow_plan() -> TransactionPlanInput {
        TransactionPlanInput {
            schema: Some(PLAN_SCHEMA_V1.to_owned()),
            network: Some("testnet".to_owned()),
            network_passphrase: Some("Test SDF Network ; September 2015".to_owned()),
            contract: Some(SAMPLE_CONTRACT_ID.to_owned()),
            contract_type: Some("escrow".to_owned()),
            method: Some("create_escrow".to_owned()),
            source: Some(SAMPLE_ACCOUNT_A.to_owned()),
            args: vec![
                RawPlanArg::Spec(format!("buyer=address:{SAMPLE_ACCOUNT_A}")),
                RawPlanArg::Spec(format!("seller=address:{SAMPLE_ACCOUNT_B}")),
                RawPlanArg::Spec(format!("arbiter=address:{SAMPLE_ACCOUNT_A}")),
                RawPlanArg::Spec(format!("token=address:{SAMPLE_CONTRACT_ID}")),
                RawPlanArg::Spec("amount=i128:250000".to_owned()),
                RawPlanArg::Spec("timeout=u64:86400".to_owned()),
            ],
            ..TransactionPlanInput::default()
        }
    }

    #[test]
    fn valid_plan_produces_clean_report_and_zero_exit_code() {
        let report = validate_plan(&valid_escrow_plan(), Some("testnet"));

        assert!(report.valid, "expected valid report, got: {report:#?}");
        assert_eq!(report.exit_code, EXIT_OK);
        assert_eq!(report.error_count, 0);
        assert_eq!(report.encoded_args.len(), 6);
        assert_eq!(report.encoded_args[0].scval_variant, "ScVal::Address");
        assert_eq!(report.encoded_args[4].scval_variant, "ScVal::I128");
        assert_eq!(report.encoded_args[5].scval_variant, "ScVal::U64");

        let text = render_text(&report);
        assert!(text.contains("Transaction Plan Validation: VALID (exit_code=0)"));
        assert!(text.contains("Result: all transaction-plan checks passed."));

        let json = render_json(&report).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["schema"], REPORT_SCHEMA_V1);
        assert_eq!(parsed["valid"], true);
        assert_eq!(parsed["exit_code"], 0);
        assert_eq!(parsed["error_count"], 0);
    }

    #[test]
    fn multiple_independent_errors_are_reported_together() {
        let plan = TransactionPlanInput {
            schema: Some("soroban-forge/transaction-plan@99".to_owned()),
            network: Some("testnet".to_owned()),
            network_passphrase: Some("Wrong Passphrase".to_owned()),
            contract: Some("not-a-valid-contract-id".to_owned()),
            contract_type: Some("escrow".to_owned()),
            method: Some("nonexistent_method".to_owned()),
            args: vec![
                RawPlanArg::Spec("amount=i128:not_a_number".to_owned()),
                RawPlanArg::Spec("bad_arg_without_type".to_owned()),
            ],
            ..TransactionPlanInput::default()
        };

        let report = validate_plan(&plan, Some("futurenet"));
        assert!(!report.valid);
        assert_eq!(report.exit_code, EXIT_INVALID_PLAN);

        let categories: Vec<ValidationCategory> =
            report.errors.iter().map(|issue| issue.category).collect();
        for category in [
            ValidationCategory::SchemaMismatch,
            ValidationCategory::NetworkMismatch,
            ValidationCategory::InvalidContractId,
            ValidationCategory::UnsupportedMethod,
            ValidationCategory::MalformedArgument,
        ] {
            assert!(categories.contains(&category), "missing {category:?}");
        }
        assert!(report.error_count >= 5);
    }

    #[test]
    fn missing_required_fields_and_method_parameters_are_detected() {
        let empty_report = validate_plan(&TransactionPlanInput::default(), None);
        assert!(!empty_report.valid);

        let missing_fields: Vec<&str> = empty_report
            .errors
            .iter()
            .map(|issue| issue.field.as_str())
            .collect();
        assert!(missing_fields.contains(&"network"));
        assert!(missing_fields.contains(&"contract"));
        assert!(missing_fields.contains(&"method"));

        let partial_escrow = TransactionPlanInput {
            network: Some("testnet".to_owned()),
            contract: Some("escrow".to_owned()),
            method: Some("release_partial".to_owned()),
            args: vec![RawPlanArg::Spec("escrow_id=u64:1".to_owned())],
            ..TransactionPlanInput::default()
        };
        let report = validate_plan(&partial_escrow, None);
        assert!(!report.valid);
        assert!(report.errors.iter().any(|issue| {
            issue.category == ValidationCategory::MissingRequiredField
                && issue.field == "args.amount"
        }));
    }

    #[test]
    fn sensitive_values_are_rejected_and_never_echoed() {
        let plan = TransactionPlanInput {
            network: Some("testnet".to_owned()),
            contract: Some(SAMPLE_CONTRACT_ID.to_owned()),
            method: Some("deposit".to_owned()),
            source: Some(SAMPLE_SECRET_SEED.to_owned()),
            secret: Some(SAMPLE_SECRET_SEED.to_owned()),
            args: vec![
                RawPlanArg::Spec(format!("signer=address:{SAMPLE_SECRET_SEED}")),
                RawPlanArg::Spec("secret_key=string:my-super-secret-value".to_owned()),
            ],
            ..TransactionPlanInput::default()
        };

        let report = validate_plan(&plan, None);
        assert!(!report.valid);
        assert!(report
            .errors
            .iter()
            .any(|issue| issue.category == ValidationCategory::SensitiveInputRejected));

        let text = render_text(&report);
        let json = render_json(&report).unwrap();
        assert!(!text.contains(SAMPLE_SECRET_SEED));
        assert!(!json.contains(SAMPLE_SECRET_SEED));
        assert!(!text.contains("my-super-secret-value"));
        assert!(!json.contains("my-super-secret-value"));
    }

    #[test]
    fn parses_json_and_toml_plan_documents() {
        let json_doc = r#"{
            "schema": "soroban-forge/transaction-plan@1",
            "network": "testnet",
            "contract": "escrow",
            "method": "deposit",
            "args": [
                { "name": "escrow_id", "type": "u64", "value": 42 }
            ]
        }"#;
        let json_plan = parse_plan_str(json_doc, Some("json")).unwrap();
        assert!(validate_plan(&json_plan, Some("testnet")).valid);

        let toml_doc = r#"
            schema = "soroban-forge/transaction-plan@1"
            network = "testnet"
            contract = "escrow"
            method = "deposit"
            args = ["escrow_id=u64:42"]
        "#;
        let toml_plan = parse_plan_str(toml_doc, Some("toml")).unwrap();
        assert!(validate_plan(&toml_plan, Some("testnet")).valid);
    }

    #[test]
    fn scval_encoding_supports_supported_types() {
        assert!(encode_scval("u32", "42").is_ok());
        assert!(encode_scval("i32", "-7").is_ok());
        assert!(encode_scval("u64", "100000").is_ok());
        assert!(encode_scval("i64", "-100000").is_ok());
        assert!(encode_scval("u128", "340282366920938463463374607431768211455").is_ok());
        assert!(encode_scval("i128", "-170141183460469231731687303715884105728").is_ok());
        assert!(encode_scval("bool", "true").is_ok());
        assert!(encode_scval("symbol", "transfer_ok").is_ok());
        assert!(encode_scval("string", "hello soroban").is_ok());
        assert!(encode_scval("bytes", "0xdeadbeef").is_ok());
        assert!(encode_scval("address", SAMPLE_ACCOUNT_A).is_ok());
        assert!(encode_scval("address", SAMPLE_CONTRACT_ID).is_ok());
        assert!(encode_scval("void", "").is_ok());

        assert!(encode_scval("bytes", "0x123").is_err());
        assert!(encode_scval("symbol", "symbol_longer_than_thirty_two_characters_limit").is_err());
        assert!(encode_scval("unknown_kind", "123").is_err());
    }

    #[test]
    fn bundled_plan_fixtures_validate_deterministically() {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let valid_plan =
            load_plan_file(&manifest_dir.join("fixtures/plans/valid-create-escrow.json")).unwrap();
        let valid_report = validate_plan(&valid_plan, Some("testnet"));
        assert!(valid_report.valid, "{}", render_text(&valid_report));

        let invalid_plan =
            load_plan_file(&manifest_dir.join("fixtures/plans/invalid-multi-error.json")).unwrap();
        let invalid_report = validate_plan(&invalid_plan, Some("futurenet"));
        assert!(!invalid_report.valid);
        assert_eq!(invalid_report.exit_code, EXIT_INVALID_PLAN);
        assert!(invalid_report.error_count >= 4);
    }
}
