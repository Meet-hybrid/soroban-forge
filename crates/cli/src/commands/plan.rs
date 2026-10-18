use crate::cli::ValidatePlanArgs;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use stellar_strkey::{ed25519, Contract, Strkey};
use stellar_xdr::curr::{
    Int128Parts, ScAddress, ScBytes, ScString, ScSymbol, ScVal, StringM, UInt128Parts,
};

pub const PLAN_SCHEMA_V1: &str = "soroban-forge/transaction-plan@1";
pub const REPORT_SCHEMA_V1: &str = "soroban-forge/tx-plan-validation-report@1";

pub const EXIT_OK: i32 = 0;
pub const EXIT_INVALID_PLAN: i32 = 1;

/// Machine-readable validation error categories.
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

/// A single actionable validation finding in a transaction plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub category: ValidationCategory,
    pub field: String,
    pub message: String,
    pub hint: String,
}

/// Summary of a validated argument and its target Soroban `ScVal` encoding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncodedArgumentSummary {
    pub index: usize,
    pub name: Option<String>,
    pub kind: String,
    pub scval_variant: String,
}

/// Stable validation report rendered as text or JSON.
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

/// Raw transaction plan parsed from JSON, TOML, or CLI flags.
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

/// Flexible argument representation supporting structured objects or shorthand specs.
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

struct KnownMethodSpec {
    name: &'static str,
    required_args: &'static [(&'static str, &'static str)],
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
    let mut plan = if let Some(path) = path {
        load_plan_file(path)?
    } else {
        TransactionPlanInput::default()
    };

    if let Some(ref network) = args.network {
        plan.network = Some(network.clone());
    }
    if let Some(ref contract) = args.contract {
        plan.contract = Some(contract.clone());
    }
    if let Some(ref method) = args.method {
        plan.method = Some(method.clone());
    }
    if let Some(ref source) = args.source {
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
    if matches!(extension_hint, Some("toml"))
        || (!trimmed.starts_with('{') && !trimmed.is_empty() && extension_hint != Some("json"))
    {
        if let Ok(plan) = toml::from_str::<TransactionPlanInput>(raw) {
            return Ok(plan);
        }
    }
    serde_json::from_str::<TransactionPlanInput>(raw)
        .context("failed to parse transaction plan as JSON or TOML")
}

/// Validate a [`TransactionPlanInput`] and collect all independent findings into a
/// deterministic [`ValidationReport`].
pub fn validate_plan(
    plan: &TransactionPlanInput,
    expected_network: Option<&str>,
) -> ValidationReport {
    let mut errors = Vec::new();
    let mut encoded_args = Vec::new();

    // 1. Validate schema if explicitly provided.
    if let Some(schema) = plan
        .schema
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if schema != PLAN_SCHEMA_V1 {
            errors.push(ValidationIssue {
                category: ValidationCategory::SchemaMismatch,
                field: "schema".to_owned(),
                message: format!(
                    "unsupported transaction-plan schema `{}`",
                    sanitize_display(schema)
                ),
                hint: format!("set `schema` to `{PLAN_SCHEMA_V1}` or omit the field"),
            });
        }
    }

    // 2. Reject any explicit secret / private-key fields without echoing values.
    if plan.secret.as_deref().is_some_and(|s| !s.trim().is_empty()) {
        errors.push(ValidationIssue {
            category: ValidationCategory::SensitiveInputRejected,
            field: "secret".to_owned(),
            message: "transaction plans must not embed secret keys ([REDACTED])".to_owned(),
            hint: "remove `secret` from the plan and pass signing identities at submission time"
                .to_owned(),
        });
    }
    if plan
        .private_key
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
    {
        errors.push(ValidationIssue {
            category: ValidationCategory::SensitiveInputRejected,
            field: "private_key".to_owned(),
            message: "transaction plans must not embed private keys ([REDACTED])".to_owned(),
            hint: "remove `private_key` from the plan and use external key management for signing"
                .to_owned(),
        });
    }

    // 3. Validate source account if present.
    if let Some(source) = plan
        .source
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if looks_like_secret_key(source) {
            errors.push(ValidationIssue {
                category: ValidationCategory::SensitiveInputRejected,
                field: "source".to_owned(),
                message: "source field contains a secret seed ([REDACTED]) instead of a public key"
                    .to_owned(),
                hint: "provide a public `G...` account address in `source`, never an `S...` seed"
                    .to_owned(),
            });
        } else if ed25519::PublicKey::from_string(source).is_err() {
            errors.push(ValidationIssue {
                category: ValidationCategory::MalformedArgument,
                field: "source".to_owned(),
                message: format!(
                    "invalid Stellar source public key `{}`",
                    sanitize_display(source)
                ),
                hint: "provide a valid 56-character `G...` Stellar account strkey".to_owned(),
            });
        }
    }

    // 4. Validate network identity, passphrase, and expected-network match.
    let normalized_network = plan
        .network
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase);

    match normalized_network.as_deref() {
        None => {
            errors.push(ValidationIssue {
                category: ValidationCategory::MissingRequiredField,
                field: "network".to_owned(),
                message: "required field `network` is missing or empty".to_owned(),
                hint: "specify `network` as `testnet`, `futurenet`, `standalone`, or `mainnet`"
                    .to_owned(),
            });
        }
        Some(net) => {
            if let Some(canonical_passphrase) = expected_passphrase_for_network(net) {
                if let Some(expected_net) = expected_network
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_lowercase)
                {
                    if canonical_network_name(net) != canonical_network_name(&expected_net) {
                        errors.push(ValidationIssue {
                            category: ValidationCategory::NetworkMismatch,
                            field: "network".to_owned(),
                            message: format!(
                                "plan targets network `{net}`, which does not match expected network `{expected_net}`"
                            ),
                            hint: format!(
                                "update the plan `network` to `{expected_net}` or run against `{net}`"
                            ),
                        });
                    }
                }

                if let Some(passphrase) = plan
                    .network_passphrase
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    if passphrase != canonical_passphrase {
                        errors.push(ValidationIssue {
                            category: ValidationCategory::NetworkMismatch,
                            field: "network_passphrase".to_owned(),
                            message: format!(
                                "network passphrase does not match network `{net}`"
                            ),
                            hint: format!(
                                "use `{canonical_passphrase}` for `{net}` or omit `network_passphrase`"
                            ),
                        });
                    }
                }
            } else {
                errors.push(ValidationIssue {
                    category: ValidationCategory::NetworkMismatch,
                    field: "network".to_owned(),
                    message: format!("unsupported Stellar network `{}`", sanitize_display(net)),
                    hint: "use one of: `testnet`, `futurenet`, `standalone`, `mainnet`".to_owned(),
                });
            }
        }
    }

    // 5. Validate contract identifier.
    let contract_value = plan
        .contract
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(sanitize_display);

    let mut resolved_contract_alias: Option<&'static str> = plan
        .contract_type
        .as_deref()
        .and_then(resolve_known_contract_alias);

    match plan
        .contract
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None => {
            errors.push(ValidationIssue {
                category: ValidationCategory::MissingRequiredField,
                field: "contract".to_owned(),
                message: "required field `contract` is missing or empty".to_owned(),
                hint: "provide a 56-character `C...` contract strkey or a known Soroban Forge contract alias".to_owned(),
            });
        }
        Some(raw_contract) => {
            if looks_like_secret_key(raw_contract) {
                errors.push(ValidationIssue {
                    category: ValidationCategory::SensitiveInputRejected,
                    field: "contract".to_owned(),
                    message: "contract field contains a secret seed ([REDACTED])".to_owned(),
                    hint: "provide a public `C...` contract address, never a secret seed"
                        .to_owned(),
                });
            } else if let Some(alias) = resolve_known_contract_alias(raw_contract) {
                resolved_contract_alias = Some(alias);
            } else if Contract::from_string(raw_contract).is_err() {
                errors.push(ValidationIssue {
                    category: ValidationCategory::InvalidContractId,
                    field: "contract".to_owned(),
                    message: format!(
                        "invalid contract identifier `{}`",
                        sanitize_display(raw_contract)
                    ),
                    hint: "use a valid 56-character `C...` Stellar contract strkey or a workspace alias (`escrow`, `vesting`, `multi-sig-wallet`, `dao-governance`, `subscription-payments`, `marketplace-royalties`)".to_owned(),
                });
            }
        }
    }

    // 6. Validate method name and lookup known method specification if applicable.
    let method_value = plan
        .method
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    let mut matched_method_spec: Option<&'static KnownMethodSpec> = None;

    match method_value.as_deref() {
        None => {
            errors.push(ValidationIssue {
                category: ValidationCategory::MissingRequiredField,
                field: "method".to_owned(),
                message: "required field `method` is missing or empty".to_owned(),
                hint: "specify the contract function name to invoke (for example `create_escrow`)"
                    .to_owned(),
            });
        }
        Some(method) => {
            if !is_valid_soroban_symbol(method) {
                errors.push(ValidationIssue {
                    category: ValidationCategory::UnsupportedMethod,
                    field: "method".to_owned(),
                    message: format!(
                        "method name `{}` is not a valid Soroban symbol",
                        sanitize_display(method)
                    ),
                    hint: "method names must be 1..=32 ASCII characters (`a-z`, `A-Z`, `0-9`, `_`) and start with a letter or `_`".to_owned(),
                });
            } else if is_explicitly_unsupported_method(method) {
                errors.push(ValidationIssue {
                    category: ValidationCategory::UnsupportedMethod,
                    field: "method".to_owned(),
                    message: format!(
                        "method `{method}` is not supported for Soroban contract invocation plans"
                    ),
                    hint:
                        "choose a public exported #[contractimpl] function on the target contract"
                            .to_owned(),
                });
            } else if let Some(alias) = resolved_contract_alias {
                let specs = known_methods_for_contract(alias);
                if let Some(spec) = specs.iter().find(|spec| spec.name == method) {
                    matched_method_spec = Some(spec);
                } else {
                    let supported: Vec<&str> = specs.iter().map(|spec| spec.name).collect();
                    errors.push(ValidationIssue {
                        category: ValidationCategory::UnsupportedMethod,
                        field: "method".to_owned(),
                        message: format!(
                            "method `{method}` is not exported by the `{alias}` contract"
                        ),
                        hint: format!(
                            "supported `{alias}` methods include: {}",
                            supported.join(", ")
                        ),
                    });
                }
            }
        }
    }

    // 7. Validate each argument and encode to ScVal.
    let mut seen_arg_names = BTreeSet::new();
    let mut provided_arg_kinds = BTreeMap::new();

    for (index, raw_arg) in plan.args.iter().enumerate() {
        let field_label = format!("args[{index}]");
        let parsed = match normalize_raw_arg(raw_arg) {
            Ok(parsed) => parsed,
            Err(issue) => {
                errors.push(ValidationIssue {
                    field: field_label,
                    ..issue
                });
                continue;
            }
        };

        if let Some(ref name) = parsed.name {
            if !is_valid_soroban_symbol(name) {
                errors.push(ValidationIssue {
                    category: ValidationCategory::MalformedArgument,
                    field: format!("{field_label}.name"),
                    message: format!(
                        "argument name `{}` is not a valid identifier",
                        sanitize_display(name)
                    ),
                    hint: "use snake_case ASCII parameter names up to 32 characters".to_owned(),
                });
            } else if !seen_arg_names.insert(name.clone()) {
                errors.push(ValidationIssue {
                    category: ValidationCategory::MalformedArgument,
                    field: format!("{field_label}.name"),
                    message: format!("duplicate argument `{name}`"),
                    hint: "provide each named argument at most once".to_owned(),
                });
            }
        }

        if looks_like_secret_key(&parsed.raw_value)
            || parsed.name.as_deref().is_some_and(is_sensitive_field_name)
        {
            errors.push(ValidationIssue {
                category: ValidationCategory::SensitiveInputRejected,
                field: parsed
                    .name
                    .as_ref()
                    .map(|n| format!("args.{n}"))
                    .unwrap_or(field_label),
                message: "argument contains sensitive secret material ([REDACTED])".to_owned(),
                hint: "never pass secret seeds or private keys as contract call arguments"
                    .to_owned(),
            });
            continue;
        }

        match encode_scval(&parsed.kind, &parsed.raw_value) {
            Ok(scval) => {
                if let Some(ref name) = parsed.name {
                    provided_arg_kinds.insert(name.clone(), canonical_kind(&parsed.kind));
                }
                encoded_args.push(EncodedArgumentSummary {
                    index,
                    name: parsed.name,
                    kind: canonical_kind(&parsed.kind),
                    scval_variant: scval_variant_name(&scval).to_owned(),
                });
            }
            Err(message) => {
                let target_field = parsed
                    .name
                    .as_ref()
                    .map(|n| format!("args.{n}"))
                    .unwrap_or(field_label);
                errors.push(ValidationIssue {
                    category: ValidationCategory::MalformedArgument,
                    field: target_field,
                    message,
                    hint: hint_for_kind(&parsed.kind),
                });
            }
        }
    }

    // 8. Check required method arguments and expected types when the method spec is known
    //    and named arguments are used (or positional count is too short).
    if let Some(spec) = matched_method_spec {
        let uses_named_args = plan.args.iter().any(|arg| match arg {
            RawPlanArg::Spec(s) => s.contains('='),
            RawPlanArg::Structured(s) => s.name.is_some(),
        });

        if uses_named_args || plan.args.is_empty() {
            for (req_name, req_kind) in spec.required_args {
                if !seen_arg_names.contains(*req_name) {
                    errors.push(ValidationIssue {
                        category: ValidationCategory::MissingRequiredField,
                        field: format!("args.{req_name}"),
                        message: format!(
                            "missing required argument `{req_name}` (expected `{req_kind}`) for method `{}`",
                            spec.name
                        ),
                        hint: format!("pass `--arg {req_name}={req_kind}:<value>` or include `{req_name}` in the plan `args` list"),
                    });
                } else if let Some(actual_kind) = provided_arg_kinds.get(*req_name) {
                    if actual_kind != req_kind {
                        errors.push(ValidationIssue {
                            category: ValidationCategory::MalformedArgument,
                            field: format!("args.{req_name}"),
                            message: format!(
                                "argument `{req_name}` for method `{}` expects type `{req_kind}`, got `{actual_kind}`",
                                spec.name
                            ),
                            hint: format!("encode `{req_name}` as `{req_kind}`"),
                        });
                    }
                }
            }
        } else if plan.args.len() < spec.required_args.len() {
            for (req_name, req_kind) in &spec.required_args[plan.args.len()..] {
                errors.push(ValidationIssue {
                    category: ValidationCategory::MissingRequiredField,
                    field: format!("args.{req_name}"),
                    message: format!(
                        "missing required argument `{req_name}` (expected `{req_kind}`) for method `{}`",
                        spec.name
                    ),
                    hint: format!("provide `{req_name}` of type `{req_kind}`"),
                });
            }
        }
    }

    let valid = errors.is_empty();
    let exit_code = if valid { EXIT_OK } else { EXIT_INVALID_PLAN };

    ValidationReport {
        schema: REPORT_SCHEMA_V1.to_owned(),
        valid,
        exit_code,
        network: normalized_network,
        contract: contract_value,
        method: method_value.map(|m| sanitize_display(&m)),
        encoded_args,
        error_count: errors.len(),
        errors,
    }
}

pub fn render_json(report: &ValidationReport) -> Result<String> {
    serde_json::to_string_pretty(report).context("failed to serialize validation report as JSON")
}

pub fn render_text(report: &ValidationReport) -> String {
    let mut out = String::new();
    let status_label = if report.valid { "VALID" } else { "INVALID" };
    out.push_str(&format!(
        "Transaction Plan Validation: {status_label} (exit_code={})\n",
        report.exit_code
    ));
    out.push_str(&format!(
        "  Network : {}\n",
        report.network.as_deref().unwrap_or("<missing>")
    ));
    out.push_str(&format!(
        "  Contract: {}\n",
        report.contract.as_deref().unwrap_or("<missing>")
    ));
    out.push_str(&format!(
        "  Method  : {}\n",
        report.method.as_deref().unwrap_or("<missing>")
    ));
    out.push_str(&format!(
        "  Args    : {} encoded\n",
        report.encoded_args.len()
    ));

    if !report.encoded_args.is_empty() {
        out.push_str("\nEncoded Arguments:\n");
        for arg in &report.encoded_args {
            let label = arg.name.as_deref().unwrap_or("<positional>");
            out.push_str(&format!(
                "  [{}] {} ({} -> {})\n",
                arg.index, label, arg.kind, arg.scval_variant
            ));
        }
    }

    if report.errors.is_empty() {
        out.push_str("\nResult: all transaction-plan checks passed.\n");
    } else {
        out.push_str(&format!("\nValidation Errors ({}):\n", report.error_count));
        for (idx, issue) in report.errors.iter().enumerate() {
            out.push_str(&format!(
                "  {}. [{}] {}: {}\n     hint: {}\n",
                idx + 1,
                issue.category.as_str(),
                issue.field,
                issue.message,
                issue.hint
            ));
        }
    }

    out
}

struct NormalizedArg {
    name: Option<String>,
    kind: String,
    raw_value: String,
}

fn normalize_raw_arg(raw: &RawPlanArg) -> std::result::Result<NormalizedArg, ValidationIssue> {
    match raw {
        RawPlanArg::Spec(spec) => parse_arg_spec(spec),
        RawPlanArg::Structured(structured) => {
            let name = structured
                .name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let kind = structured
                .kind
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
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
                            name.as_ref().map(|n| format!(" `{n}`")).unwrap_or_default()
                        ),
                        hint: "provide a non-null `value` matching the declared ScVal type"
                            .to_owned(),
                    });
                }
                None | Some(serde_json::Value::Null) => String::new(),
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(serde_json::Value::Number(n)) => n.to_string(),
                Some(serde_json::Value::Bool(b)) => b.to_string(),
                Some(other) => {
                    return Err(ValidationIssue {
                        category: ValidationCategory::MalformedArgument,
                        field: "args".to_owned(),
                        message: format!(
                            "unsupported JSON value representation for argument ({})",
                            other
                        ),
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
    if raw_value.is_empty() && kind != "void" && kind != "string" && kind != "bytes" {
        return Err(ValidationIssue {
            category: ValidationCategory::MissingRequiredField,
            field: "args".to_owned(),
            message: format!(
                "argument{} of type `{kind}` has an empty value",
                name.as_ref().map(|n| format!(" `{n}`")).unwrap_or_default()
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
            if !raw_value.is_empty() && raw_value != "null" && raw_value != "()" {
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
            let parsed = raw_value.parse::<u128>().map_err(|_| {
                format!("invalid `u128` integer `{}`", sanitize_display(raw_value))
            })?;
            let hi = (parsed >> 64) as u64;
            let lo = parsed as u64;
            Ok(ScVal::U128(UInt128Parts { hi, lo }))
        }
        "i128" => {
            let parsed = raw_value.parse::<i128>().map_err(|_| {
                format!("invalid `i128` integer `{}`", sanitize_display(raw_value))
            })?;
            let hi = (parsed >> 64) as i64;
            let lo = parsed as u64;
            Ok(ScVal::I128(Int128Parts { hi, lo }))
        }
        "symbol" | "sym" => {
            if !is_valid_soroban_symbol(raw_value) {
                return Err(format!(
                    "invalid Soroban symbol `{}` (must be 1..=32 ASCII alphanumeric/`_` characters)",
                    sanitize_display(raw_value)
                ));
            }
            let sym: ScSymbol = raw_value
                .try_into()
                .map_err(|_| format!("failed to encode symbol `{}`", sanitize_display(raw_value)))?;
            Ok(ScVal::Symbol(sym))
        }
        "string" | "str" => {
            let s: StringM = raw_value.try_into().map_err(|_| {
                format!("failed to encode ScVal string `{}`", sanitize_display(raw_value))
            })?;
            Ok(ScVal::String(ScString(s)))
        }
        "bytes" | "hex" => {
            let clean = raw_value.strip_prefix("0x").unwrap_or(raw_value);
            if clean.len() % 2 != 0 || !clean.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(format!(
                    "invalid hex byte string `{}` (expected an even number of hex digits)",
                    sanitize_display(raw_value)
                ));
            }
            let mut decoded = Vec::with_capacity(clean.len() / 2);
            for chunk in clean.as_bytes().chunks(2) {
                let pair = std::str::from_utf8(chunk).unwrap_or_default();
                let byte = u8::from_str_radix(pair, 16).map_err(|_| {
                    format!("invalid hex byte `{pair}` in `{}`", sanitize_display(raw_value))
                })?;
                decoded.push(byte);
            }
            let sc_bytes: ScBytes = decoded
                .try_into()
                .map_err(|_| "byte array exceeds maximum ScBytes length".to_owned())?;
            Ok(ScVal::Bytes(sc_bytes))
        }
        "address" | "addr" => match Strkey::from_string(raw_value) {
            Ok(Strkey::PublicKeyEd25519(pk)) => Ok(ScVal::Address(ScAddress::Account(
                stellar_xdr::curr::AccountId(
                    stellar_xdr::curr::PublicKey::PublicKeyTypeEd25519(stellar_xdr::curr::Uint256(
                        pk.0,
                    )),
                ),
            ))),
            Ok(Strkey::Contract(contract)) => Ok(ScVal::Address(ScAddress::Contract(
                stellar_xdr::curr::Hash(contract.0),
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

fn scval_variant_name(val: &ScVal) -> &'static str {
    match val {
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

fn known_methods_for_contract(alias: &str) -> &'static [KnownMethodSpec] {
    match alias {
        "escrow" => &[
            KnownMethodSpec {
                name: "create_escrow",
                required_args: &[
                    ("depositor", "address"),
                    ("beneficiary", "address"),
                    ("arbiter", "address"),
                    ("token", "address"),
                    ("amount", "i128"),
                    ("timeout_duration", "u64"),
                ],
            },
            KnownMethodSpec {
                name: "deposit",
                required_args: &[("escrow_id", "u64")],
            },
            KnownMethodSpec {
                name: "release",
                required_args: &[("escrow_id", "u64")],
            },
            KnownMethodSpec {
                name: "release_partial",
                required_args: &[("escrow_id", "u64"), ("amount", "i128")],
            },
            KnownMethodSpec {
                name: "refund",
                required_args: &[("escrow_id", "u64")],
            },
            KnownMethodSpec {
                name: "dispute",
                required_args: &[("escrow_id", "u64"), ("claimant", "address")],
            },
            KnownMethodSpec {
                name: "resolve",
                required_args: &[("escrow_id", "u64"), ("release_to_beneficiary", "bool")],
            },
            KnownMethodSpec {
                name: "cancel",
                required_args: &[("escrow_id", "u64")],
            },
            KnownMethodSpec {
                name: "get_escrow",
                required_args: &[("escrow_id", "u64")],
            },
        ],
        "vesting" => &[
            KnownMethodSpec {
                name: "create_schedule",
                required_args: &[
                    ("admin", "address"),
                    ("beneficiary", "address"),
                    ("token", "address"),
                    ("total_amount", "i128"),
                    ("start_time", "u64"),
                    ("cliff_duration", "u64"),
                    ("total_duration", "u64"),
                ],
            },
            KnownMethodSpec {
                name: "claim",
                required_args: &[("schedule_id", "u64")],
            },
            KnownMethodSpec {
                name: "revoke",
                required_args: &[("schedule_id", "u64")],
            },
            KnownMethodSpec {
                name: "get_schedule",
                required_args: &[("schedule_id", "u64")],
            },
        ],
        "multi-sig-wallet" => &[
            KnownMethodSpec {
                name: "submit_transaction",
                required_args: &[
                    ("proposer", "address"),
                    ("target_contract", "address"),
                    ("function", "symbol"),
                ],
            },
            KnownMethodSpec {
                name: "approve",
                required_args: &[("signer", "address"), ("tx_id", "u64")],
            },
            KnownMethodSpec {
                name: "execute",
                required_args: &[("executor", "address"), ("tx_id", "u64")],
            },
            KnownMethodSpec {
                name: "get_transaction",
                required_args: &[("tx_id", "u64")],
            },
        ],
        "dao-governance" => &[
            KnownMethodSpec {
                name: "create_proposal",
                required_args: &[("proposer", "address"), ("title", "symbol")],
            },
            KnownMethodSpec {
                name: "vote",
                required_args: &[
                    ("voter", "address"),
                    ("proposal_id", "u64"),
                    ("support", "bool"),
                ],
            },
            KnownMethodSpec {
                name: "finalize",
                required_args: &[("proposal_id", "u64")],
            },
            KnownMethodSpec {
                name: "execute",
                required_args: &[("proposal_id", "u64")],
            },
            KnownMethodSpec {
                name: "get_proposal",
                required_args: &[("proposal_id", "u64")],
            },
        ],
        "subscription-payments" => &[
            KnownMethodSpec {
                name: "create_plan",
                required_args: &[
                    ("merchant", "address"),
                    ("token", "address"),
                    ("amount", "i128"),
                    ("interval", "u64"),
                ],
            },
            KnownMethodSpec {
                name: "subscribe",
                required_args: &[("subscriber", "address"), ("plan_id", "u64")],
            },
            KnownMethodSpec {
                name: "charge",
                required_args: &[("subscription_id", "u64")],
            },
            KnownMethodSpec {
                name: "cancel",
                required_args: &[("subscription_id", "u64")],
            },
        ],
        "marketplace-royalties" => &[
            KnownMethodSpec {
                name: "create_listing",
                required_args: &[
                    ("seller", "address"),
                    ("token", "address"),
                    ("price", "i128"),
                ],
            },
            KnownMethodSpec {
                name: "buy",
                required_args: &[("buyer", "address"), ("listing_id", "u64")],
            },
            KnownMethodSpec {
                name: "cancel_listing",
                required_args: &[("listing_id", "u64")],
            },
            KnownMethodSpec {
                name: "get_listing",
                required_args: &[("listing_id", "u64")],
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
    let first = bytes[0];
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return false;
    }
    bytes
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
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
        && trimmed.chars().all(|c| matches!(c, 'A'..='Z' | '2'..='7'))
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
            secret: None,
            private_key: None,
            args: vec![
                RawPlanArg::Spec(format!("depositor=address:{SAMPLE_ACCOUNT_A}")),
                RawPlanArg::Spec(format!("beneficiary=address:{SAMPLE_ACCOUNT_B}")),
                RawPlanArg::Spec(format!("arbiter=address:{SAMPLE_ACCOUNT_A}")),
                RawPlanArg::Spec(format!("token=address:{SAMPLE_CONTRACT_ID}")),
                RawPlanArg::Spec("amount=i128:250000".to_owned()),
                RawPlanArg::Spec("timeout_duration=u64:86400".to_owned()),
            ],
        }
    }

    #[test]
    fn valid_plan_produces_clean_report_and_zero_exit_code() {
        let plan = valid_escrow_plan();
        let report = validate_plan(&plan, Some("testnet"));

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

        let json_str = render_json(&report).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["valid"], serde_json::Value::Bool(true));
        assert_eq!(parsed["exit_code"], serde_json::Value::from(0));
        assert_eq!(parsed["error_count"], serde_json::Value::from(0));
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
            source: None,
            secret: None,
            private_key: None,
            args: vec![
                RawPlanArg::Spec("amount=i128:not_a_number".to_owned()),
                RawPlanArg::Spec("bad_arg_without_type".to_owned()),
            ],
        };

        let report = validate_plan(&plan, Some("futurenet"));
        assert!(!report.valid);
        assert_eq!(report.exit_code, EXIT_INVALID_PLAN);

        let categories: Vec<ValidationCategory> =
            report.errors.iter().map(|e| e.category).collect();
        assert!(categories.contains(&ValidationCategory::SchemaMismatch));
        assert!(categories.contains(&ValidationCategory::NetworkMismatch));
        assert!(categories.contains(&ValidationCategory::InvalidContractId));
        assert!(categories.contains(&ValidationCategory::UnsupportedMethod));
        assert!(categories.contains(&ValidationCategory::MalformedArgument));
        assert!(report.error_count >= 6);
    }

    #[test]
    fn missing_required_fields_and_method_parameters_are_detected() {
        let empty = TransactionPlanInput::default();
        let report = validate_plan(&empty, None);
        assert!(!report.valid);

        let missing_fields: Vec<&str> = report.errors.iter().map(|e| e.field.as_str()).collect();
        assert!(missing_fields.contains(&"network"));
        assert!(missing_fields.contains(&"contract"));
        assert!(missing_fields.contains(&"method"));

        // Now test missing required method inputs for a known contract method.
        let partial_escrow = TransactionPlanInput {
            network: Some("testnet".to_owned()),
            contract: Some("escrow".to_owned()),
            method: Some("release_partial".to_owned()),
            args: vec![RawPlanArg::Spec("escrow_id=u64:1".to_owned())],
            ..Default::default()
        };
        let method_report = validate_plan(&partial_escrow, None);
        assert!(!method_report.valid);
        assert!(method_report.errors.iter().any(|e| {
            e.category == ValidationCategory::MissingRequiredField && e.field == "args.amount"
        }));
    }

    #[test]
    fn sensitive_values_are_rejected_and_never_echoed_in_text_or_json() {
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
            ..Default::default()
        };

        let report = validate_plan(&plan, None);
        assert!(!report.valid);
        assert!(report
            .errors
            .iter()
            .any(|e| e.category == ValidationCategory::SensitiveInputRejected));

        let text = render_text(&report);
        let json = render_json(&report).unwrap();
        assert!(!text.contains(SAMPLE_SECRET_SEED));
        assert!(!json.contains(SAMPLE_SECRET_SEED));
        assert!(!text.contains("my-super-secret-value"));
        assert!(!json.contains("my-super-secret-value"));
    }

    #[test]
    fn parses_both_json_and_toml_plan_documents() {
        let json_doc = r#"{
            "schema": "soroban-forge/transaction-plan@1",
            "network": "testnet",
            "contract": "escrow",
            "method": "deposit",
            "args": [
                { "name": "escrow_id", "type": "u64", "value": 42 }
            ]
        }"#;
        let parsed_json = parse_plan_str(json_doc, Some("json")).unwrap();
        let report_json = validate_plan(&parsed_json, Some("testnet"));
        assert!(report_json.valid);

        let toml_doc = r#"
            schema = "soroban-forge/transaction-plan@1"
            network = "testnet"
            contract = "escrow"
            method = "deposit"
            args = ["escrow_id=u64:42"]
        "#;
        let parsed_toml = parse_plan_str(toml_doc, Some("toml")).unwrap();
        let report_toml = validate_plan(&parsed_toml, Some("testnet"));
        assert!(report_toml.valid);
    }

    #[test]
    fn scval_encoding_supports_all_primitive_and_composite_scalar_types() {
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
        let valid_path = manifest_dir.join("fixtures/plans/valid-escrow-create.json");
        let invalid_path = manifest_dir.join("fixtures/plans/invalid-multi-error.json");

        let valid_plan = load_plan_file(&valid_path).unwrap();
        let valid_report = validate_plan(&valid_plan, Some("testnet"));
        assert!(valid_report.valid);
        assert_eq!(valid_report.exit_code, EXIT_OK);

        let invalid_plan = load_plan_file(&invalid_path).unwrap();
        let invalid_report = validate_plan(&invalid_plan, Some("futurenet"));
        assert!(!invalid_report.valid);
        assert_eq!(invalid_report.exit_code, EXIT_INVALID_PLAN);
        assert!(invalid_report.error_count >= 4);
    }
}
