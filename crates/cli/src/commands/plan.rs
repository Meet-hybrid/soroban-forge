use crate::cli::ValidatePlanArgs;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use stellar_strkey::{ed25519, Strkey};
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
pub struct EncodedArgument {
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
    pub encoded_args: Vec<EncodedArgument>,
    pub error_count: usize,
    pub errors: Vec<ValidationIssue>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct TransactionPlan {
    schema: Option<String>,
    version: Option<String>,
    network: Option<String>,
    network_passphrase: Option<String>,
    expected_network: Option<String>,
    #[serde(alias = "contract_id")]
    contract: Option<String>,
    contract_type: Option<String>,
    #[serde(alias = "function")]
    method: Option<String>,
    #[serde(alias = "source_account")]
    source: Option<String>,
    secret: Option<String>,
    #[serde(alias = "secret_key")]
    private_key: Option<String>,
    calls: Vec<PlanCall>,
    args: Vec<RawArg>,
    supported_methods: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct PlanCall {
    contract: Option<String>,
    contract_type: Option<String>,
    method: Option<String>,
    network: Option<String>,
    network_passphrase: Option<String>,
    source: Option<String>,
    args: Vec<RawArg>,
    supported_methods: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
enum RawArg {
    Spec(String),
    Structured(StructuredArg),
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct StructuredArg {
    name: Option<String>,
    #[serde(alias = "type")]
    kind: Option<String>,
    value: Option<serde_json::Value>,
}

struct NormalizedArg {
    name: Option<String>,
    kind: String,
    raw_value: String,
}

pub fn run(args: ValidatePlanArgs) -> Result<()> {
    let report = build_report(&args);
    let output = if args.json {
        render_json(&report)?
    } else {
        render_text(&report)
    };
    println!("{output}");
    if report.exit_code != EXIT_OK {
        std::process::exit(report.exit_code);
    }
    Ok(())
}

fn build_report(args: &ValidatePlanArgs) -> ValidationReport {
    let Some(path) = args.plan.as_deref().or(args.file.as_deref()) else {
        return report_from_issue(
            ValidationCategory::MissingRequiredField,
            "plan",
            "no transaction-plan path was provided".to_owned(),
            "pass a path positionally or with `--plan`".to_owned(),
        );
    };

    match load_plan(path) {
        Ok(plan) => validate_plan(&plan, args),
        Err(error) => report_from_issue(
            ValidationCategory::SchemaMismatch,
            "plan",
            format!("could not load transaction plan: {error}"),
            "provide a readable JSON or TOML transaction-plan file".to_owned(),
        ),
    }
}

fn load_plan(path: &Path) -> Result<TransactionPlan> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read plan file at {}", path.display()))?;
    let is_toml = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("toml"));
    parse_plan(&raw, is_toml)
}

fn parse_plan(raw: &str, is_toml: bool) -> Result<TransactionPlan> {
    if is_toml {
        toml::from_str(raw).context("failed to parse TOML transaction plan")
    } else {
        serde_json::from_str(raw).context("failed to parse JSON transaction plan")
    }
}

fn validate_plan(plan: &TransactionPlan, args: &ValidatePlanArgs) -> ValidationReport {
    let mut errors = Vec::new();
    validate_schema(plan, &mut errors);
    validate_secrets(plan, &mut errors);

    let fallback_call = PlanCall::default();
    let calls: Vec<&PlanCall> = if plan.calls.is_empty() {
        vec![&fallback_call]
    } else {
        plan.calls.iter().collect()
    };

    let mut encoded_args = Vec::new();
    for (index, call) in calls.iter().enumerate() {
        let prefix = if plan.calls.is_empty() {
            String::new()
        } else {
            format!("calls[{index}].")
        };

        let network = effective_opt(
            args.network.as_deref(),
            call.network.as_deref(),
            plan.network.as_deref(),
        );
        let passphrase = effective_opt(
            None,
            call.network_passphrase.as_deref(),
            plan.network_passphrase.as_deref(),
        );
        let expected_network = args
            .expected_network
            .as_deref()
            .or(plan.expected_network.as_deref());
        validate_network(
            network.as_deref(),
            passphrase.as_deref(),
            expected_network,
            &format!("{prefix}network"),
            &mut errors,
        );

        let source = effective_opt(
            args.source.as_deref(),
            call.source.as_deref(),
            plan.source.as_deref(),
        );
        validate_source(source.as_deref(), &format!("{prefix}source"), &mut errors);

        let contract = effective_opt(
            args.contract.as_deref(),
            call.contract.as_deref(),
            plan.contract.as_deref(),
        );
        validate_contract(
            contract.as_deref(),
            &format!("{prefix}contract"),
            &mut errors,
        );

        let alias_hint = known_alias(
            call.contract_type
                .as_deref()
                .or(plan.contract_type.as_deref())
                .unwrap_or_default(),
        )
        .or_else(|| known_alias(contract.as_deref().unwrap_or_default()));
        let method = effective_opt(
            args.method.as_deref(),
            call.method.as_deref(),
            plan.method.as_deref(),
        );
        let supported: Vec<String> = if call.supported_methods.is_empty() {
            plan.supported_methods.clone()
        } else {
            call.supported_methods.clone()
        };
        validate_method(
            method.as_deref(),
            alias_hint,
            &supported,
            &format!("{prefix}method"),
            &mut errors,
        );

        let raw_args: Vec<RawArg> = if !args.args.is_empty() {
            args.args.iter().cloned().map(RawArg::Spec).collect()
        } else if !call.args.is_empty() {
            call.args.clone()
        } else {
            plan.args.clone()
        };
        validate_arguments(
            &raw_args,
            &format!("{prefix}args"),
            &mut errors,
            &mut encoded_args,
        );
    }

    let first = calls[0];
    finalize_report(
        effective_opt(
            args.network.as_deref(),
            first.network.as_deref(),
            plan.network.as_deref(),
        )
        .as_deref(),
        effective_opt(
            args.contract.as_deref(),
            first.contract.as_deref(),
            plan.contract.as_deref(),
        )
        .as_deref(),
        effective_opt(
            args.method.as_deref(),
            first.method.as_deref(),
            plan.method.as_deref(),
        )
        .as_deref(),
        encoded_args,
        errors,
    )
}

fn effective_opt(cli: Option<&str>, call: Option<&str>, plan: Option<&str>) -> Option<String> {
    cli.or(call)
        .or(plan)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
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

fn validate_schema(plan: &TransactionPlan, errors: &mut Vec<ValidationIssue>) {
    if plan.schema.is_none() && plan.version.is_none() {
        push_error(
            errors,
            ValidationCategory::MissingRequiredField,
            "version",
            "plan has neither `version` nor `schema`".to_owned(),
            format!("set `version` to `1` or `schema` to `{PLAN_SCHEMA_V1}`"),
        );
    }
    if let Some(schema) = non_empty(plan.schema.as_deref()) {
        if schema != PLAN_SCHEMA_V1 {
            push_error(
                errors,
                ValidationCategory::SchemaMismatch,
                "schema",
                "plan declares an unsupported schema".to_owned(),
                format!("set `schema` to `{PLAN_SCHEMA_V1}` or omit it"),
            );
        }
    }
    if let Some(version) = non_empty(plan.version.as_deref()) {
        if version != "1" {
            push_error(
                errors,
                ValidationCategory::SchemaMismatch,
                "version",
                "plan declares an unsupported version".to_owned(),
                "set `version` to `1`".to_owned(),
            );
        }
    }
}

fn validate_secrets(plan: &TransactionPlan, errors: &mut Vec<ValidationIssue>) {
    for (field, value) in [
        ("secret", plan.secret.as_deref()),
        ("private_key", plan.private_key.as_deref()),
    ] {
        if non_empty(value).is_some() {
            push_error(
                errors,
                ValidationCategory::SensitiveInputRejected,
                field,
                format!("transaction plans must not embed `{field}` ([REDACTED])"),
                "remove the value and supply signing material only at submission time".to_owned(),
            );
        }
    }
}

fn validate_network(
    network: Option<&str>,
    passphrase: Option<&str>,
    expected: Option<&str>,
    field: &str,
    errors: &mut Vec<ValidationIssue>,
) {
    let Some(network) = non_empty(network) else {
        push_error(
            errors,
            ValidationCategory::MissingRequiredField,
            field,
            "plan has no target network".to_owned(),
            "set `network` to `testnet`, `pubnet`, or `futurenet`".to_owned(),
        );
        return;
    };

    let canonical = canonical_network(network);
    let expected_canonical = non_empty(expected).map(canonical_network);
    if let Some(expected) = &expected_canonical {
        if &canonical != expected {
            push_error(
                errors,
                ValidationCategory::NetworkMismatch,
                field,
                format!("expected `{expected}`, plan declares `{canonical}`"),
                format!("pass `--expected-network {expected}` or update the plan"),
            );
        }
    }

    if let Some(passphrase) = non_empty(passphrase) {
        let target = expected_canonical.as_deref().unwrap_or(&canonical);
        if let Some(canonical_passphrase) = passphrase_for_network(target) {
            if passphrase != canonical_passphrase {
                push_error(
                    errors,
                    ValidationCategory::NetworkMismatch,
                    &format!("{field}_passphrase"),
                    format!("passphrase does not match network `{target}`"),
                    format!("use `{canonical_passphrase}` for `{target}`"),
                );
            }
        } else if let Some(passphrase_network) = network_for_passphrase(passphrase) {
            if passphrase_network != target {
                push_error(
                    errors,
                    ValidationCategory::NetworkMismatch,
                    &format!("{field}_passphrase"),
                    format!("passphrase belongs to `{passphrase_network}`, not `{target}`"),
                    format!("use the `{target}` passphrase or change `network`"),
                );
            }
        }
    }
}

fn validate_source(source: Option<&str>, field: &str, errors: &mut Vec<ValidationIssue>) {
    let Some(source) = non_empty(source) else {
        push_error(
            errors,
            ValidationCategory::MissingRequiredField,
            field,
            "plan has no source account".to_owned(),
            "set `source` to a public `G...` Stellar account strkey".to_owned(),
        );
        return;
    };

    if looks_like_secret_key(source) {
        push_error(
            errors,
            ValidationCategory::SensitiveInputRejected,
            field,
            "source contains a secret seed ([REDACTED])".to_owned(),
            "use a public `G...` account strkey instead".to_owned(),
        );
    } else if ed25519::PublicKey::from_string(source).is_err() {
        push_error(
            errors,
            ValidationCategory::MalformedArgument,
            field,
            "source is not a valid public account strkey".to_owned(),
            "provide a 56-character `G...` Stellar account strkey".to_owned(),
        );
    }
}

fn validate_contract(contract: Option<&str>, field: &str, errors: &mut Vec<ValidationIssue>) {
    let Some(contract) = non_empty(contract) else {
        push_error(
            errors,
            ValidationCategory::MissingRequiredField,
            field,
            "plan has no contract identifier".to_owned(),
            "set `contract` to a `C...` strkey or a known Soroban Forge alias".to_owned(),
        );
        return;
    };

    if matches!(Strkey::from_string(contract), Ok(Strkey::Contract(_)))
        || known_alias(contract).is_some()
    {
        return;
    }

    push_error(
        errors,
        ValidationCategory::InvalidContractId,
        field,
        "contract is neither a valid `C...` strkey nor a known workspace alias".to_owned(),
        "use a 56-character `C...` contract strkey or an alias such as `escrow`".to_owned(),
    );
}

fn validate_method(
    method: Option<&str>,
    alias: Option<&str>,
    supported: &[String],
    field: &str,
    errors: &mut Vec<ValidationIssue>,
) {
    let Some(method) = non_empty(method) else {
        push_error(
            errors,
            ValidationCategory::MissingRequiredField,
            field,
            "plan has no contract method".to_owned(),
            "set `method` to the function the plan invokes".to_owned(),
        );
        return;
    };

    if !is_valid_method_name(method) {
        push_error(
            errors,
            ValidationCategory::UnsupportedMethod,
            field,
            "method is not a valid Soroban function name".to_owned(),
            "use 1..=32 lowercase ASCII letters, digits, or `_`".to_owned(),
        );
        return;
    }

    if !supported.is_empty() && !supported.iter().any(|name| name.as_str() == method) {
        push_error(
            errors,
            ValidationCategory::UnsupportedMethod,
            field,
            "method is not in the plan's supported method list".to_owned(),
            format!("choose one of: {}", supported.join(", ")),
        );
    } else if alias.is_some() && !is_known_method(method) {
        push_error(
            errors,
            ValidationCategory::UnsupportedMethod,
            field,
            format!(
                "method is not a known method of the `{}` contract family",
                alias.unwrap_or("selected")
            ),
            "check the contract spec before running this plan".to_owned(),
        );
    }
}

fn validate_arguments(
    raw_args: &[RawArg],
    field: &str,
    errors: &mut Vec<ValidationIssue>,
    encoded: &mut Vec<EncodedArgument>,
) {
    for (index, raw) in raw_args.iter().enumerate() {
        let argument_field = format!("{field}[{index}]");
        let Some(argument) = normalize_arg(raw, &argument_field, errors) else {
            continue;
        };
        match encode_scval(&argument.kind, &argument.raw_value) {
            Ok(value) => encoded.push(EncodedArgument {
                index,
                name: argument.name,
                kind: argument.kind,
                scval_variant: scval_variant(&value).to_owned(),
            }),
            Err(reason) => push_error(
                errors,
                ValidationCategory::MalformedArgument,
                &argument_field,
                format!("could not encode argument as `{}`: {reason}", argument.kind),
                hint_for_kind(&argument.kind),
            ),
        }
    }
}

fn normalize_arg(
    raw: &RawArg,
    field: &str,
    errors: &mut Vec<ValidationIssue>,
) -> Option<NormalizedArg> {
    match raw {
        RawArg::Spec(spec) => normalize_spec(spec, field, errors),
        RawArg::Structured(structured) => {
            let name = non_empty(structured.name.as_deref()).map(str::to_owned);
            if name.as_deref().is_some_and(is_sensitive_name) {
                push_error(
                    errors,
                    ValidationCategory::SensitiveInputRejected,
                    field,
                    "argument name indicates secret material ([REDACTED])".to_owned(),
                    "rename the argument and pass secrets only at signing time".to_owned(),
                );
                return None;
            }

            let Some(kind) = non_empty(structured.kind.as_deref()).map(canonical_kind) else {
                push_error(
                    errors,
                    ValidationCategory::MissingRequiredField,
                    field,
                    "structured argument has no `type`".to_owned(),
                    "set `type` to a supported ScVal type such as `u64` or `address`".to_owned(),
                );
                return None;
            };

            let raw_value = match &structured.value {
                Some(serde_json::Value::String(value)) => value.clone(),
                Some(serde_json::Value::Number(value)) => value.to_string(),
                Some(serde_json::Value::Bool(value)) => value.to_string(),
                None | Some(serde_json::Value::Null) if kind == "void" => String::new(),
                None | Some(serde_json::Value::Null) => {
                    push_error(
                        errors,
                        ValidationCategory::MissingRequiredField,
                        field,
                        "structured argument has no `value`".to_owned(),
                        hint_for_kind(&kind),
                    );
                    return None;
                }
                Some(_) => {
                    push_error(
                        errors,
                        ValidationCategory::MalformedArgument,
                        field,
                        "structured argument value must be a scalar".to_owned(),
                        "use a string, number, boolean, or null for `void`".to_owned(),
                    );
                    return None;
                }
            };

            if looks_like_secret_key(&raw_value) {
                push_error(
                    errors,
                    ValidationCategory::SensitiveInputRejected,
                    field,
                    "argument contains a secret seed ([REDACTED])".to_owned(),
                    "never include secret seeds in transaction plans".to_owned(),
                );
                return None;
            }

            Some(NormalizedArg {
                name,
                kind,
                raw_value,
            })
        }
    }
}

fn normalize_spec(
    spec: &str,
    field: &str,
    errors: &mut Vec<ValidationIssue>,
) -> Option<NormalizedArg> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        push_error(
            errors,
            ValidationCategory::MissingRequiredField,
            field,
            "argument specification is empty".to_owned(),
            "use `name=type:value`, for example `amount=i128:500`".to_owned(),
        );
        return None;
    }
    if looks_like_secret_key(trimmed) {
        push_error(
            errors,
            ValidationCategory::SensitiveInputRejected,
            field,
            "argument contains a secret seed ([REDACTED])".to_owned(),
            "never include secret seeds in transaction plans".to_owned(),
        );
        return None;
    }

    let (name, rhs) = match trimmed.split_once('=') {
        Some((name, rhs)) if !name.trim().is_empty() => (Some(name.trim().to_owned()), rhs.trim()),
        Some(_) => {
            push_error(
                errors,
                ValidationCategory::MalformedArgument,
                field,
                "argument name before `=` is empty".to_owned(),
                "use `name=type:value`".to_owned(),
            );
            return None;
        }
        None => (None, trimmed),
    };

    if name.as_deref().is_some_and(is_sensitive_name) {
        push_error(
            errors,
            ValidationCategory::SensitiveInputRejected,
            field,
            "argument name indicates secret material ([REDACTED])".to_owned(),
            "rename the argument and pass secrets only at signing time".to_owned(),
        );
        return None;
    }

    let Some((kind, raw_value)) = rhs.split_once(':') else {
        push_error(
            errors,
            ValidationCategory::MalformedArgument,
            field,
            "argument is missing an explicit ScVal type".to_owned(),
            "use `name=type:value`, for example `amount=i128:500`".to_owned(),
        );
        return None;
    };

    let kind = canonical_kind(kind);
    let raw_value = raw_value.trim().to_owned();
    if raw_value.is_empty() && !matches!(kind.as_str(), "void" | "string" | "bytes") {
        push_error(
            errors,
            ValidationCategory::MissingRequiredField,
            field,
            format!("argument of type `{kind}` has an empty value"),
            hint_for_kind(&kind),
        );
        return None;
    }
    if looks_like_secret_key(&raw_value) {
        push_error(
            errors,
            ValidationCategory::SensitiveInputRejected,
            field,
            "argument contains a secret seed ([REDACTED])".to_owned(),
            "never include secret seeds in transaction plans".to_owned(),
        );
        return None;
    }

    Some(NormalizedArg {
        name,
        kind,
        raw_value,
    })
}

fn canonical_kind(kind: &str) -> String {
    match kind.trim().to_ascii_lowercase().as_str() {
        "unit" => "void".to_owned(),
        "sym" => "symbol".to_owned(),
        "str" => "string".to_owned(),
        "hex" => "bytes".to_owned(),
        "addr" => "address".to_owned(),
        other => other.to_owned(),
    }
}

fn encode_scval(kind: &str, raw_value: &str) -> std::result::Result<ScVal, String> {
    match kind {
        "void" => match raw_value {
            "" | "null" | "()" => Ok(ScVal::Void),
            _ => Err("`void` accepts only an empty value, `null`, or `()`".to_owned()),
        },
        "bool" => match raw_value {
            "true" => Ok(ScVal::Bool(true)),
            "false" => Ok(ScVal::Bool(false)),
            _ => Err("expected `true` or `false`".to_owned()),
        },
        "u32" => raw_value
            .parse::<u32>()
            .map(ScVal::U32)
            .map_err(|_| "expected an unsigned 32-bit integer".to_owned()),
        "i32" => raw_value
            .parse::<i32>()
            .map(ScVal::I32)
            .map_err(|_| "expected a signed 32-bit integer".to_owned()),
        "u64" => raw_value
            .parse::<u64>()
            .map(ScVal::U64)
            .map_err(|_| "expected an unsigned 64-bit integer".to_owned()),
        "i64" => raw_value
            .parse::<i64>()
            .map(ScVal::I64)
            .map_err(|_| "expected a signed 64-bit integer".to_owned()),
        "u128" => {
            let parsed = raw_value
                .parse::<u128>()
                .map_err(|_| "expected an unsigned 128-bit integer".to_owned())?;
            Ok(ScVal::U128(UInt128Parts {
                hi: (parsed >> 64) as u64,
                lo: parsed as u64,
            }))
        }
        "i128" => {
            let parsed = raw_value
                .parse::<i128>()
                .map_err(|_| "expected a signed 128-bit integer".to_owned())?;
            Ok(ScVal::I128(Int128Parts {
                hi: (parsed >> 64) as i64,
                lo: parsed as u64,
            }))
        }
        "symbol" => {
            let symbol: ScSymbol = raw_value
                .try_into()
                .map_err(|_| "expected 1..=32 ASCII alphanumeric or `_` characters".to_owned())?;
            Ok(ScVal::Symbol(symbol))
        }
        "string" => {
            let string: StringM = raw_value
                .try_into()
                .map_err(|_| "value exceeds the maximum ScVal string length".to_owned())?;
            Ok(ScVal::String(ScString(string)))
        }
        "bytes" => {
            let clean = raw_value.strip_prefix("0x").unwrap_or(raw_value);
            if clean.len() % 2 != 0 || !clean.chars().all(|ch| ch.is_ascii_hexdigit()) {
                return Err(
                    "expected an even number of hex digits, optionally prefixed with `0x`"
                        .to_owned(),
                );
            }
            let decoded = clean
                .as_bytes()
                .chunks(2)
                .filter_map(|chunk| std::str::from_utf8(chunk).ok())
                .map(|pair| u8::from_str_radix(pair, 16))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|_| "invalid hex byte string".to_owned())?;
            let bytes: ScBytes = decoded
                .try_into()
                .map_err(|_| "value exceeds the maximum ScVal byte length".to_owned())?;
            Ok(ScVal::Bytes(bytes))
        }
        "address" => match Strkey::from_string(raw_value) {
            Ok(Strkey::PublicKeyEd25519(public_key)) => Ok(ScVal::Address(ScAddress::Account(
                AccountId(PublicKey::PublicKeyTypeEd25519(Uint256(public_key.0))),
            ))),
            Ok(Strkey::Contract(contract)) => {
                Ok(ScVal::Address(ScAddress::Contract(Hash(contract.0))))
            }
            _ => Err("expected a `G...` account or `C...` contract strkey".to_owned()),
        },
        other => Err(format!("unsupported ScVal type `{other}`")),
    }
}

fn scval_variant(value: &ScVal) -> &'static str {
    match value {
        ScVal::Void => "void",
        ScVal::Bool(_) => "bool",
        ScVal::U32(_) => "u32",
        ScVal::I32(_) => "i32",
        ScVal::U64(_) => "u64",
        ScVal::I64(_) => "i64",
        ScVal::U128(_) => "u128",
        ScVal::I128(_) => "i128",
        ScVal::Symbol(_) => "symbol",
        ScVal::String(_) => "string",
        ScVal::Bytes(_) => "bytes",
        ScVal::Address(_) => "address",
        _ => "other",
    }
}

fn hint_for_kind(kind: &str) -> String {
    match kind {
        "bool" => "use `true` or `false`".to_owned(),
        "u32" | "i32" | "u64" | "i64" | "u128" | "i128" => {
            "use a decimal integer within the declared width".to_owned()
        }
        "address" => "use a valid `G...` account or `C...` contract strkey".to_owned(),
        "bytes" => "use an even-length hex string, optionally prefixed with `0x`".to_owned(),
        "symbol" => "use 1..=32 lowercase ASCII letters, digits, or `_`".to_owned(),
        "string" => "use a UTF-8 string".to_owned(),
        "void" => "use an empty value, `null`, or `()`".to_owned(),
        other => format!("use a valid value for `{other}`"),
    }
}

fn canonical_network(network: &str) -> String {
    let lower = network.trim().to_ascii_lowercase();
    match lower.as_str() {
        "public" | "mainnet" | "pubnet" => "pubnet".to_owned(),
        "test" | "testnet" => "testnet".to_owned(),
        "future" | "futurenet" => "futurenet".to_owned(),
        "local" | "standalone" => "standalone".to_owned(),
        _ => lower,
    }
}

fn passphrase_for_network(network: &str) -> Option<&'static str> {
    match network {
        "pubnet" => Some("Public Global Stellar Network ; September 2015"),
        "testnet" => Some("Test SDF Network ; September 2015"),
        "futurenet" => Some("Test SDF Future Network ; October 2022"),
        "standalone" => Some("Standalone Network ; February 2017"),
        _ => None,
    }
}

fn network_for_passphrase(passphrase: &str) -> Option<&'static str> {
    match passphrase {
        "Public Global Stellar Network ; September 2015" => Some("pubnet"),
        "Test SDF Network ; September 2015" => Some("testnet"),
        "Test SDF Future Network ; October 2022" => Some("futurenet"),
        "Standalone Network ; February 2017" => Some("standalone"),
        _ => None,
    }
}

fn known_alias(raw: &str) -> Option<&'static str> {
    let normalized = raw.trim().to_ascii_lowercase().replace('_', "-");
    match normalized.as_str() {
        "escrow" => Some("escrow"),
        "vesting" => Some("vesting"),
        "multi-sig-wallet" | "multisig" => Some("multi-sig-wallet"),
        "dao-governance" | "dao" => Some("dao-governance"),
        "subscription-payments" | "subscription" => Some("subscription-payments"),
        "marketplace-royalties" | "marketplace" => Some("marketplace-royalties"),
        _ => None,
    }
}

fn is_valid_method_name(method: &str) -> bool {
    let bytes = method.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 32
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
}

fn is_known_method(method: &str) -> bool {
    const KNOWN_METHODS: &[&str] = &[
        "approve",
        "authorize_provider",
        "balance",
        "buy",
        "cancel",
        "cancel_listing",
        "cancel_proposal",
        "charge",
        "charge_catchup",
        "check_withdrawal",
        "claim",
        "claimable",
        "configure_bond",
        "confirm",
        "create_escrow",
        "create_listing",
        "create_plan",
        "create_proposal",
        "create_schedule",
        "create_tranche_schedule",
        "deposit",
        "dispute",
        "distribute",
        "escrows_for_participant",
        "execute",
        "finalize",
        "get_bond_config",
        "get_escrow",
        "get_listing",
        "get_owners",
        "get_proposal",
        "get_proposal_count",
        "get_royalty",
        "get_schedule",
        "get_settlement_summary",
        "get_status",
        "get_subscription",
        "get_threshold",
        "get_tranche_schedule",
        "get_transaction",
        "get_tx",
        "get_tx_count",
        "get_withdrawal_limit",
        "has_voted",
        "initialize",
        "is_owner",
        "is_provider_authorized",
        "pause",
        "propose",
        "refund",
        "reject",
        "release",
        "release_partial",
        "remove_withdrawal_limit",
        "resolve",
        "resume",
        "revoke",
        "revoke_provider",
        "set_royalty",
        "set_withdrawal_limit",
        "settle_sale",
        "settle_sales",
        "submit",
        "submit_call",
        "submit_transaction",
        "submit_withdrawal",
        "subscribe",
        "subscribe_on_behalf_of",
        "touch_ttl",
        "vote",
    ];
    KNOWN_METHODS.contains(&method)
}

fn is_sensitive_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        "secret",
        "private",
        "password",
        "mnemonic",
        "seed",
        "passphrase",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
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

fn report_from_issue(
    category: ValidationCategory,
    field: &str,
    message: String,
    hint: String,
) -> ValidationReport {
    finalize_report(
        None,
        None,
        None,
        Vec::new(),
        vec![ValidationIssue {
            category,
            field: field.to_owned(),
            message,
            hint,
        }],
    )
}

fn finalize_report(
    network: Option<&str>,
    contract: Option<&str>,
    method: Option<&str>,
    encoded_args: Vec<EncodedArgument>,
    errors: Vec<ValidationIssue>,
) -> ValidationReport {
    let mut seen = BTreeSet::new();
    let mut errors: Vec<ValidationIssue> = errors
        .into_iter()
        .filter(|issue| {
            seen.insert((
                issue.category.as_str(),
                issue.field.clone(),
                issue.message.clone(),
            ))
        })
        .collect();
    errors.sort_by(|left, right| {
        left.category
            .as_str()
            .cmp(right.category.as_str())
            .then(left.field.cmp(&right.field))
            .then(left.message.cmp(&right.message))
    });
    let error_count = errors.len();
    ValidationReport {
        schema: REPORT_SCHEMA_V1.to_owned(),
        valid: error_count == 0,
        exit_code: if error_count == 0 {
            EXIT_OK
        } else {
            EXIT_INVALID_PLAN
        },
        network: network.map(str::to_owned),
        contract: contract.map(str::to_owned),
        method: method.map(str::to_owned),
        encoded_args,
        error_count,
        errors,
    }
}

fn render_json(report: &ValidationReport) -> Result<String> {
    serde_json::to_string_pretty(report).context("failed to serialize validation report")
}

fn render_text(report: &ValidationReport) -> String {
    let status = if report.valid { "VALID" } else { "INVALID" };
    let mut output = String::new();
    let _ = writeln!(
        output,
        "Transaction plan: {status} (exit_code={})",
        report.exit_code
    );
    let _ = writeln!(
        output,
        "  network : {}",
        report.network.as_deref().unwrap_or("<missing>")
    );
    let _ = writeln!(
        output,
        "  contract: {}",
        report.contract.as_deref().unwrap_or("<missing>")
    );
    let _ = writeln!(
        output,
        "  method  : {}",
        report.method.as_deref().unwrap_or("<missing>")
    );
    let _ = writeln!(output, "  args    : {} encoded", report.encoded_args.len());

    if report.errors.is_empty() {
        let _ = writeln!(output, "\nResult: all transaction-plan checks passed.");
    } else {
        let _ = writeln!(output, "\nErrors ({}):", report.error_count);
        for issue in &report.errors {
            let _ = writeln!(
                output,
                "  [{}] {}: {}",
                issue.category.as_str(),
                issue.field,
                issue.message
            );
            let _ = writeln!(output, "    hint: {}", issue.hint);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> ValidatePlanArgs {
        ValidatePlanArgs::default()
    }

    #[test]
    fn valid_fixture_has_no_errors_and_encodes_arguments() {
        let plan: TransactionPlan = serde_json::from_str(include_str!(
            "../../fixtures/plans/valid-create-escrow.json"
        ))
        .unwrap();
        let report = validate_plan(&plan, &args());
        assert_eq!(report.exit_code, EXIT_OK);
        assert!(report.errors.is_empty());
        assert_eq!(report.method.as_deref(), Some("create_escrow"));
        assert!(!report.encoded_args.is_empty());
    }

    #[test]
    fn invalid_fixture_reports_independent_categories() {
        let plan: TransactionPlan = serde_json::from_str(include_str!(
            "../../fixtures/plans/invalid-multi-error.json"
        ))
        .unwrap();
        let report = validate_plan(&plan, &args());
        let categories: BTreeSet<&str> = report
            .errors
            .iter()
            .map(|issue| issue.category.as_str())
            .collect();
        for category in [
            "network_mismatch",
            "invalid_contract_id",
            "unsupported_method",
            "malformed_argument",
            "sensitive_input_rejected",
        ] {
            assert!(
                categories.contains(category),
                "missing category: {category}"
            );
        }
        assert_eq!(report.exit_code, EXIT_INVALID_PLAN);
    }

    #[test]
    fn missing_fields_and_bad_schema_are_reported_together() {
        let plan: TransactionPlan =
            serde_json::from_str(include_str!("../../fixtures/plans/missing-fields.json")).unwrap();
        let report = validate_plan(&plan, &args());
        let categories: BTreeSet<&str> = report
            .errors
            .iter()
            .map(|issue| issue.category.as_str())
            .collect();
        assert!(categories.contains("schema_mismatch"));
        assert!(categories.contains("missing_required_field"));
        assert!(categories.contains("sensitive_input_rejected"));
    }

    #[test]
    fn toml_plans_parse_with_structured_arguments() {
        let plan = parse_plan(
            include_str!("../../fixtures/plans/valid-deposit.toml"),
            true,
        )
        .unwrap();
        let report = validate_plan(&plan, &args());
        assert_eq!(report.exit_code, EXIT_OK);
        assert_eq!(report.method.as_deref(), Some("deposit"));
    }

    #[test]
    fn encode_scval_errors_do_not_echo_values() {
        let error = encode_scval("u64", "SUPER_SECRET_VALUE").unwrap_err();
        assert!(!error.contains("SUPER_SECRET_VALUE"));
    }

    #[test]
    fn json_report_uses_stable_schema_and_categories() {
        let plan: TransactionPlan = serde_json::from_str(include_str!(
            "../../fixtures/plans/invalid-multi-error.json"
        ))
        .unwrap();
        let report = validate_plan(&plan, &args());
        let json: serde_json::Value = serde_json::from_str(&render_json(&report).unwrap()).unwrap();
        assert_eq!(json["schema"], REPORT_SCHEMA_V1);
        assert_eq!(json["exit_code"], EXIT_INVALID_PLAN);
        assert_eq!(json["valid"], false);
        assert!(json["errors"]
            .as_array()
            .unwrap()
            .iter()
            .all(|error| error["category"].is_string()));
    }
}
