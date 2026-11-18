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
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn run(args: crate::cli::PlanArgs) -> Result<()> {
    let plan = load_plan(&args.plan)?;
    validate_plan(&plan)?;

    if args.dry_run {
        dry_run(&plan, &args.plan)?;
        return Ok(());
    }

    if !args.force {
        confirmation_prompt(&plan)?;
    }

    execute_plan(&plan, &args.plan, args.receipt.as_deref())?;
    Ok(())
}

fn load_plan(path: &Path) -> Result<Plan> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read plan file at {path:?}"))?;
    serde_json::from_str(&raw).with_context(|| format!("failed to parse plan file at {path:?}"))
}

fn validate_plan(plan: &Plan) -> Result<()> {
    if plan.version != "1" {
        bail!(
            "unsupported plan version `{version}`: only version `1` is supported",
            version = plan.version
        );
    }
    if plan.network.trim().is_empty() {
        bail!("plan.network must be a non-empty string");
    }
    if plan.source.trim().is_empty() {
        bail!("plan.source must be a non-empty source account");
    }
    if plan.calls.is_empty() {
        bail!("plan.calls must contain at least one call");
    }
    for (idx, call) in plan.calls.iter().enumerate() {
        validate_call(idx, call)?;
    }
    Ok(())
}

fn validate_call(idx: usize, call: &Call) -> Result<()> {
    if call.contract.trim().is_empty() {
        bail!("call[{idx}].contract must be a non-empty contract address");
    }
    if call.function.trim().is_empty() {
        bail!("call[{idx}].function must be a non-empty function name");
    }
    for (jdx, arg) in call.args.iter().enumerate() {
        if arg.contains('=') {
            let (key, value) = arg
                .split_once('=')
                .expect("split_once failed for arg with `=`");
            if key.trim().is_empty() {
                bail!("call[{idx}].args[{jdx}] has an empty key");
            }
        } else {
            bail!("call[{idx}].args[{jdx}] must be in `key=value` form");
        }
    }
    Ok(())
}

fn dry_run(plan: &Plan, path: &Path) -> Result<()> {
    let report = DryRunReport::from_plan(plan);
    let serialized = serde_json::to_string_pretty(&report)?;
    fs::write(path.with_extension("dryrun.json"), serialized).with_context(|| {
        format!(
            "failed to write dry-run report at {:?}",
            path.with_extension("dryrun.json")
        )
    })?;
    println!("dry-run report written to {path:?}.dryrun.json");
    Ok(())
}

fn confirmation_prompt(plan: &Plan) -> Result<()> {
    let call_summary: Vec<String> = plan
        .calls
        .iter()
        .enumerate()
        .map(|(idx, call)| format!("  {idx}: {}({})", call.function, call.contract))
        .collect();
    println!("Transaction plan preview:");
    println!("  network: {}", plan.network);
    println!("  source:  {}", plan.source);
    println!("  calls:   {}", plan.calls.len());
    for line in &call_summary {
        println!("{line}");
    }
    println!("This will execute the plan through the existing CLI transport.");
    println!("Type `yes` to continue: ");
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .context("failed to read confirmation from stdin")?;
    if answer.trim().to_ascii_lowercase() != "yes" {
        bail!("plan execution aborted by user");
    }
    Ok(())
}

fn execute_plan(plan: &Plan, plan_path: &Path, receipt_path: Option<&Path>) -> Result<()> {
    let receipt_path = receipt_path
        .map(PathBuf::from)
        .unwrap_or_else(|| plan_path.with_extension("receipt.json"));
    let mut receipt = PlanReceipt {
        plan_file: plan_path.to_string_lossy().into_owned(),
        network: plan.network.clone(),
        started_at: chrono_now_iso(),
        steps: Vec::new(),
        completed_transaction_hashes: Vec::new(),
        errors: Vec::new(),
    };

    for (idx, call) in plan.calls.iter().enumerate() {
        let step_result = execute_call(idx, call, &plan.source, &plan.network);
        let step = PlanStep {
            step_index: idx,
            contract: call.contract.clone(),
            function: call.function.clone(),
            args: call.args.clone(),
            status: step_result.status.clone(),
            detail: step_result.detail.clone(),
            transaction_hash: step_result.transaction_hash.clone(),
        };
        receipt.steps.push(step);
        if let Some(hash) = &step_result.transaction_hash {
            receipt.completed_transaction_hashes.push(hash.clone());
        }
        if let Err(ref detail) = step_result.status {
            receipt.errors.push(format!("step {idx}: {detail}"));
        }
        if step_result.status.is_err() {
            let _ = fs::write(&receipt_path, serde_json::to_string_pretty(&receipt)?);
            let error_message = step_result.status.as_ref().unwrap_err().clone();
            bail!(
                "plan failed at step {idx}: {detail}",
                idx = idx,
                detail = error_message,
            );
        }
        if let Some(assertion) = &call.assertion {
            evaluate_assertion(&assertion, &step, &receipt_path)?;
        }
    }

    receipt.succeeded = true;
    receipt.finished_at = Some(chrono_now_iso());
    fs::write(&receipt_path, serde_json::to_string_pretty(&receipt)?)
        .with_context(|| format!("failed to write receipt at {:?}", receipt_path))?;
    println!("plan completed; receipt written to {receipt_path:?}");
    Ok(())
}

fn execute_call(idx: usize, call: &Call, source: &str, network: &str) -> StepResult {
    match call.kind.as_deref().unwrap_or("invoke") {
        "invoke" => execute_invoke(idx, call, source, network),
        "deploy" => execute_deploy(idx, call, source, network),
        other => {
            let msg = format!("unsupported call kind `{other}`");
            StepResult {
                status: Err(msg.clone()),
                detail: None,
                transaction_hash: None,
            }
        }
    }
}

fn execute_invoke(idx: usize, call: &Call, source: &str, network: &str) -> StepResult {
    let mut cmd = Command::new("stellar");
    cmd.arg("contract")
        .arg("invoke")
        .arg("--id")
        .arg(&call.contract);
    if !source.is_empty() {
        cmd.arg("--source-account").arg(source);
    }
    cmd.arg("--network").arg(network);
    cmd.arg("--").arg(&call.function);
    for arg in &call.args {
        let (key, value) = arg
            .split_once('=')
            .expect("args are validated as key=value");
        cmd.arg(format!("--{key}")).arg(value);
    }
    match cmd.output() {
        Ok(output) => {
            let ok = output.status.success();
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            let combined: String = vec![stdout, stderr]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            if ok {
                StepResult {
                    status: Ok(()),
                    detail: Some(combined),
                    transaction_hash: first_transaction_hash(&stdout),
                }
            } else {
                StepResult {
                    status: Err(combined),
                    detail: Some(combined),
                    transaction_hash: None,
                }
            }
        }
        Err(error) => {
            let msg = error.to_string();
            StepResult {
                status: Err(msg.clone()),
                detail: None,
                transaction_hash: None,
            }
        }
    }
}

fn execute_deploy(idx: usize, call: &Call, source: &str, network: &str) -> StepResult {
    let wasm = call
        .args
        .iter()
        .find_map(|arg| {
            let (key, value) = arg
                .split_once('=')
                .expect("args are validated as key=value");
            if key == "wasm" {
                Some(value)
            } else {
                None
            }
        })
        .ok_or_else(|| "deploy call must include wasm=WASM_PATH".to_string())?;
    let mut cmd = Command::new("stellar");
    cmd.arg("contract").arg("deploy").arg("--wasm").arg(wasm);
    if !source.is_empty() {
        cmd.arg("--source-account").arg(source);
    }
    cmd.arg("--network").arg(network);
    match cmd.output() {
        Ok(output) => {
            let ok = output.status.success();
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            let combined: String = vec![stdout, stderr]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            if ok {
                StepResult {
                    status: Ok(()),
                    detail: Some(combined),
                    transaction_hash: first_transaction_hash(&stdout),
                }
            } else {
                StepResult {
                    status: Err(combined),
                    detail: Some(combined),
                    transaction_hash: None,
                }
            }
        }
        Err(error) => {
            let msg = error.to_string();
            StepResult {
                status: Err(msg.clone()),
                detail: None,
                transaction_hash: None,
            }
        }
    }
}

fn first_transaction_hash(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let trimmed = line.trim();
        let lowered = trimmed.to_ascii_lowercase();
        if lowered.starts_with("transaction:") {
            return Some(lowered["transaction:".len()..].trim().to_string());
        }
        let segments: Vec<&str> = trimmed.split_whitespace().collect();
        if segments.len() >= 3 {
            let candidate = segments[segments.len() - 1];
            if candidate.chars().all(|c| c.is_ascii_hexdigit()) && candidate.len() == 64 {
                return Some(candidate.to_ascii_lowercase());
            }
        }
        None
    })
}

fn evaluate_assertion(assertion: &Assertion, step: &PlanStep, _receipt_path: &Path) -> Result<()> {
    match assertion.kind.as_deref().unwrap_or("result") {
        "result" => {
            let found = step
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains(&assertion.expect);
            if !found {
                bail!(
                    "step {step_index} assertion failed: expected output to contain {expect:?}",
                    step_index = step.step_index,
                    expect = assertion.expect
                );
            }
        }
        "event" => {
            let found = step
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains(&assertion.expect);
            if !found {
                bail!(
                    "step {step_index} event assertion failed: expected event data to contain {expect:?}",
                    step_index = step.step_index,
                    expect = assertion.expect
                );
            }
        }
        other => bail!("unsupported assertion kind `{other}`"),
    }
    Ok(())
}

fn chrono_now_iso() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = now.as_secs();
    let days = total_secs / 86_400;
    let remaining = total_secs % 86_400;
    let hours = remaining / 3_600;
    let minutes = (remaining % 3_600) / 60;
    let seconds = remaining % 60;
    let (y, m, d) = days_to_ymd(days as i64);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y, m, d, hours, minutes, seconds
    )
}

fn days_to_ymd(days: i64) -> (i64, u32, u32) {
    let total = days + 719_468;
    let year = if total >= 0 {
        (total as f64 / 365.2425).floor() as i64
    } else {
        (total as f64 / 365.2425).ceil() as i64 - 1
    };
    let mut year = year.max(1);
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        let next_start = cumulative_days_before_year(year + 1);
        if total < next_start {
            break;
        }
        year += 1;
    }
    let start = cumulative_days_before_year(year);
    let doy = (total - start) as u32;
    let month = month_from_doy(year, doy);
    let day = doy - cumulative_days_before_month(year, month) + 1;
    (year, month, day)
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0) && (year % 100 != 0 || year % 400 == 0)
}

fn cumulative_days_before_year(year: i64) -> i64 {
    let y = year - 1;
    y * 365 + y / 4 - y / 100 + y / 400
}

fn month_from_doy(year: i64, doy: u32) -> u32 {
    let cumulative: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let leap_adj = if is_leap(year) && doy >= 59 { 1 } else { 0 };
    let mut month = 0;
    for (i, &c) in cumulative.iter().enumerate() {
        if doy >= c + leap_adj {
            month = i as u32;
        }
    }
    if month >= 11 {
        11
    } else {
        month + 1
    }
}

fn cumulative_days_before_month(year: i64, month: u32) -> u32 {
    let cumulative: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let leap_adj = if is_leap(year) && month > 2 { 1 } else { 0 };
    cumulative[(month - 1) as usize] + leap_adj
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Plan {
    pub version: String,
    pub network: String,
    pub source: String,
    pub calls: Vec<Call>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Call {
    #[serde(default)]
    pub kind: Option<String>,
    pub contract: String,
    pub function: String,
    pub args: Vec<String>,
    pub assertion: Option<Assertion>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Assertion {
    #[serde(default = "default_assertion_kind")]
    pub kind: String,
    pub expect: String,
}

fn default_assertion_kind() -> String {
    "result".to_string()
}

#[derive(Debug, Serialize, Clone)]
pub struct PlanReceipt {
    pub plan_file: String,
    pub network: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub succeeded: bool,
    pub steps: Vec<PlanStep>,
    pub completed_transaction_hashes: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct PlanStep {
    pub step_index: usize,
    pub contract: String,
    pub function: String,
    pub args: Vec<String>,
    pub status: ResultStatus,
    pub detail: Option<String>,
    pub transaction_hash: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ResultStatus {
    pub ok: bool,
}

impl From<anyhow::Result<()>> for ResultStatus {
    fn from(result: anyhow::Result<()>) -> Self {
        match result {
            Ok(()) => ResultStatus { ok: true },
            Err(_) => ResultStatus { ok: false },
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct DryRunReport {
    pub version: String,
    pub network: String,
    pub source: String,
    pub calls: Vec<DryRunCall>,
    pub authorization_summary: Vec<DryRunAuthorization>,
}

#[derive(Debug, Serialize, Clone)]
pub struct DryRunCall {
    pub step_index: usize,
    pub kind: String,
    pub contract: String,
    pub function: String,
    pub args: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct DryRunAuthorization {
    pub call_index: usize,
    pub note: String,
}

impl DryRunReport {
    fn from_plan(plan: &Plan) -> Self {
        let calls = plan
            .calls
            .iter()
            .enumerate()
            .map(|(idx, call)| DryRunCall {
                step_index: idx,
                kind: call.kind.as_deref().unwrap_or("invoke").to_string(),
                contract: call.contract.clone(),
                function: call.function.clone(),
                args: call.args.clone(),
            })
            .collect();
        let authorization_summary = plan
            .calls
            .iter()
            .enumerate()
            .map(|(idx, call)| DryRunAuthorization {
                call_index: idx,
                note: authorization_note(call),
            })
            .collect();
        DryRunReport {
            version: plan.version.clone(),
            network: plan.network.clone(),
            source: plan.source.clone(),
            calls,
            authorization_summary,
        }
    }
}

fn authorization_note(call: &Call) -> String {
    if call.function == "create_escrow" {
        return "buyer authorizes token transfer and contract invocation".to_string();
    }
    if matches!(
        call.function.as_str(),
        "deposit" | "release" | "refund" | "dispute" | "resolve" | "cancel"
    ) || call.function.starts_with("release_partial")
    {
        return "caller authorizes the nested SEP-41 token transfer and contract call".to_string();
    }
    "caller authorizes the contract invocation".to_string()
}
