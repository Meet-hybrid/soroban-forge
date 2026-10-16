use crate::cli::{InvokeArgs, InvokeOutputFormat};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::process::Command;
use std::str::FromStr;
use stellar_xdr::curr::{
    AccountId, BytesM, Duration, Hash, Int128Parts, Limited, Limits, PublicKey, ReadXdr, ScAddress,
    ScBytes, ScError, ScErrorCode, ScMap, ScMapEntry, ScSpecEntry, ScSpecFunctionV0, ScSpecTypeDef,
    ScSpecUdtEnumV0, ScSpecUdtErrorEnumV0, ScSpecUdtStructV0, ScSpecUdtUnionCaseV0,
    ScSpecUdtUnionV0, ScString, ScSymbol, ScVal, ScVec, StringM, TimePoint, Uint256, VecM,
    WriteXdr,
};

/// WASM custom section name carrying concatenated `ScSpecEntry` XDR payloads.
pub const CONTRACT_SPEC_SECTION: &str = "contractspecv0";

/// Explicit type tag supported by `soroban-forge invoke --arg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScValTypeTag {
    Address,
    I128,
    U128,
    I64,
    U64,
    I32,
    U32,
    Bool,
    Symbol,
    String,
    Bytes,
    Void,
    Vec,
    Map,
    Json,
    Xdr,
}

impl ScValTypeTag {
    /// Parse a type tag identifier (case-insensitive).
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "address" | "addr" => Some(Self::Address),
            "i128" => Some(Self::I128),
            "u128" => Some(Self::U128),
            "i64" => Some(Self::I64),
            "u64" => Some(Self::U64),
            "i32" => Some(Self::I32),
            "u32" => Some(Self::U32),
            "bool" | "boolean" => Some(Self::Bool),
            "symbol" | "sym" => Some(Self::Symbol),
            "string" | "str" => Some(Self::String),
            "bytes" | "hex" | "bytes_n" | "bytes32" => Some(Self::Bytes),
            "void" | "unit" => Some(Self::Void),
            "vec" | "array" => Some(Self::Vec),
            "map" => Some(Self::Map),
            "json" | "scval" => Some(Self::Json),
            "xdr" => Some(Self::Xdr),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Address => "address",
            Self::I128 => "i128",
            Self::U128 => "u128",
            Self::I64 => "i64",
            Self::U64 => "u64",
            Self::I32 => "i32",
            Self::U32 => "u32",
            Self::Bool => "bool",
            Self::Symbol => "symbol",
            Self::String => "string",
            Self::Bytes => "bytes",
            Self::Void => "void",
            Self::Vec => "vec",
            Self::Map => "map",
            Self::Json => "json",
            Self::Xdr => "xdr",
        }
    }
}

/// A parsed CLI argument ready for spec validation and `stellar contract invoke` transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedInvokeArg {
    /// Optional parameter name (`--<name>` flag when invoking `stellar`).
    pub name: Option<String>,
    /// Explicit type tag if supplied by the caller.
    pub type_tag: Option<ScValTypeTag>,
    /// Raw string value provided on the command line.
    pub raw_value: String,
    /// Decoded `ScVal` when typed or coerced via contract spec.
    pub scval: Option<ScVal>,
}

impl ParsedInvokeArg {
    /// Render the argument value in the format expected by `stellar contract invoke`.
    #[must_use]
    pub fn cli_value(&self) -> String {
        match &self.scval {
            Some(scval) => scval_to_cli_arg(scval),
            None => self.raw_value.clone(),
        }
    }
}

/// Decoded output from a simulated or executed contract invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecodedInvokeOutput {
    pub contract: String,
    pub function: String,
    pub simulated: bool,
    pub scval_type: String,
    pub value: Value,
    pub xdr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spec_return_type: Option<String>,
}

/// Transport abstraction so unit tests can verify pre-network validation,
/// command construction, and simulation decoding without invoking external processes.
pub trait InvokeRunner {
    fn invoke(&self, program: &str, args: &[String]) -> Result<InvokeCommandOutput>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvokeCommandOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

pub struct SystemInvokeRunner;

impl InvokeRunner for SystemInvokeRunner {
    fn invoke(&self, program: &str, args: &[String]) -> Result<InvokeCommandOutput> {
        let output = Command::new(program)
            .args(args)
            .output()
            .with_context(|| format!("failed to execute `{program}` CLI"))?;
        Ok(InvokeCommandOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Entry point for `soroban-forge invoke`.
pub fn run(args: InvokeArgs) -> Result<()> {
    let rendered = execute_with_runner(&args, &SystemInvokeRunner)?;
    if !rendered.is_empty() {
        println!("{rendered}");
    }
    Ok(())
}

/// Validate arguments (and optional contract spec), run the invocation via `runner`,
/// and decode the simulation or execution result.
pub fn execute_with_runner(args: &InvokeArgs, runner: &impl InvokeRunner) -> Result<String> {
    validate_contract_identifier(&args.contract)?;
    validate_function_name(&args.function)?;

    let spec = match args.wasm.as_deref() {
        Some(path) => Some(ContractSpec::from_path(path)?),
        None => None,
    };

    let mut parsed_args: Vec<ParsedInvokeArg> = args
        .args
        .iter()
        .map(|raw| parse_invoke_arg(raw))
        .collect::<Result<Vec<_>>>()?;

    let spec_return_type = if let Some(ref contract_spec) = spec {
        let fn_spec = contract_spec.validate_and_coerce_call(&args.function, &mut parsed_args)?;
        fn_spec.outputs.first().map(format_spec_type)
    } else {
        None
    };

    let cmd_args = build_stellar_invoke_args(args, &parsed_args);
    let output = runner
        .invoke("stellar", &cmd_args)
        .context("failed to run stellar contract invoke")?;

    if !output.success {
        return Err(enrich_invoke_error(
            args,
            spec.as_ref(),
            &output.stdout,
            &output.stderr,
        ));
    }

    let decoded = decode_simulation_or_invoke_output(
        &args.contract,
        &args.function,
        args.simulate_only,
        spec_return_type,
        &output.stdout,
    )?;

    let format = if args.json {
        InvokeOutputFormat::Json
    } else {
        args.output
    };
    format_decoded_output(&decoded, format)
}

/// Validate `--contract` before any network request.
///
/// Accepts either a valid 56-character `C...` contract Strkey or a non-empty
/// contract alias (e.g. `my_escrow`). Rejects `G...` account Strkeys, `S...`
/// secret keys, or malformed `C...` Strkeys with bad checksums.
pub fn validate_contract_identifier(contract: &str) -> Result<()> {
    let trimmed = contract.trim();
    if trimmed.is_empty() {
        bail!("contract identifier cannot be empty");
    }
    if trimmed.len() == 56 && trimmed.starts_with('G') {
        bail!(
            "invalid contract identifier `{trimmed}`: received a Stellar account Strkey (G...), expected a contract Strkey (C...) or contract alias"
        );
    }
    if trimmed.len() == 56 && trimmed.starts_with('S') {
        bail!(
            "invalid contract identifier `{trimmed}`: received a secret key Strkey (S...), expected a contract Strkey (C...) or contract alias"
        );
    }
    if trimmed.starts_with('C')
        && (trimmed.len() == 56
            || trimmed
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()))
        && trimmed.len() >= 10
    {
        stellar_strkey::Contract::from_string(trimmed).map_err(|err| {
            anyhow::anyhow!(
                "invalid contract Strkey `{trimmed}`: {err} (expected a valid 56-character C... contract address or alias)"
            )
        })?;
    }
    Ok(())
}

/// Validate that the function name is a non-empty Soroban symbol.
pub fn validate_function_name(function: &str) -> Result<()> {
    let trimmed = function.trim();
    if trimmed.is_empty() {
        bail!("function name cannot be empty");
    }
    validate_symbol_str(trimmed)
        .with_context(|| format!("invalid contract function name `{trimmed}`"))?;
    Ok(())
}

/// Parse a single `--arg` entry into a [`ParsedInvokeArg`].
///
/// Supported forms:
/// - `<name>:<type>=<value>` (e.g. `buyer:address=G...`, `amount:i128=1000`)
/// - `<name>=<type>:<value>` (e.g. `buyer=address:G...`, `amount=i128:1000`)
/// - `<type>:<value>` (positional typed, e.g. `u64:42`, `symbol:release`)
/// - `<name>=<value>` (legacy simple named argument, e.g. `amount=100`)
/// - `<value>` (legacy simple positional argument)
pub fn parse_invoke_arg(raw: &str) -> Result<ParsedInvokeArg> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("argument cannot be empty");
    }

    // Check for `<lhs>=<rhs>` first, unless `=` is inside a positional JSON/XDR payload like `json:{"a":1}` or `xdr:AAAA=`.
    if let Some((lhs, rhs)) = split_top_level_equals(trimmed) {
        // Form 1: `<name>:<type>=<value>`
        if let Some((name, type_str)) = lhs.split_once(':') {
            let name = validate_arg_name(name)?;
            let tag = ScValTypeTag::parse(type_str).ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown ScVal type tag `{type_str}` for argument `{name}`; supported types: address, i128, u128, i64, u64, i32, u32, bool, symbol, string, bytes, void, vec, map, json, xdr"
                )
            })?;
            let scval = parse_typed_scval(tag, rhs).with_context(|| {
                format!("invalid `{}` value for argument `{name}`", tag.as_str())
            })?;
            return Ok(ParsedInvokeArg {
                name: Some(name),
                type_tag: Some(tag),
                raw_value: rhs.to_owned(),
                scval: Some(scval),
            });
        }

        let name = validate_arg_name(lhs)?;

        // Form 2: `<name>=<type>:<value>` where `<type>` is a recognized ScValTypeTag.
        if let Some((maybe_type, typed_val)) = rhs.split_once(':') {
            if let Some(tag) = ScValTypeTag::parse(maybe_type) {
                let scval = parse_typed_scval(tag, typed_val).with_context(|| {
                    format!("invalid `{}` value for argument `{name}`", tag.as_str())
                })?;
                return Ok(ParsedInvokeArg {
                    name: Some(name),
                    type_tag: Some(tag),
                    raw_value: typed_val.to_owned(),
                    scval: Some(scval),
                });
            }
        }

        // Form 3: Legacy simple `<name>=<value>`
        return Ok(ParsedInvokeArg {
            name: Some(name),
            type_tag: None,
            raw_value: rhs.to_owned(),
            scval: None,
        });
    }

    // Positional `<type>:<value>`
    if let Some((maybe_type, typed_val)) = trimmed.split_once(':') {
        if let Some(tag) = ScValTypeTag::parse(maybe_type) {
            let scval = parse_typed_scval(tag, typed_val)
                .with_context(|| format!("invalid positional `{}` argument", tag.as_str()))?;
            return Ok(ParsedInvokeArg {
                name: None,
                type_tag: Some(tag),
                raw_value: typed_val.to_owned(),
                scval: Some(scval),
            });
        }
        // If `maybe_type` looks like an unknown type identifier (alphabetic), report a clear error.
        if !maybe_type.is_empty()
            && maybe_type
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            bail!(
                "unknown ScVal type tag `{maybe_type}` in `{trimmed}`; supported types: address, i128, u128, i64, u64, i32, u32, bool, symbol, string, bytes, void, vec, map, json, xdr"
            );
        }
    }

    // Legacy positional `<value>`
    Ok(ParsedInvokeArg {
        name: None,
        type_tag: None,
        raw_value: trimmed.to_owned(),
        scval: None,
    })
}

/// Split on the first `=` only if it appears before any JSON `{`/`[` or positional `<type>:` prefix.
fn split_top_level_equals(s: &str) -> Option<(&str, &str)> {
    // If the string starts with `<known_type>:`, it is a positional typed arg (which may contain `=` in base64 XDR or strings).
    if let Some((prefix, _)) = s.split_once(':') {
        if ScValTypeTag::parse(prefix).is_some() && !prefix.contains('=') {
            return None;
        }
    }
    let eq_idx = s.find('=')?;
    let lhs = &s[..eq_idx];
    if lhs.is_empty() || lhs.contains('{') || lhs.contains('[') || lhs.contains('"') {
        return None;
    }
    Some((lhs, &s[eq_idx + 1..]))
}

fn validate_arg_name(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("argument name cannot be empty");
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        bail!(
            "invalid argument name `{trimmed}`: must contain only ASCII alphanumeric characters, `_`, or `-`"
        );
    }
    Ok(trimmed.to_owned())
}

/// Parse a value string according to an explicit [`ScValTypeTag`].
pub fn parse_typed_scval(tag: ScValTypeTag, raw: &str) -> Result<ScVal> {
    match tag {
        ScValTypeTag::Address => parse_address_scval(raw),
        ScValTypeTag::I128 => {
            let v = parse_int_literal::<i128>(raw, "i128")?;
            Ok(ScVal::from(v))
        }
        ScValTypeTag::U128 => {
            let v = parse_int_literal::<u128>(raw, "u128")?;
            Ok(ScVal::from(v))
        }
        ScValTypeTag::I64 => {
            let v = parse_int_literal::<i64>(raw, "i64")?;
            Ok(ScVal::I64(v))
        }
        ScValTypeTag::U64 => {
            let v = parse_int_literal::<u64>(raw, "u64")?;
            Ok(ScVal::U64(v))
        }
        ScValTypeTag::I32 => {
            let v = parse_int_literal::<i32>(raw, "i32")?;
            Ok(ScVal::I32(v))
        }
        ScValTypeTag::U32 => {
            let v = parse_int_literal::<u32>(raw, "u32")?;
            Ok(ScVal::U32(v))
        }
        ScValTypeTag::Bool => match raw.trim() {
            "true" => Ok(ScVal::Bool(true)),
            "false" => Ok(ScVal::Bool(false)),
            other => bail!("invalid bool `{other}`: expected `true` or `false`"),
        },
        ScValTypeTag::Symbol => {
            let sym = unquote_optional(raw.trim());
            validate_symbol_str(sym)?;
            let string_m: StringM<32> = sym
                .as_bytes()
                .to_vec()
                .try_into()
                .map_err(|_| anyhow::anyhow!("symbol `{sym}` exceeds 32-byte limit"))?;
            Ok(ScVal::Symbol(ScSymbol(string_m)))
        }
        ScValTypeTag::String => {
            let text = unquote_optional(raw);
            let string_m: StringM = text
                .as_bytes()
                .to_vec()
                .try_into()
                .map_err(|_| anyhow::anyhow!("string exceeds maximum XDR length"))?;
            Ok(ScVal::String(ScString(string_m)))
        }
        ScValTypeTag::Bytes => {
            let bytes = decode_hex_bytes(raw)?;
            let bytes_m: BytesM = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("byte slice exceeds maximum XDR length"))?;
            Ok(ScVal::Bytes(ScBytes(bytes_m)))
        }
        ScValTypeTag::Void => {
            let t = raw.trim();
            if t.is_empty() || t == "()" || t == "null" || t == "void" {
                Ok(ScVal::Void)
            } else {
                bail!("invalid void literal `{t}`: expected empty value, `()`, `void`, or `null`")
            }
        }
        ScValTypeTag::Vec => {
            let value: Value = serde_json::from_str(raw.trim())
                .with_context(|| format!("invalid JSON array for `vec`: `{raw}`"))?;
            let arr = value.as_array().ok_or_else(|| {
                anyhow::anyhow!("`vec` value must be a JSON array `[...]`, got {value}")
            })?;
            let mut items = Vec::with_capacity(arr.len());
            for (idx, item) in arr.iter().enumerate() {
                let elem = scval_from_json(item)
                    .with_context(|| format!("invalid element at vec[{idx}]"))?;
                items.push(elem);
            }
            let vec_m: VecM<ScVal> = items
                .try_into()
                .map_err(|_| anyhow::anyhow!("vec exceeds maximum XDR length"))?;
            Ok(ScVal::Vec(Some(ScVec(vec_m))))
        }
        ScValTypeTag::Map => {
            let value: Value = serde_json::from_str(raw.trim())
                .with_context(|| format!("invalid JSON for `map`: `{raw}`"))?;
            parse_map_from_json_value(&value)
        }
        ScValTypeTag::Json => {
            let value: Value = serde_json::from_str(raw.trim())
                .with_context(|| format!("invalid JSON ScVal payload: `{raw}`"))?;
            scval_from_json(&value)
        }
        ScValTypeTag::Xdr => decode_scval_xdr_base64(raw.trim()),
    }
}

/// Validate and parse a Stellar account (`G...`) or contract (`C...`) Strkey into `ScVal::Address`.
pub fn parse_address_scval(raw: &str) -> Result<ScVal> {
    let trimmed = unquote_optional(raw.trim());
    if trimmed.is_empty() {
        bail!("address cannot be empty (expected a 56-character G... account or C... contract Strkey)");
    }
    if trimmed.starts_with('G') {
        let pk = stellar_strkey::ed25519::PublicKey::from_string(trimmed).map_err(|err| {
            anyhow::anyhow!(
                "invalid Stellar account address `{trimmed}`: {err} (check length and CRC16 checksum)"
            )
        })?;
        return Ok(ScVal::Address(ScAddress::Account(AccountId(
            PublicKey::PublicKeyTypeEd25519(Uint256(pk.0)),
        ))));
    }
    if trimmed.starts_with('C') {
        let contract = stellar_strkey::Contract::from_string(trimmed).map_err(|err| {
            anyhow::anyhow!(
                "invalid Soroban contract address `{trimmed}`: {err} (check length and CRC16 checksum)"
            )
        })?;
        return Ok(ScVal::Address(ScAddress::Contract(Hash(contract.0))));
    }
    bail!(
        "invalid Stellar address `{trimmed}`: must start with `G` (account public key) or `C` (contract address)"
    )
}

/// Validate that a Soroban symbol is 1..=32 ASCII `[a-zA-Z0-9_]` characters.
pub fn validate_symbol_str(sym: &str) -> Result<()> {
    if sym.is_empty() {
        bail!("symbol cannot be empty");
    }
    if sym.len() > 32 {
        bail!(
            "symbol `{sym}` is {} bytes; Soroban symbols must be at most 32 bytes",
            sym.len()
        );
    }
    if !sym.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        bail!(
            "symbol `{sym}` contains invalid characters; Soroban symbols only allow `[a-zA-Z0-9_]`"
        );
    }
    Ok(())
}

fn parse_int_literal<T>(raw: &str, type_name: &str) -> Result<T>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    let cleaned = unquote_optional(raw.trim()).replace('_', "");
    if cleaned.is_empty() {
        bail!("empty `{type_name}` integer literal");
    }
    cleaned
        .parse::<T>()
        .map_err(|err| anyhow::anyhow!("invalid `{type_name}` integer `{raw}`: {err}"))
}

fn unquote_optional(s: &str) -> &str {
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Decode a hex string (with optional `0x` / `0X` prefix) into raw bytes.
pub fn decode_hex_bytes(raw: &str) -> Result<Vec<u8>> {
    let trimmed = unquote_optional(raw.trim());
    let hex_str = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    if hex_str.len() % 2 != 0 {
        bail!(
            "hex byte string `{raw}` has odd length ({}); expected an even number of hex digits",
            hex_str.len()
        );
    }
    let mut out = Vec::with_capacity(hex_str.len() / 2);
    let bytes = hex_str.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_nibble(bytes[i]).ok_or_else(|| {
            anyhow::anyhow!(
                "invalid hex character `{}` at index {i} in `{raw}`",
                bytes[i] as char
            )
        })?;
        let lo = hex_nibble(bytes[i + 1]).ok_or_else(|| {
            anyhow::anyhow!(
                "invalid hex character `{}` at index {} in `{raw}`",
                bytes[i + 1] as char,
                i + 1
            )
        })?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Ok(out)
}

const fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Encode raw bytes as lowercase hex.
#[must_use]
pub fn encode_hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Encode raw bytes as standard RFC 4648 base64.
#[must_use]
pub fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };

        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Decode standard RFC 4648 base64 into raw bytes.
pub fn base64_decode(input: &str) -> Result<Vec<u8>> {
    let cleaned: Vec<u8> = input
        .trim()
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    if cleaned.is_empty() {
        bail!("base64 input cannot be empty");
    }
    if cleaned.len() % 4 != 0 {
        bail!(
            "invalid base64 length ({}): must be a multiple of 4",
            cleaned.len()
        );
    }

    let mut out = Vec::with_capacity((cleaned.len() / 4) * 3);
    for (chunk_idx, chunk) in cleaned.chunks_exact(4).enumerate() {
        let pad = chunk.iter().rev().take_while(|&&b| b == b'=').count();
        if pad > 2 || (pad > 0 && (chunk_idx + 1) * 4 != cleaned.len()) {
            bail!("invalid base64 padding in `{input}`");
        }
        let v0 = b64_val(chunk[0])
            .ok_or_else(|| anyhow::anyhow!("invalid base64 character `{}`", chunk[0] as char))?;
        let v1 = b64_val(chunk[1])
            .ok_or_else(|| anyhow::anyhow!("invalid base64 character `{}`", chunk[1] as char))?;
        let v2 = if chunk[2] == b'=' {
            0
        } else {
            b64_val(chunk[2])
                .ok_or_else(|| anyhow::anyhow!("invalid base64 character `{}`", chunk[2] as char))?
        };
        let v3 = if chunk[3] == b'=' {
            0
        } else {
            b64_val(chunk[3])
                .ok_or_else(|| anyhow::anyhow!("invalid base64 character `{}`", chunk[3] as char))?
        };

        out.push((v0 << 2) | (v1 >> 4));
        if pad < 2 {
            out.push(((v1 & 0x0f) << 4) | (v2 >> 2));
        }
        if pad == 0 {
            out.push(((v2 & 0x03) << 6) | v3);
        }
    }
    Ok(out)
}

const fn b64_val(b: u8) -> Option<u8> {
    match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Encode an [`ScVal`] to base64 XDR.
pub fn encode_scval_xdr_base64(scval: &ScVal) -> Result<String> {
    let bytes = scval
        .to_xdr(Limits::none())
        .context("failed to serialize ScVal to XDR")?;
    Ok(base64_encode(&bytes))
}

/// Decode a base64-encoded XDR string into an [`ScVal`].
pub fn decode_scval_xdr_base64(b64: &str) -> Result<ScVal> {
    let bytes = base64_decode(unquote_optional(b64.trim()))?;
    ScVal::from_xdr(&bytes, Limits::none()).context("failed to decode XDR bytes as ScVal")
}

/// Convert an [`ScVal`] into a canonical, lossless JSON representation that round-trips
/// through [`scval_from_json`].
#[must_use]
pub fn scval_to_json(scval: &ScVal) -> Value {
    match scval {
        ScVal::Void => json!({ "void": null }),
        ScVal::Bool(b) => json!({ "bool": b }),
        ScVal::U32(v) => json!({ "u32": v }),
        ScVal::I32(v) => json!({ "i32": v }),
        ScVal::U64(v) => json!({ "u64": v }),
        ScVal::I64(v) => json!({ "i64": v }),
        ScVal::Timepoint(TimePoint(v)) => json!({ "timepoint": v }),
        ScVal::Duration(Duration(v)) => json!({ "duration": v }),
        ScVal::U128(parts) => {
            let val: u128 = parts.into();
            json!({ "u128": val.to_string() })
        }
        ScVal::I128(parts) => {
            let val = i128_from_parts(parts);
            json!({ "i128": val.to_string() })
        }
        ScVal::Bytes(ScBytes(bytes)) => json!({ "bytes": encode_hex_bytes(bytes.as_slice()) }),
        ScVal::String(ScString(s)) => {
            let text = String::from_utf8_lossy(s.as_slice()).into_owned();
            json!({ "string": text })
        }
        ScVal::Symbol(ScSymbol(s)) => {
            let sym = String::from_utf8_lossy(s.as_slice()).into_owned();
            json!({ "symbol": sym })
        }
        ScVal::Address(addr) => json!({ "address": addr.to_string() }),
        ScVal::Vec(Some(ScVec(items))) => {
            let list: Vec<Value> = items.iter().map(scval_to_json).collect();
            json!({ "vec": list })
        }
        ScVal::Vec(None) => json!({ "vec": [] }),
        ScVal::Map(Some(ScMap(entries))) => {
            let list: Vec<Value> = entries
                .iter()
                .map(|entry| {
                    json!({
                        "key": scval_to_json(&entry.key),
                        "val": scval_to_json(&entry.val),
                    })
                })
                .collect();
            json!({ "map": list })
        }
        ScVal::Map(None) => json!({ "map": [] }),
        ScVal::Error(err) => json!({ "error": sc_error_to_json(err) }),
        other => {
            let xdr = encode_scval_xdr_base64(other).unwrap_or_default();
            json!({ "xdr": xdr })
        }
    }
}

/// Parse an [`ScVal`] from a JSON value.
///
/// Accepts:
/// - Tagged single-key objects: `{"address": "G..."}`, `{"i128": "1000"}`, `{"u64": 42}`,
///   `{"bool": true}`, `{"symbol": "transfer"}`, `{"string": "hello"}`, `{"bytes": "deadbeef"}`,
///   `{"void": null}`, `{"vec": [...]}`, `{"map": [...]}`, `{"error": ...}`, `{"xdr": "..."}`
/// - Explicit `{ "type": "<tag>", "value": ... }` objects
/// - Direct JSON primitives inside vectors/maps (`null` -> `Void`, `bool` -> `Bool`,
///   integer -> `I64`/`U64`, `[...]` -> `Vec`)
pub fn scval_from_json(value: &Value) -> Result<ScVal> {
    match value {
        Value::Null => Ok(ScVal::Void),
        Value::Bool(b) => Ok(ScVal::Bool(*b)),
        Value::Number(num) => {
            if let Some(i) = num.as_i64() {
                Ok(ScVal::I64(i))
            } else if let Some(u) = num.as_u64() {
                Ok(ScVal::U64(u))
            } else {
                bail!("floating-point numbers are not supported in Soroban ScVal JSON: {num}")
            }
        }
        Value::String(s) => {
            if let Some((prefix, rest)) = s.split_once(':') {
                if let Some(tag) = ScValTypeTag::parse(prefix) {
                    return parse_typed_scval(tag, rest);
                }
            }
            let string_m: StringM = s
                .as_bytes()
                .to_vec()
                .try_into()
                .map_err(|_| anyhow::anyhow!("string exceeds maximum XDR length"))?;
            Ok(ScVal::String(ScString(string_m)))
        }
        Value::Array(arr) => {
            let mut items = Vec::with_capacity(arr.len());
            for (idx, item) in arr.iter().enumerate() {
                items.push(
                    scval_from_json(item)
                        .with_context(|| format!("invalid ScVal at array index {idx}"))?,
                );
            }
            let vec_m: VecM<ScVal> = items
                .try_into()
                .map_err(|_| anyhow::anyhow!("vec exceeds maximum XDR length"))?;
            Ok(ScVal::Vec(Some(ScVec(vec_m))))
        }
        Value::Object(obj) => {
            if obj.len() == 2 && obj.contains_key("type") && obj.contains_key("value") {
                let type_str = obj["type"].as_str().ok_or_else(|| {
                    anyhow::anyhow!("`type` field in ScVal JSON must be a string")
                })?;
                let single =
                    serde_json::Map::from_iter([(type_str.to_owned(), obj["value"].clone())]);
                return scval_from_json(&Value::Object(single));
            }

            if obj.len() == 1 {
                let (key, val) = obj.iter().next().expect("len == 1");
                match key.as_str() {
                    "void" | "unit" => return Ok(ScVal::Void),
                    "bool" => {
                        let b = val
                            .as_bool()
                            .ok_or_else(|| anyhow::anyhow!("`bool` field must be a boolean"))?;
                        return Ok(ScVal::Bool(b));
                    }
                    "u32" => {
                        let n = json_to_int::<u32>(val, "u32")?;
                        return Ok(ScVal::U32(n));
                    }
                    "i32" => {
                        let n = json_to_int::<i32>(val, "i32")?;
                        return Ok(ScVal::I32(n));
                    }
                    "u64" => {
                        let n = json_to_int::<u64>(val, "u64")?;
                        return Ok(ScVal::U64(n));
                    }
                    "i64" => {
                        let n = json_to_int::<i64>(val, "i64")?;
                        return Ok(ScVal::I64(n));
                    }
                    "timepoint" => {
                        let n = json_to_int::<u64>(val, "timepoint")?;
                        return Ok(ScVal::Timepoint(TimePoint(n)));
                    }
                    "duration" => {
                        let n = json_to_int::<u64>(val, "duration")?;
                        return Ok(ScVal::Duration(Duration(n)));
                    }
                    "u128" => {
                        let n = json_to_int::<u128>(val, "u128")?;
                        return Ok(ScVal::from(n));
                    }
                    "i128" => {
                        let n = json_to_int::<i128>(val, "i128")?;
                        return Ok(ScVal::from(n));
                    }
                    "address" | "addr" => {
                        let s = val.as_str().ok_or_else(|| {
                            anyhow::anyhow!("`address` field must be a Strkey string")
                        })?;
                        return parse_address_scval(s);
                    }
                    "symbol" | "sym" => {
                        let s = val
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("`symbol` field must be a string"))?;
                        return parse_typed_scval(ScValTypeTag::Symbol, s);
                    }
                    "string" | "str" => {
                        let s = val
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("`string` field must be a string"))?;
                        return parse_typed_scval(ScValTypeTag::String, s);
                    }
                    "bytes" | "hex" => {
                        let s = val
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("`bytes` field must be a hex string"))?;
                        return parse_typed_scval(ScValTypeTag::Bytes, s);
                    }
                    "vec" => {
                        let arr = val
                            .as_array()
                            .ok_or_else(|| anyhow::anyhow!("`vec` field must be a JSON array"))?;
                        return scval_from_json(&Value::Array(arr.clone()));
                    }
                    "map" => return parse_map_from_json_value(val),
                    "error" => return sc_error_from_json(val).map(ScVal::Error),
                    "xdr" => {
                        let s = val.as_str().ok_or_else(|| {
                            anyhow::anyhow!("`xdr` field must be a base64 string")
                        })?;
                        return decode_scval_xdr_base64(s);
                    }
                    other => {
                        bail!(
                            "unknown ScVal JSON tag `{other}`; expected one of: void, bool, u32, i32, u64, i64, timepoint, duration, u128, i128, address, symbol, string, bytes, vec, map, error, xdr"
                        )
                    }
                }
            }

            bail!(
                "ambiguous JSON ScVal object with {} keys; wrap map entries in `{{\"map\": [...]}}` or use a single-key type tag",
                obj.len()
            )
        }
    }
}

fn parse_map_from_json_value(val: &Value) -> Result<ScVal> {
    match val {
        Value::Array(arr) => {
            let mut entries = Vec::with_capacity(arr.len());
            for (idx, item) in arr.iter().enumerate() {
                let obj = item.as_object().ok_or_else(|| {
                    anyhow::anyhow!(
                        "map entry at index {idx} must be an object `{{\"key\": ..., \"val\": ...}}`"
                    )
                })?;
                let key_json = obj.get("key").ok_or_else(|| {
                    anyhow::anyhow!("map entry at index {idx} is missing `key` field")
                })?;
                let val_json = obj
                    .get("val")
                    .or_else(|| obj.get("value"))
                    .ok_or_else(|| {
                        anyhow::anyhow!("map entry at index {idx} is missing `val` field")
                    })?;
                let key = scval_from_json(key_json)
                    .with_context(|| format!("invalid map key at index {idx}"))?;
                let value = scval_from_json(val_json)
                    .with_context(|| format!("invalid map value at index {idx}"))?;
                entries.push(ScMapEntry { key, val: value });
            }
            let map_m: VecM<ScMapEntry> = entries
                .try_into()
                .map_err(|_| anyhow::anyhow!("map exceeds maximum XDR length"))?;
            Ok(ScVal::Map(Some(ScMap(map_m))))
        }
        Value::Object(obj) => {
            let mut entries = Vec::with_capacity(obj.len());
            for (k, v) in obj {
                let key = if validate_symbol_str(k).is_ok() {
                    parse_typed_scval(ScValTypeTag::Symbol, k)?
                } else {
                    parse_typed_scval(ScValTypeTag::String, k)?
                };
                let value = scval_from_json(v)
                    .with_context(|| format!("invalid value for map key `{k}`"))?;
                entries.push(ScMapEntry { key, val: value });
            }
            let map_m: VecM<ScMapEntry> = entries
                .try_into()
                .map_err(|_| anyhow::anyhow!("map exceeds maximum XDR length"))?;
            Ok(ScVal::Map(Some(ScMap(map_m))))
        }
        other => bail!("`map` value must be a JSON array of `{{\"key\":...,\"val\":...}}` or a JSON object, got {other}"),
    }
}

fn json_to_int<T>(val: &Value, type_name: &str) -> Result<T>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    match val {
        Value::Number(n) => parse_int_literal::<T>(&n.to_string(), type_name),
        Value::String(s) => parse_int_literal::<T>(s, type_name),
        other => bail!("expected integer or numeric string for `{type_name}`, got {other}"),
    }
}

fn sc_error_to_json(err: &ScError) -> Value {
    match err {
        ScError::Contract(code) => json!({
            "type": "contract",
            "code": code,
        }),
        other => {
            let (kind, code) = sc_error_domain_and_code(other);
            json!({
                "type": kind,
                "code": code.name(),
            })
        }
    }
}

fn sc_error_from_json(val: &Value) -> Result<ScError> {
    let obj = val
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("`error` field must be a JSON object"))?;
    let err_type = obj
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("`error` object requires string `type`"))?;
    if err_type.eq_ignore_ascii_case("contract") {
        let code = obj
            .get("code")
            .ok_or_else(|| anyhow::anyhow!("contract error requires `code`"))?;
        let n = json_to_int::<u32>(code, "u32")?;
        return Ok(ScError::Contract(n));
    }
    let code_str = obj
        .get("code")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("host ScError requires string `code`"))?;
    let code = parse_sc_error_code(code_str)?;
    match err_type.to_ascii_lowercase().as_str() {
        "wasm_vm" | "wasmvm" => Ok(ScError::WasmVm(code)),
        "context" => Ok(ScError::Context(code)),
        "storage" => Ok(ScError::Storage(code)),
        "object" => Ok(ScError::Object(code)),
        "crypto" => Ok(ScError::Crypto(code)),
        "events" => Ok(ScError::Events(code)),
        "budget" => Ok(ScError::Budget(code)),
        "value" => Ok(ScError::Value(code)),
        "auth" => Ok(ScError::Auth(code)),
        other => bail!("unknown ScError type `{other}`"),
    }
}

fn parse_sc_error_code(raw: &str) -> Result<ScErrorCode> {
    match raw.to_ascii_lowercase().replace('_', "").as_str() {
        "arithdomain" => Ok(ScErrorCode::ArithDomain),
        "indexbounds" => Ok(ScErrorCode::IndexBounds),
        "invalidinput" => Ok(ScErrorCode::InvalidInput),
        "missingvalue" => Ok(ScErrorCode::MissingValue),
        "existingvalue" => Ok(ScErrorCode::ExistingValue),
        "exceededlimit" => Ok(ScErrorCode::ExceededLimit),
        "invalidaction" => Ok(ScErrorCode::InvalidAction),
        "internalerror" => Ok(ScErrorCode::InternalError),
        "unexpectedtype" => Ok(ScErrorCode::UnexpectedType),
        "unexpectedsize" => Ok(ScErrorCode::UnexpectedSize),
        _ => bail!("unknown ScErrorCode `{raw}`"),
    }
}

const fn sc_error_domain_and_code(err: &ScError) -> (&'static str, ScErrorCode) {
    match err {
        ScError::Contract(_) => ("contract", ScErrorCode::InvalidInput),
        ScError::WasmVm(c) => ("wasm_vm", *c),
        ScError::Context(c) => ("context", *c),
        ScError::Storage(c) => ("storage", *c),
        ScError::Object(c) => ("object", *c),
        ScError::Crypto(c) => ("crypto", *c),
        ScError::Events(c) => ("events", *c),
        ScError::Budget(c) => ("budget", *c),
        ScError::Value(c) => ("value", *c),
        ScError::Auth(c) => ("auth", *c),
    }
}

#[allow(clippy::cast_sign_loss)]
const fn i128_from_parts(parts: &Int128Parts) -> i128 {
    ((parts.hi as i128) << 64) | (parts.lo as i128)
}

/// Convert an [`ScVal`] into the CLI argument string representation expected by `stellar contract invoke`.
#[must_use]
pub fn scval_to_cli_arg(scval: &ScVal) -> String {
    match scval {
        ScVal::Void => "null".to_owned(),
        ScVal::Bool(b) => b.to_string(),
        ScVal::U32(v) => v.to_string(),
        ScVal::I32(v) => v.to_string(),
        ScVal::U64(v) => v.to_string(),
        ScVal::I64(v) => v.to_string(),
        ScVal::Timepoint(TimePoint(v)) | ScVal::Duration(Duration(v)) => v.to_string(),
        ScVal::U128(parts) => {
            let v: u128 = parts.into();
            v.to_string()
        }
        ScVal::I128(parts) => i128_from_parts(parts).to_string(),
        ScVal::Bytes(ScBytes(bytes)) => encode_hex_bytes(bytes.as_slice()),
        ScVal::String(ScString(s)) => String::from_utf8_lossy(s.as_slice()).into_owned(),
        ScVal::Symbol(ScSymbol(s)) => String::from_utf8_lossy(s.as_slice()).into_owned(),
        ScVal::Address(addr) => addr.to_string(),
        ScVal::Vec(Some(ScVec(items))) => {
            let vals: Vec<Value> = items.iter().map(scval_to_plain_json).collect();
            serde_json::to_string(&vals).unwrap_or_else(|_| "[]".to_owned())
        }
        ScVal::Vec(None) => "[]".to_owned(),
        ScVal::Map(Some(ScMap(entries))) => {
            let mut map = serde_json::Map::new();
            for entry in entries.iter() {
                let k = scval_to_cli_arg(&entry.key);
                map.insert(k, scval_to_plain_json(&entry.val));
            }
            serde_json::to_string(&Value::Object(map)).unwrap_or_else(|_| "{}".to_owned())
        }
        ScVal::Map(None) => "{}".to_owned(),
        other => encode_scval_xdr_base64(other).unwrap_or_default(),
    }
}

fn scval_to_plain_json(scval: &ScVal) -> Value {
    match scval {
        ScVal::Void => Value::Null,
        ScVal::Bool(b) => Value::Bool(*b),
        ScVal::U32(v) => json!(v),
        ScVal::I32(v) => json!(v),
        ScVal::U64(v) => json!(v),
        ScVal::I64(v) => json!(v),
        ScVal::Timepoint(TimePoint(v)) | ScVal::Duration(Duration(v)) => json!(v),
        ScVal::U128(parts) => {
            let v: u128 = parts.into();
            json!(v.to_string())
        }
        ScVal::I128(parts) => json!(i128_from_parts(parts).to_string()),
        ScVal::Bytes(ScBytes(bytes)) => json!(encode_hex_bytes(bytes.as_slice())),
        ScVal::String(ScString(s)) => json!(String::from_utf8_lossy(s.as_slice())),
        ScVal::Symbol(ScSymbol(s)) => json!(String::from_utf8_lossy(s.as_slice())),
        ScVal::Address(addr) => json!(addr.to_string()),
        ScVal::Vec(Some(ScVec(items))) => {
            Value::Array(items.iter().map(scval_to_plain_json).collect())
        }
        ScVal::Vec(None) => Value::Array(Vec::new()),
        ScVal::Map(Some(ScMap(entries))) => {
            let mut map = serde_json::Map::new();
            for entry in entries.iter() {
                map.insert(
                    scval_to_cli_arg(&entry.key),
                    scval_to_plain_json(&entry.val),
                );
            }
            Value::Object(map)
        }
        ScVal::Map(None) => Value::Object(serde_json::Map::new()),
        other => scval_to_json(other),
    }
}

/// Build the `stellar contract invoke` argument list from [`InvokeArgs`] and validated [`ParsedInvokeArg`]s.
#[must_use]
pub fn build_stellar_invoke_args(
    args: &InvokeArgs,
    parsed_args: &[ParsedInvokeArg],
) -> Vec<String> {
    let mut cmd_args = vec![
        "contract".to_owned(),
        "invoke".to_owned(),
        "--id".to_owned(),
        args.contract.clone(),
    ];

    if let Some(ref source) = args.source {
        cmd_args.push("--source-account".to_owned());
        cmd_args.push(source.clone());
    }

    cmd_args.push("--network".to_owned());
    cmd_args.push(args.network.clone());

    if args.simulate_only {
        cmd_args.push("--send=no".to_owned());
    }

    cmd_args.push("--".to_owned());
    cmd_args.push(args.function.clone());

    for arg in parsed_args {
        if let Some(ref name) = arg.name {
            cmd_args.push(format!("--{name}"));
            cmd_args.push(arg.cli_value());
        } else {
            cmd_args.push(arg.cli_value());
        }
    }

    cmd_args
}

/// Parsed Soroban contract specification loaded from a `.wasm` binary or raw XDR spec stream.
#[derive(Debug, Clone, Default)]
pub struct ContractSpec {
    pub functions: BTreeMap<String, ScSpecFunctionV0>,
    pub structs: BTreeMap<String, ScSpecUdtStructV0>,
    pub unions: BTreeMap<String, ScSpecUdtUnionV0>,
    pub enums: BTreeMap<String, ScSpecUdtEnumV0>,
    pub errors: BTreeMap<u32, (String, String)>,
}

impl ContractSpec {
    /// Load a [`ContractSpec`] from a `.wasm` artifact or raw XDR spec file on disk.
    pub fn from_path(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).with_context(|| {
            format!(
                "failed to read contract spec/WASM file at {}",
                path.display()
            )
        })?;
        if bytes.is_empty() {
            bail!("contract spec/WASM file at {} is empty", path.display());
        }
        Self::from_wasm_or_xdr_bytes(&bytes)
            .with_context(|| format!("failed to parse contract spec from {}", path.display()))
    }

    /// Parse a [`ContractSpec`] from either a WebAssembly binary (extracting the
    /// `contractspecv0` custom section) or raw concatenated `ScSpecEntry` XDR bytes.
    pub fn from_wasm_or_xdr_bytes(bytes: &[u8]) -> Result<Self> {
        let spec_bytes = if bytes.starts_with(b"\0asm") {
            extract_wasm_custom_section(bytes, CONTRACT_SPEC_SECTION)?.ok_or_else(|| {
                anyhow::anyhow!(
                    "WASM binary does not contain a `{CONTRACT_SPEC_SECTION}` custom section"
                )
            })?
        } else {
            bytes.to_vec()
        };

        let entries = decode_spec_entries(&spec_bytes)?;
        if entries.is_empty() {
            bail!("contract spec contains no ScSpecEntry definitions");
        }
        Ok(Self::from_entries(entries))
    }

    /// Build a [`ContractSpec`] from decoded [`ScSpecEntry`] items.
    #[must_use]
    pub fn from_entries(entries: Vec<ScSpecEntry>) -> Self {
        let mut spec = Self::default();
        for entry in entries {
            match entry {
                ScSpecEntry::FunctionV0(func) => {
                    let name = String::from_utf8_lossy(func.name.0.as_slice()).into_owned();
                    spec.functions.insert(name, func);
                }
                ScSpecEntry::UdtStructV0(s) => {
                    let name = String::from_utf8_lossy(s.name.as_slice()).into_owned();
                    spec.structs.insert(name, s);
                }
                ScSpecEntry::UdtUnionV0(u) => {
                    let name = String::from_utf8_lossy(u.name.as_slice()).into_owned();
                    spec.unions.insert(name, u);
                }
                ScSpecEntry::UdtEnumV0(e) => {
                    let name = String::from_utf8_lossy(e.name.as_slice()).into_owned();
                    spec.enums.insert(name, e);
                }
                ScSpecEntry::UdtErrorEnumV0(err_enum) => {
                    spec.register_error_enum(&err_enum);
                }
            }
        }
        spec
    }

    fn register_error_enum(&mut self, err_enum: &ScSpecUdtErrorEnumV0) {
        for case in err_enum.cases.iter() {
            let name = String::from_utf8_lossy(case.name.as_slice()).into_owned();
            let doc = String::from_utf8_lossy(case.doc.as_slice()).into_owned();
            self.errors.insert(case.value, (name, doc));
        }
    }

    /// Validate that `function` exists in the spec and that `args` match its signature,
    /// coercing untyped arguments and reordering named arguments to match the spec order.
    pub fn validate_and_coerce_call<'a>(
        &'a self,
        function: &str,
        args: &mut Vec<ParsedInvokeArg>,
    ) -> Result<&'a ScSpecFunctionV0> {
        let func = self.functions.get(function).ok_or_else(|| {
            let available: Vec<&str> = self.functions.keys().map(String::as_str).collect();
            anyhow::anyhow!(
                "function `{function}` not found in contract spec; available functions: [{}]",
                available.join(", ")
            )
        })?;

        let expected_inputs = func.inputs.as_slice();
        if args.len() != expected_inputs.len() {
            let signature: Vec<String> = expected_inputs
                .iter()
                .map(|inp| {
                    format!(
                        "{}: {}",
                        String::from_utf8_lossy(inp.name.as_slice()),
                        format_spec_type(&inp.type_)
                    )
                })
                .collect();
            bail!(
                "function `{function}` expects {} argument(s) ({}), but {} argument(s) were supplied",
                expected_inputs.len(),
                signature.join(", "),
                args.len()
            );
        }

        let all_named = args.iter().all(|a| a.name.is_some());
        let any_named = args.iter().any(|a| a.name.is_some());
        if any_named && !all_named {
            bail!(
                "cannot mix named (`--arg name=...`) and positional arguments when validating `{function}` against contract spec"
            );
        }

        if all_named {
            let mut reordered = Vec::with_capacity(expected_inputs.len());
            for expected in expected_inputs {
                let param_name = String::from_utf8_lossy(expected.name.as_slice()).into_owned();
                let pos = args
                    .iter()
                    .position(|a| a.name.as_deref() == Some(param_name.as_str()))
                    .ok_or_else(|| {
                        let expected_names: Vec<String> = expected_inputs
                            .iter()
                            .map(|i| String::from_utf8_lossy(i.name.as_slice()).into_owned())
                            .collect();
                        anyhow::anyhow!(
                            "missing required argument `{param_name}` (type `{}`) for function `{function}`; expected parameters: [{}]",
                            format_spec_type(&expected.type_),
                            expected_names.join(", ")
                        )
                    })?;
                let mut arg = args.remove(pos);
                self.validate_or_coerce_arg(function, &param_name, &expected.type_, &mut arg)?;
                reordered.push(arg);
            }
            *args = reordered;
        } else {
            for (idx, (expected, arg)) in expected_inputs.iter().zip(args.iter_mut()).enumerate() {
                let param_name = String::from_utf8_lossy(expected.name.as_slice()).into_owned();
                arg.name = Some(param_name.clone());
                self.validate_or_coerce_arg(
                    function,
                    &format!("{param_name} (position #{})", idx + 1),
                    &expected.type_,
                    arg,
                )?;
            }
        }

        Ok(func)
    }

    fn validate_or_coerce_arg(
        &self,
        function: &str,
        param_label: &str,
        expected_type: &ScSpecTypeDef,
        arg: &mut ParsedInvokeArg,
    ) -> Result<()> {
        if arg.scval.is_none() {
            let coerced = self
                .coerce_raw_to_spec_type(&arg.raw_value, expected_type)
                .with_context(|| {
                    format!(
                        "argument `{param_label}` in `{function}` failed contract-spec validation (expected `{}`)",
                        format_spec_type(expected_type)
                    )
                })?;
            arg.scval = Some(coerced);
            return Ok(());
        }

        let scval = arg.scval.as_ref().expect("checked Some");
        self.check_scval_matches_spec(scval, expected_type)
            .with_context(|| {
                format!(
                    "type mismatch for argument `{param_label}` in `{function}`: expected `{}`",
                    format_spec_type(expected_type)
                )
            })
    }

    fn coerce_raw_to_spec_type(&self, raw: &str, expected: &ScSpecTypeDef) -> Result<ScVal> {
        match expected {
            ScSpecTypeDef::Val => {
                if let Ok(v) = serde_json::from_str::<Value>(raw) {
                    scval_from_json(&v)
                } else {
                    parse_typed_scval(ScValTypeTag::String, raw)
                }
            }
            ScSpecTypeDef::Bool => parse_typed_scval(ScValTypeTag::Bool, raw),
            ScSpecTypeDef::Void => parse_typed_scval(ScValTypeTag::Void, raw),
            ScSpecTypeDef::U32 => parse_typed_scval(ScValTypeTag::U32, raw),
            ScSpecTypeDef::I32 => parse_typed_scval(ScValTypeTag::I32, raw),
            ScSpecTypeDef::U64 => parse_typed_scval(ScValTypeTag::U64, raw),
            ScSpecTypeDef::I64 => parse_typed_scval(ScValTypeTag::I64, raw),
            ScSpecTypeDef::Timepoint => {
                let n = parse_int_literal::<u64>(raw, "timepoint")?;
                Ok(ScVal::Timepoint(TimePoint(n)))
            }
            ScSpecTypeDef::Duration => {
                let n = parse_int_literal::<u64>(raw, "duration")?;
                Ok(ScVal::Duration(Duration(n)))
            }
            ScSpecTypeDef::U128 => parse_typed_scval(ScValTypeTag::U128, raw),
            ScSpecTypeDef::I128 => parse_typed_scval(ScValTypeTag::I128, raw),
            ScSpecTypeDef::Bytes => parse_typed_scval(ScValTypeTag::Bytes, raw),
            ScSpecTypeDef::BytesN(bn) => {
                let scval = parse_typed_scval(ScValTypeTag::Bytes, raw)?;
                self.check_scval_matches_spec(&scval, expected)?;
                let _ = bn;
                Ok(scval)
            }
            ScSpecTypeDef::String => parse_typed_scval(ScValTypeTag::String, raw),
            ScSpecTypeDef::Symbol => parse_typed_scval(ScValTypeTag::Symbol, raw),
            ScSpecTypeDef::Address => parse_typed_scval(ScValTypeTag::Address, raw),
            ScSpecTypeDef::Option(opt) => {
                let t = raw.trim();
                if t.is_empty() || t == "null" || t == "none" || t == "None" {
                    Ok(ScVal::Void)
                } else {
                    self.coerce_raw_to_spec_type(t, &opt.value_type)
                }
            }
            ScSpecTypeDef::Vec(_) | ScSpecTypeDef::Tuple(_) => {
                let scval = parse_typed_scval(ScValTypeTag::Vec, raw)?;
                self.check_scval_matches_spec(&scval, expected)?;
                Ok(scval)
            }
            ScSpecTypeDef::Map(_) => {
                let scval = parse_typed_scval(ScValTypeTag::Map, raw)?;
                self.check_scval_matches_spec(&scval, expected)?;
                Ok(scval)
            }
            ScSpecTypeDef::Udt(udt) => {
                let name = String::from_utf8_lossy(udt.name.as_slice());
                if self.enums.contains_key(name.as_ref()) {
                    let scval = parse_typed_scval(ScValTypeTag::U32, raw)?;
                    self.check_scval_matches_spec(&scval, expected)?;
                    return Ok(scval);
                }
                let value: Value = serde_json::from_str(raw.trim()).with_context(|| {
                    format!("expected JSON representation for user-defined type `{name}`")
                })?;
                let scval = scval_from_json(&value)?;
                self.check_scval_matches_spec(&scval, expected)?;
                Ok(scval)
            }
            other => bail!(
                "unsupported untyped coercion for spec type `{}`; supply an explicit typed argument",
                format_spec_type(other)
            ),
        }
    }

    pub fn check_scval_matches_spec(&self, scval: &ScVal, expected: &ScSpecTypeDef) -> Result<()> {
        match (expected, scval) {
            (ScSpecTypeDef::Val, _) => Ok(()),
            (ScSpecTypeDef::Bool, ScVal::Bool(_)) => Ok(()),
            (ScSpecTypeDef::Void, ScVal::Void) => Ok(()),
            (ScSpecTypeDef::U32, ScVal::U32(_)) => Ok(()),
            (ScSpecTypeDef::I32, ScVal::I32(_)) => Ok(()),
            (ScSpecTypeDef::U64, ScVal::U64(_)) => Ok(()),
            (ScSpecTypeDef::I64, ScVal::I64(_)) => Ok(()),
            (ScSpecTypeDef::Timepoint, ScVal::Timepoint(_) | ScVal::U64(_)) => Ok(()),
            (ScSpecTypeDef::Duration, ScVal::Duration(_) | ScVal::U64(_)) => Ok(()),
            (ScSpecTypeDef::U128, ScVal::U128(_)) => Ok(()),
            (ScSpecTypeDef::I128, ScVal::I128(_)) => Ok(()),
            (ScSpecTypeDef::Bytes, ScVal::Bytes(_)) => Ok(()),
            (ScSpecTypeDef::BytesN(bn), ScVal::Bytes(ScBytes(bytes))) => {
                let actual_len = bytes.len();
                let expected_len = bn.n as usize;
                if actual_len != expected_len {
                    bail!(
                        "BytesN<{expected_len}> requires exactly {expected_len} bytes, but got {actual_len} bytes"
                    );
                }
                Ok(())
            }
            (ScSpecTypeDef::String, ScVal::String(_)) => Ok(()),
            (ScSpecTypeDef::Symbol, ScVal::Symbol(_)) => Ok(()),
            (ScSpecTypeDef::Address, ScVal::Address(_)) => Ok(()),
            (ScSpecTypeDef::Option(_), ScVal::Void) => Ok(()),
            (ScSpecTypeDef::Option(opt), other) => {
                self.check_scval_matches_spec(other, &opt.value_type)
            }
            (ScSpecTypeDef::Vec(vec_spec), ScVal::Vec(Some(ScVec(items)))) => {
                for (idx, item) in items.iter().enumerate() {
                    self.check_scval_matches_spec(item, &vec_spec.element_type)
                        .with_context(|| format!("invalid element at vec[{idx}]"))?;
                }
                Ok(())
            }
            (ScSpecTypeDef::Vec(_), ScVal::Vec(None)) => Ok(()),
            (ScSpecTypeDef::Map(map_spec), ScVal::Map(Some(ScMap(entries)))) => {
                for (idx, entry) in entries.iter().enumerate() {
                    self.check_scval_matches_spec(&entry.key, &map_spec.key_type)
                        .with_context(|| format!("invalid map key at entry #{idx}"))?;
                    self.check_scval_matches_spec(&entry.val, &map_spec.value_type)
                        .with_context(|| format!("invalid map value at entry #{idx}"))?;
                }
                Ok(())
            }
            (ScSpecTypeDef::Map(_), ScVal::Map(None)) => Ok(()),
            (ScSpecTypeDef::Tuple(tuple_spec), ScVal::Vec(Some(ScVec(items)))) => {
                let expected_types = tuple_spec.value_types.as_slice();
                if items.len() != expected_types.len() {
                    bail!(
                        "tuple expects {} element(s), got {}",
                        expected_types.len(),
                        items.len()
                    );
                }
                for (idx, (item, exp)) in items.iter().zip(expected_types.iter()).enumerate() {
                    self.check_scval_matches_spec(item, exp)
                        .with_context(|| format!("invalid tuple element at index {idx}"))?;
                }
                Ok(())
            }
            (ScSpecTypeDef::Udt(udt), val) => {
                let udt_name = String::from_utf8_lossy(udt.name.as_slice()).into_owned();
                if let Some(enum_def) = self.enums.get(&udt_name) {
                    if let ScVal::U32(disc) = val {
                        if enum_def.cases.iter().any(|c| c.value == *disc) {
                            return Ok(());
                        }
                        bail!("u32 value {disc} is not a valid discriminant for enum `{udt_name}`");
                    }
                    bail!(
                        "UDT enum `{udt_name}` requires `u32`, got `{}`",
                        scval_type_name(val)
                    );
                }
                if let Some(struct_def) = self.structs.get(&udt_name) {
                    if let ScVal::Map(Some(ScMap(entries))) = val {
                        if entries.len() != struct_def.fields.len() {
                            bail!(
                                "UDT struct `{udt_name}` expects {} field(s), got {}",
                                struct_def.fields.len(),
                                entries.len()
                            );
                        }
                        return Ok(());
                    }
                    bail!(
                        "UDT struct `{udt_name}` requires `map`, got `{}`",
                        scval_type_name(val)
                    );
                }
                if let Some(union_def) = self.unions.get(&udt_name) {
                    if let ScVal::Vec(Some(ScVec(items))) = val {
                        if let Some(ScVal::Symbol(ScSymbol(tag_bytes))) = items.first() {
                            let tag_str = String::from_utf8_lossy(tag_bytes.as_slice());
                            let matching_case = union_def.cases.iter().find(|case| match case {
                                ScSpecUdtUnionCaseV0::VoidV0(v) => {
                                    String::from_utf8_lossy(v.name.as_slice()) == tag_str
                                }
                                ScSpecUdtUnionCaseV0::TupleV0(t) => {
                                    String::from_utf8_lossy(t.name.as_slice()) == tag_str
                                }
                            });
                            if matching_case.is_some() {
                                return Ok(());
                            }
                            bail!("unknown variant `{tag_str}` for UDT union `{udt_name}`");
                        }
                    }
                    bail!(
                        "UDT union `{udt_name}` requires a `vec` starting with a variant `symbol`, got `{}`",
                        scval_type_name(val)
                    );
                }
                Ok(())
            }
            (exp, actual) => bail!(
                "expected `{}` but received ScVal of type `{}`",
                format_spec_type(exp),
                scval_type_name(actual)
            ),
        }
    }

    /// Resolve a numeric contract error code against `ScSpecUdtErrorEnumV0` entries
    /// (falling back to standard `ForgeError` codes when applicable).
    #[must_use]
    pub fn describe_contract_error(&self, code: u32) -> Option<String> {
        if let Some((name, doc)) = self.errors.get(&code) {
            if doc.trim().is_empty() {
                return Some(format!("{name} (code #{code})"));
            }
            return Some(format!("{name} (code #{code}): {}", doc.trim()));
        }
        describe_forge_error_code(code)
    }
}

/// Extract a named custom section (`section_id == 0`) from a WebAssembly binary.
pub fn extract_wasm_custom_section(wasm: &[u8], target_name: &str) -> Result<Option<Vec<u8>>> {
    if wasm.len() < 8 || &wasm[0..4] != b"\0asm" {
        bail!("invalid WebAssembly magic header");
    }
    if &wasm[4..8] != b"\x01\x00\x00\x00" {
        bail!("unsupported WebAssembly version header");
    }

    let mut pos = 8;
    let mut accumulated = Vec::new();
    while pos < wasm.len() {
        let section_id = wasm[pos];
        pos += 1;
        let (section_len, bytes_read) = read_leb128_u32(&wasm[pos..])?;
        pos += bytes_read;
        let section_end = pos
            .checked_add(section_len as usize)
            .ok_or_else(|| anyhow::anyhow!("WASM section length overflow"))?;
        if section_end > wasm.len() {
            bail!("truncated WASM section (expected {section_len} bytes)");
        }

        if section_id == 0 {
            let section_slice = &wasm[pos..section_end];
            let (name_len, name_len_bytes) = read_leb128_u32(section_slice)?;
            let name_start = name_len_bytes;
            let name_end = name_start
                .checked_add(name_len as usize)
                .ok_or_else(|| anyhow::anyhow!("WASM custom section name overflow"))?;
            if name_end > section_slice.len() {
                bail!("truncated WASM custom section name");
            }
            let name = std::str::from_utf8(&section_slice[name_start..name_end])
                .context("invalid UTF-8 in WASM custom section name")?;
            if name == target_name {
                accumulated.extend_from_slice(&section_slice[name_end..]);
            }
        }

        pos = section_end;
    }

    if accumulated.is_empty() {
        Ok(None)
    } else {
        Ok(Some(accumulated))
    }
}

fn read_leb128_u32(bytes: &[u8]) -> Result<(u32, usize)> {
    let mut result = 0_u32;
    let mut shift = 0_u32;
    for (idx, &byte) in bytes.iter().take(5).enumerate() {
        let low = u32::from(byte & 0x7f);
        if shift == 28 && (byte & 0xf0) != 0 {
            bail!("LEB128 u32 overflow");
        }
        result |= low << shift;
        if (byte & 0x80) == 0 {
            return Ok((result, idx + 1));
        }
        shift += 7;
    }
    bail!("unterminated or invalid LEB128 u32 in WASM binary")
}

/// Decode a stream of concatenated XDR [`ScSpecEntry`] records.
pub fn decode_spec_entries(spec_bytes: &[u8]) -> Result<Vec<ScSpecEntry>> {
    let mut cursor = Cursor::new(spec_bytes);
    let mut limited = Limited::new(&mut cursor, Limits::none());
    let mut entries = Vec::new();
    while (limited.inner.position() as usize) < spec_bytes.len() {
        let entry = ScSpecEntry::read_xdr(&mut limited)
            .context("failed to read ScSpecEntry from contract spec stream")?;
        entries.push(entry);
    }
    Ok(entries)
}

/// Format an [`ScSpecTypeDef`] as a human-readable type signature.
#[must_use]
pub fn format_spec_type(ty: &ScSpecTypeDef) -> String {
    match ty {
        ScSpecTypeDef::Val => "val".to_owned(),
        ScSpecTypeDef::Bool => "bool".to_owned(),
        ScSpecTypeDef::Void => "void".to_owned(),
        ScSpecTypeDef::Error => "error".to_owned(),
        ScSpecTypeDef::U32 => "u32".to_owned(),
        ScSpecTypeDef::I32 => "i32".to_owned(),
        ScSpecTypeDef::U64 => "u64".to_owned(),
        ScSpecTypeDef::I64 => "i64".to_owned(),
        ScSpecTypeDef::Timepoint => "timepoint".to_owned(),
        ScSpecTypeDef::Duration => "duration".to_owned(),
        ScSpecTypeDef::U128 => "u128".to_owned(),
        ScSpecTypeDef::I128 => "i128".to_owned(),
        ScSpecTypeDef::U256 => "u256".to_owned(),
        ScSpecTypeDef::I256 => "i256".to_owned(),
        ScSpecTypeDef::Bytes => "bytes".to_owned(),
        ScSpecTypeDef::BytesN(bn) => format!("bytes_n<{}>", bn.n),
        ScSpecTypeDef::String => "string".to_owned(),
        ScSpecTypeDef::Symbol => "symbol".to_owned(),
        ScSpecTypeDef::Address => "address".to_owned(),
        ScSpecTypeDef::Option(opt) => format!("option<{}>", format_spec_type(&opt.value_type)),
        ScSpecTypeDef::Result(res) => format!(
            "result<{}, {}>",
            format_spec_type(&res.ok_type),
            format_spec_type(&res.error_type)
        ),
        ScSpecTypeDef::Vec(vec_ty) => format!("vec<{}>", format_spec_type(&vec_ty.element_type)),
        ScSpecTypeDef::Map(map_ty) => format!(
            "map<{}, {}>",
            format_spec_type(&map_ty.key_type),
            format_spec_type(&map_ty.value_type)
        ),
        ScSpecTypeDef::Tuple(tup) => {
            let elems: Vec<String> = tup.value_types.iter().map(format_spec_type).collect();
            format!("tuple<({})>", elems.join(", "))
        }
        ScSpecTypeDef::Udt(udt) => String::from_utf8_lossy(udt.name.as_slice()).into_owned(),
    }
}

#[must_use]
pub const fn scval_type_name(scval: &ScVal) -> &'static str {
    match scval {
        ScVal::Bool(_) => "bool",
        ScVal::Void => "void",
        ScVal::Error(_) => "error",
        ScVal::U32(_) => "u32",
        ScVal::I32(_) => "i32",
        ScVal::U64(_) => "u64",
        ScVal::I64(_) => "i64",
        ScVal::Timepoint(_) => "timepoint",
        ScVal::Duration(_) => "duration",
        ScVal::U128(_) => "u128",
        ScVal::I128(_) => "i128",
        ScVal::U256(_) => "u256",
        ScVal::I256(_) => "i256",
        ScVal::Bytes(_) => "bytes",
        ScVal::String(_) => "string",
        ScVal::Symbol(_) => "symbol",
        ScVal::Vec(_) => "vec",
        ScVal::Map(_) => "map",
        ScVal::Address(_) => "address",
        ScVal::LedgerKeyContractInstance => "ledger_key_contract_instance",
        ScVal::LedgerKeyNonce(_) => "ledger_key_nonce",
        ScVal::ContractInstance(_) => "contract_instance",
    }
}

/// Decode stdout emitted by `stellar contract invoke` into a [`DecodedInvokeOutput`].
///
/// Handles:
/// - Empty output / `null` (`ScVal::Void`)
/// - Base64-encoded XDR `ScVal` (or `{"xdr": "<base64>"}` / `{"result": "<base64>"}`)
/// - Canonical JSON `ScVal` or plain JSON values returned by `stellar contract invoke`
/// - Strkey addresses, booleans, integers, and strings
pub fn decode_simulation_or_invoke_output(
    contract: &str,
    function: &str,
    simulated: bool,
    spec_return_type: Option<String>,
    raw_stdout: &str,
) -> Result<DecodedInvokeOutput> {
    let trimmed = raw_stdout.trim();
    let scval = parse_stdout_to_scval(trimmed, spec_return_type.as_deref())?;
    let xdr = encode_scval_xdr_base64(&scval)?;
    let value = scval_to_json(&scval);

    Ok(DecodedInvokeOutput {
        contract: contract.to_owned(),
        function: function.to_owned(),
        simulated,
        scval_type: scval_type_name(&scval).to_owned(),
        value,
        xdr,
        spec_return_type,
    })
}

fn parse_stdout_to_scval(trimmed: &str, spec_return_type: Option<&str>) -> Result<ScVal> {
    if trimmed.is_empty() || trimmed == "null" || trimmed == "()" {
        return Ok(ScVal::Void);
    }

    // 1. Check if stdout is a base64-encoded XDR ScVal directly (e.g. starts with `AAAA`).
    if let Some(rest) = trimmed.strip_prefix("xdr:") {
        return decode_scval_xdr_base64(rest);
    }
    if trimmed.len() >= 8
        && trimmed.len() % 4 == 0
        && !trimmed.starts_with('{')
        && !trimmed.starts_with('[')
        && !trimmed.starts_with('"')
        && trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=')
    {
        if let Ok(scval) = decode_scval_xdr_base64(trimmed) {
            return Ok(scval);
        }
    }

    // 2. Check if stdout is valid JSON.
    if let Ok(json_val) = serde_json::from_str::<Value>(trimmed) {
        if let Some(obj) = json_val.as_object() {
            if let Some(b64) = obj
                .get("returnValueXdr")
                .or_else(|| obj.get("result_xdr"))
                .or_else(|| obj.get("xdr"))
                .and_then(Value::as_str)
            {
                return decode_scval_xdr_base64(b64);
            }
            if let Some(inner) = obj.get("result") {
                if let Some(s) = inner.as_str() {
                    if let Ok(scval) = decode_scval_xdr_base64(s) {
                        return Ok(scval);
                    }
                }
                if let Ok(scval) = scval_from_json(inner) {
                    return Ok(scval);
                }
            }
        }
        if let Ok(scval) = scval_from_json(&json_val) {
            return coerce_decoded_scval_with_hint(scval, spec_return_type);
        }
        if let Value::Object(_) = json_val {
            return parse_map_from_json_value(&json_val);
        }
    }

    // 3. Check if stdout is a Stellar Strkey address, boolean, integer, or symbol/string.
    if (trimmed.starts_with('G') || trimmed.starts_with('C')) && trimmed.len() == 56 {
        if let Ok(addr) = parse_address_scval(trimmed) {
            return Ok(addr);
        }
    }
    if trimmed == "true" {
        return Ok(ScVal::Bool(true));
    }
    if trimmed == "false" {
        return Ok(ScVal::Bool(false));
    }
    if let Ok(i) = trimmed.parse::<i128>() {
        return coerce_integer_with_hint(i, spec_return_type);
    }

    parse_typed_scval(ScValTypeTag::String, trimmed)
}

fn coerce_decoded_scval_with_hint(scval: ScVal, hint: Option<&str>) -> Result<ScVal> {
    match (&scval, hint) {
        (ScVal::I64(n), Some(h)) => coerce_integer_with_hint(i128::from(*n), Some(h)),
        (ScVal::U64(n), Some(h)) => coerce_integer_with_hint(i128::from(*n), Some(h)),
        (ScVal::String(ScString(s)), Some("symbol")) => {
            let text = String::from_utf8_lossy(s.as_slice());
            parse_typed_scval(ScValTypeTag::Symbol, &text)
        }
        (ScVal::String(ScString(s)), Some("address")) => {
            let text = String::from_utf8_lossy(s.as_slice());
            parse_address_scval(&text)
        }
        (ScVal::String(ScString(s)), Some(h)) if h == "i128" || h == "u128" => {
            let text = String::from_utf8_lossy(s.as_slice());
            if h == "u128" {
                parse_typed_scval(ScValTypeTag::U128, &text)
            } else {
                parse_typed_scval(ScValTypeTag::I128, &text)
            }
        }
        _ => Ok(scval),
    }
}

fn coerce_integer_with_hint(val: i128, hint: Option<&str>) -> Result<ScVal> {
    match hint {
        Some("u32") if u32::try_from(val).is_ok() => Ok(ScVal::U32(val as u32)),
        Some("i32") if i32::try_from(val).is_ok() => Ok(ScVal::I32(val as i32)),
        Some("u64") if u64::try_from(val).is_ok() => Ok(ScVal::U64(val as u64)),
        Some("i64") if i64::try_from(val).is_ok() => Ok(ScVal::I64(val as i64)),
        Some("u128") if val >= 0 => Ok(ScVal::from(val as u128)),
        Some("i128") => Ok(ScVal::from(val)),
        _ => {
            if let Ok(i) = i64::try_from(val) {
                Ok(ScVal::I64(i))
            } else {
                Ok(ScVal::from(val))
            }
        }
    }
}

/// Format a [`DecodedInvokeOutput`] according to [`InvokeOutputFormat`].
pub fn format_decoded_output(
    decoded: &DecodedInvokeOutput,
    format: InvokeOutputFormat,
) -> Result<String> {
    match format {
        InvokeOutputFormat::Json => {
            serde_json::to_string_pretty(decoded).context("failed to serialize invoke JSON output")
        }
        InvokeOutputFormat::Xdr => Ok(decoded.xdr.clone()),
        InvokeOutputFormat::Text => {
            let mode = if decoded.simulated {
                "Simulation"
            } else {
                "Invocation"
            };
            let type_suffix = match &decoded.spec_return_type {
                Some(spec_ty) => format!("{} (spec: {spec_ty})", decoded.scval_type),
                None => decoded.scval_type.clone(),
            };
            let value_str = serde_json::to_string(&decoded.value).unwrap_or_default();
            Ok(format!(
                "{mode} result for `{}::{}`:\n  type:  {type_suffix}\n  value: {value_str}\n  xdr:   {}",
                decoded.contract, decoded.function, decoded.xdr
            ))
        }
    }
}

/// Enrich a failed simulation or invocation with decoded `ScError` / contract error context.
pub fn enrich_invoke_error(
    args: &InvokeArgs,
    spec: Option<&ContractSpec>,
    stdout: &str,
    stderr: &str,
) -> anyhow::Error {
    let combined = match (stderr.trim().is_empty(), stdout.trim().is_empty()) {
        (false, false) => format!("{}\n{}", stderr.trim(), stdout.trim()),
        (false, true) => stderr.trim().to_owned(),
        (true, false) => stdout.trim().to_owned(),
        (true, true) => "stellar CLI exited with non-zero status and no output".to_owned(),
    };

    let phase = if args.simulate_only || combined.to_ascii_lowercase().contains("simulat") {
        "simulation"
    } else {
        "invocation"
    };

    let mut details = Vec::new();

    // 1. Check for `Error(Contract, #<code>)` pattern in host diagnostic output.
    if let Some(code) = extract_contract_error_code(&combined) {
        let resolved = spec
            .and_then(|s| s.describe_contract_error(code))
            .or_else(|| describe_forge_error_code(code))
            .unwrap_or_else(|| format!("ContractError(#{code})"));
        details.push(format!("contract error: {resolved}"));
    }

    // 2. Check for host `Error(<Domain>, <Code>)` pattern (e.g. `Error(Auth, InvalidAction)`).
    if let Some((domain, code)) = extract_host_error_domain_and_code(&combined) {
        details.push(format!("host error: {domain}::{code}"));
    }

    // 3. Check if any base64 token in the error output decodes to `ScVal::Error(sc_err)`.
    if details.is_empty() {
        if let Some(sc_err) = find_embedded_xdr_sc_error(&combined) {
            match sc_err {
                ScError::Contract(code) => {
                    let resolved = spec
                        .and_then(|s| s.describe_contract_error(code))
                        .or_else(|| describe_forge_error_code(code))
                        .unwrap_or_else(|| format!("ContractError(#{code})"));
                    details.push(format!("contract error: {resolved}"));
                }
                other => {
                    let (domain, code) = sc_error_domain_and_code(&other);
                    details.push(format!("host error: {domain}::{}", code.name()));
                }
            }
        }
    }

    let context_suffix = if details.is_empty() {
        String::new()
    } else {
        format!(" [{}]", details.join("; "))
    };

    anyhow::anyhow!(
        "contract {phase} failed for `{}::{}` on network `{}`{context_suffix}: {combined}",
        args.contract,
        args.function,
        args.network
    )
}

fn extract_contract_error_code(text: &str) -> Option<u32> {
    let marker = "Error(Contract, #";
    let start = text.find(marker)? + marker.len();
    let digits: String = text[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse::<u32>().ok()
}

fn extract_host_error_domain_and_code(text: &str) -> Option<(String, String)> {
    let mut search_from = 0;
    while let Some(rel) = text[search_from..].find("Error(") {
        let start = search_from + rel + "Error(".len();
        let rest = &text[start..];
        if let Some(end) = rest.find(')') {
            let inside = &rest[..end];
            if let Some((domain, code)) = inside.split_once(',') {
                let domain = domain.trim();
                let code = code.trim();
                if !domain.eq_ignore_ascii_case("Contract") && !code.starts_with('#') {
                    return Some((domain.to_owned(), code.to_owned()));
                }
            }
        }
        search_from = start;
    }
    None
}

fn find_embedded_xdr_sc_error(text: &str) -> Option<ScError> {
    for token in text.split(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',') {
        if token.len() >= 8 && token.len() % 4 == 0 {
            if let Ok(ScVal::Error(err)) = decode_scval_xdr_base64(token) {
                return Some(err);
            }
            if let Ok(bytes) = base64_decode(token) {
                if let Ok(err) = ScError::from_xdr(&bytes, Limits::none()) {
                    return Some(err);
                }
            }
        }
    }
    None
}

/// Map standard `soroban_forge_shared_utils::ForgeError` numeric codes (`1..=10`) to names.
#[must_use]
pub fn describe_forge_error_code(code: u32) -> Option<String> {
    let name = match code {
        1 => "InvalidInput",
        2 => "Unauthorized",
        3 => "NotFound",
        4 => "AlreadyExists",
        5 => "InsufficientFunds",
        6 => "DeadlineExpired",
        7 => "InvalidState",
        8 => "ArithmeticOverflow",
        9 => "TokenTransferFailed",
        10 => "ContractInvocationFailed",
        _ => return None,
    };
    Some(format!("ForgeError::{name} (code #{code})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::InvokeOutputFormat;
    use std::cell::RefCell;
    use stellar_xdr::curr::{
        ScSpecFunctionInputV0, ScSpecTypeBytesN, ScSpecTypeOption, ScSpecTypeVec,
        ScSpecUdtErrorEnumCaseV0,
    };

    const SAMPLE_ACCOUNT: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";

    fn sample_contract_strkey() -> String {
        stellar_strkey::Contract([0xab; 32]).to_string()
    }

    #[derive(Default)]
    struct MockInvokeRunner {
        calls: RefCell<Vec<(String, Vec<String>)>>,
        response: RefCell<Option<InvokeCommandOutput>>,
    }

    impl MockInvokeRunner {
        fn with_stdout(stdout: &str) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                response: RefCell::new(Some(InvokeCommandOutput {
                    success: true,
                    stdout: stdout.to_owned(),
                    stderr: String::new(),
                })),
            }
        }

        fn with_failure(stdout: &str, stderr: &str) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                response: RefCell::new(Some(InvokeCommandOutput {
                    success: false,
                    stdout: stdout.to_owned(),
                    stderr: stderr.to_owned(),
                })),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.borrow().len()
        }

        fn last_args(&self) -> Vec<String> {
            self.calls
                .borrow()
                .last()
                .map(|(_, args)| args.clone())
                .unwrap_or_default()
        }
    }

    impl InvokeRunner for MockInvokeRunner {
        fn invoke(&self, program: &str, args: &[String]) -> Result<InvokeCommandOutput> {
            self.calls
                .borrow_mut()
                .push((program.to_owned(), args.to_vec()));
            Ok(self
                .response
                .borrow()
                .clone()
                .unwrap_or(InvokeCommandOutput {
                    success: true,
                    stdout: String::new(),
                    stderr: String::new(),
                }))
        }
    }

    fn push_leb128_u32(buf: &mut Vec<u8>, mut value: u32) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            buf.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    /// Build a minimal valid WebAssembly binary containing a `contractspecv0` custom section.
    fn build_wasm_with_spec(entries: &[ScSpecEntry]) -> Vec<u8> {
        let mut spec_payload = Vec::new();
        for entry in entries {
            let encoded = entry.to_xdr(Limits::none()).expect("encode ScSpecEntry");
            spec_payload.extend_from_slice(&encoded);
        }

        let section_name = CONTRACT_SPEC_SECTION.as_bytes();
        let mut section_body = Vec::new();
        push_leb128_u32(&mut section_body, section_name.len() as u32);
        section_body.extend_from_slice(section_name);
        section_body.extend_from_slice(&spec_payload);

        let mut wasm = Vec::from(&b"\0asm\x01\x00\x00\x00"[..]);
        wasm.push(0); // custom section ID = 0
        push_leb128_u32(&mut wasm, section_body.len() as u32);
        wasm.extend_from_slice(&section_body);
        wasm
    }

    fn sample_escrow_spec_entries() -> Vec<ScSpecEntry> {
        let create_escrow = ScSpecEntry::FunctionV0(ScSpecFunctionV0 {
            doc: StringM::default(),
            name: ScSymbol("create_escrow".as_bytes().to_vec().try_into().unwrap()),
            inputs: vec![
                ScSpecFunctionInputV0 {
                    doc: StringM::default(),
                    name: "buyer".as_bytes().to_vec().try_into().unwrap(),
                    type_: ScSpecTypeDef::Address,
                },
                ScSpecFunctionInputV0 {
                    doc: StringM::default(),
                    name: "amount".as_bytes().to_vec().try_into().unwrap(),
                    type_: ScSpecTypeDef::I128,
                },
                ScSpecFunctionInputV0 {
                    doc: StringM::default(),
                    name: "deadline".as_bytes().to_vec().try_into().unwrap(),
                    type_: ScSpecTypeDef::U64,
                },
                ScSpecFunctionInputV0 {
                    doc: StringM::default(),
                    name: "auto_release".as_bytes().to_vec().try_into().unwrap(),
                    type_: ScSpecTypeDef::Bool,
                },
                ScSpecFunctionInputV0 {
                    doc: StringM::default(),
                    name: "tag".as_bytes().to_vec().try_into().unwrap(),
                    type_: ScSpecTypeDef::Symbol,
                },
                ScSpecFunctionInputV0 {
                    doc: StringM::default(),
                    name: "receipt_hash".as_bytes().to_vec().try_into().unwrap(),
                    type_: ScSpecTypeDef::BytesN(ScSpecTypeBytesN { n: 4 }),
                },
                ScSpecFunctionInputV0 {
                    doc: StringM::default(),
                    name: "milestones".as_bytes().to_vec().try_into().unwrap(),
                    type_: ScSpecTypeDef::Vec(Box::new(ScSpecTypeVec {
                        element_type: Box::new(ScSpecTypeDef::U32),
                    })),
                },
                ScSpecFunctionInputV0 {
                    doc: StringM::default(),
                    name: "memo".as_bytes().to_vec().try_into().unwrap(),
                    type_: ScSpecTypeDef::Option(Box::new(ScSpecTypeOption {
                        value_type: Box::new(ScSpecTypeDef::String),
                    })),
                },
            ]
            .try_into()
            .unwrap(),
            outputs: vec![ScSpecTypeDef::U64].try_into().unwrap(),
        });

        let error_enum = ScSpecEntry::UdtErrorEnumV0(ScSpecUdtErrorEnumV0 {
            doc: StringM::default(),
            lib: StringM::default(),
            name: "EscrowError".as_bytes().to_vec().try_into().unwrap(),
            cases: vec![
                ScSpecUdtErrorEnumCaseV0 {
                    doc: "Caller is not authorized"
                        .as_bytes()
                        .to_vec()
                        .try_into()
                        .unwrap(),
                    name: "Unauthorized".as_bytes().to_vec().try_into().unwrap(),
                    value: 2,
                },
                ScSpecUdtErrorEnumCaseV0 {
                    doc: "Escrow deadline has expired"
                        .as_bytes()
                        .to_vec()
                        .try_into()
                        .unwrap(),
                    name: "DeadlineExpired".as_bytes().to_vec().try_into().unwrap(),
                    value: 42,
                },
            ]
            .try_into()
            .unwrap(),
        });

        vec![create_escrow, error_enum]
    }

    #[test]
    fn round_trip_every_supported_scval_type_xdr_and_json() {
        let contract_addr = sample_contract_strkey();
        let account_arg = format!("address:{SAMPLE_ACCOUNT}");
        let contract_arg = format!("address:{contract_addr}");
        let cases: Vec<(&str, ScVal)> = vec![
            (
                &account_arg,
                parse_typed_scval(ScValTypeTag::Address, SAMPLE_ACCOUNT).unwrap(),
            ),
            (
                &contract_arg,
                parse_typed_scval(ScValTypeTag::Address, &contract_addr).unwrap(),
            ),
            (
                "i128:-170141183460469231731687303715884105728",
                ScVal::from(i128::MIN),
            ),
            (
                "i128:170141183460469231731687303715884105727",
                ScVal::from(i128::MAX),
            ),
            (
                "u128:340282366920938463463374607431768211455",
                ScVal::from(u128::MAX),
            ),
            ("i64:-9223372036854775808", ScVal::I64(i64::MIN)),
            ("u64:18446744073709551615", ScVal::U64(u64::MAX)),
            ("i32:-2147483648", ScVal::I32(i32::MIN)),
            ("u32:4294967295", ScVal::U32(u32::MAX)),
            ("bool:true", ScVal::Bool(true)),
            ("bool:false", ScVal::Bool(false)),
            (
                "symbol:release_partial",
                parse_typed_scval(ScValTypeTag::Symbol, "release_partial").unwrap(),
            ),
            (
                "string:Soroban Forge Settlement",
                parse_typed_scval(ScValTypeTag::String, "Soroban Forge Settlement").unwrap(),
            ),
            (
                "bytes:0xdeadbeefcafe",
                parse_typed_scval(ScValTypeTag::Bytes, "0xdeadbeefcafe").unwrap(),
            ),
            ("void:", ScVal::Void),
            (
                r#"vec:[{"u32":7},{"symbol":"ok"},{"bool":true}]"#,
                parse_typed_scval(
                    ScValTypeTag::Vec,
                    r#"[{"u32":7},{"symbol":"ok"},{"bool":true}]"#,
                )
                .unwrap(),
            ),
            (
                r#"map:[{"key":{"symbol":"fee_bps"},"val":{"u32":250}},{"key":{"symbol":"active"},"val":{"bool":true}}]"#,
                parse_typed_scval(
                    ScValTypeTag::Map,
                    r#"[{"key":{"symbol":"fee_bps"},"val":{"u32":250}},{"key":{"symbol":"active"},"val":{"bool":true}}]"#,
                )
                .unwrap(),
            ),
            (
                r#"json:{"error":{"type":"contract","code":5}}"#,
                ScVal::Error(ScError::Contract(5)),
            ),
        ];

        for (arg_str, expected_scval) in cases {
            let parsed = parse_invoke_arg(arg_str).expect("parse typed arg");
            let scval = parsed.scval.expect("typed arg produces ScVal");
            assert_eq!(scval, expected_scval, "mismatch for `{arg_str}`");

            // Round-trip via XDR base64
            let b64 = encode_scval_xdr_base64(&scval).expect("encode XDR");
            let from_b64 = decode_scval_xdr_base64(&b64).expect("decode XDR");
            assert_eq!(from_b64, scval, "XDR round-trip failed for `{arg_str}`");

            // Also verify `xdr:<base64>` CLI arg form round-trips
            let via_xdr_arg = parse_invoke_arg(&format!("val:xdr={b64}"))
                .unwrap()
                .scval
                .unwrap();
            assert_eq!(via_xdr_arg, scval);

            // Round-trip via JSON ScVal representation
            let as_json = scval_to_json(&scval);
            let from_json = scval_from_json(&as_json).expect("decode JSON ScVal");
            assert_eq!(from_json, scval, "JSON round-trip failed for `{arg_str}`");

            // Also verify `json:<payload>` CLI arg form round-trips
            let json_str = serde_json::to_string(&as_json).unwrap();
            let via_json_arg = parse_invoke_arg(&format!("val:json={json_str}"))
                .unwrap()
                .scval
                .unwrap();
            assert_eq!(via_json_arg, scval);
        }
    }

    #[test]
    fn supports_both_named_typed_syntaxes_and_preserves_legacy_args() {
        // Syntax A: `<name>:<type>=<value>`
        let a = parse_invoke_arg("amount:i128=250_000").unwrap();
        // Syntax B: `<name>=<type>:<value>`
        let b = parse_invoke_arg("amount=i128:250000").unwrap();
        assert_eq!(a.name.as_deref(), Some("amount"));
        assert_eq!(a.scval, b.scval);
        assert_eq!(a.cli_value(), "250000");

        // Legacy simple `key=value` and positional `value` continue to work
        let simple_kv = parse_invoke_arg("buyer=alice").unwrap();
        assert_eq!(simple_kv.name.as_deref(), Some("buyer"));
        assert_eq!(simple_kv.type_tag, None);
        assert_eq!(simple_kv.cli_value(), "alice");

        let simple_pos = parse_invoke_arg("42").unwrap();
        assert_eq!(simple_pos.name, None);
        assert_eq!(simple_pos.type_tag, None);
        assert_eq!(simple_pos.cli_value(), "42");
    }

    #[test]
    fn invalid_types_and_addresses_fail_before_network_request() {
        let bad_cases = [
            // Invalid account Strkey checksum
            vec![
                "--arg",
                "buyer:address=GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHX",
            ],
            // Invalid address prefix
            vec!["--arg", "buyer:address=not_an_address"],
            // Invalid integer overflow/syntax
            vec!["--arg", "amount:u64=-5"],
            vec!["--arg", "amount:i128=not_a_number"],
            vec!["--arg", "bps:u32=99999999999999"],
            // Invalid bool
            vec!["--arg", "flag:bool=yes"],
            // Invalid symbol (contains hyphen or > 32 chars)
            vec!["--arg", "tag:symbol=invalid-symbol!"],
            vec!["--arg", "tag:symbol=abcdefghijklmnopqrstuvwxyz0123456"],
            // Invalid hex bytes (odd length / non-hex chars)
            vec!["--arg", "hash:bytes=0xabc"],
            vec!["--arg", "hash:bytes=0xzzzz"],
            // Invalid JSON vec / map
            vec!["--arg", "items:vec=not_json"],
            vec!["--arg", "items:vec={\"not\":\"an_array\"}"],
            vec!["--arg", "meta:map=123"],
            // Unknown type tag
            vec!["--arg", "foo:float64=1.23"],
        ];

        for extra in bad_cases {
            let runner = MockInvokeRunner::with_stdout("null");
            let raw_args: Vec<String> = extra
                .chunks_exact(2)
                .map(|pair| pair[1].to_owned())
                .collect();
            let invoke_args = InvokeArgs {
                contract: sample_contract_strkey(),
                function: "deposit".to_owned(),
                args: raw_args,
                wasm: None,
                simulate_only: false,
                output: InvokeOutputFormat::Text,
                json: false,
                network: "testnet".to_owned(),
                source: None,
            };

            let err = execute_with_runner(&invoke_args, &runner).unwrap_err();
            assert_eq!(
                runner.call_count(),
                0,
                "network runner must NOT be called when input validation fails (error was: {err:#})"
            );
        }

        // Also verify that passing a G... account address as `--contract` fails pre-network
        let runner = MockInvokeRunner::with_stdout("null");
        let bad_contract_args = InvokeArgs {
            contract: SAMPLE_ACCOUNT.to_owned(),
            function: "deposit".to_owned(),
            args: Vec::new(),
            wasm: None,
            simulate_only: false,
            output: InvokeOutputFormat::Text,
            json: false,
            network: "testnet".to_owned(),
            source: None,
        };
        let err = execute_with_runner(&bad_contract_args, &runner).unwrap_err();
        assert!(err.to_string().contains("account Strkey"));
        assert_eq!(runner.call_count(), 0);
    }

    #[test]
    fn contract_spec_validation_checks_wasm_custom_section_and_coerces_args() {
        let wasm_bytes = build_wasm_with_spec(&sample_escrow_spec_entries());
        let temp = tempfile::NamedTempFile::new().unwrap();
        fs::write(temp.path(), &wasm_bytes).unwrap();

        let return_xdr = encode_scval_xdr_base64(&ScVal::U64(7)).unwrap();
        let runner = MockInvokeRunner::with_stdout(&return_xdr);

        // Supply arguments out of order and mix typed + untyped values to verify spec coercion & reordering!
        let invoke_args = InvokeArgs {
            contract: sample_contract_strkey(),
            function: "create_escrow".to_owned(),
            args: vec![
                "deadline=3600".to_owned(),
                format!("buyer:address={SAMPLE_ACCOUNT}"),
                "amount:i128=50_000".to_owned(),
                "auto_release=true".to_owned(),
                "tag=escrow_v1".to_owned(),
                "receipt_hash:bytes=0x01020304".to_owned(),
                r#"milestones:vec=[{"u32":1000},{"u32":2000}]"#.to_owned(),
                "memo=none".to_owned(),
            ],
            wasm: Some(temp.path().to_path_buf()),
            simulate_only: true,
            output: InvokeOutputFormat::Json,
            json: false,
            network: "testnet".to_owned(),
            source: Some("alice".to_owned()),
        };

        let rendered = execute_with_runner(&invoke_args, &runner).unwrap();
        assert_eq!(runner.call_count(), 1);

        // Verify arguments were reordered to match the contract spec parameter order
        let sent = runner.last_args();
        assert!(sent.contains(&"--send=no".to_owned()));
        let dash_pos = sent.iter().position(|s| s == "--").unwrap();
        assert_eq!(
            &sent[dash_pos + 1..],
            &[
                "create_escrow",
                "--buyer",
                SAMPLE_ACCOUNT,
                "--amount",
                "50000",
                "--deadline",
                "3600",
                "--auto_release",
                "true",
                "--tag",
                "escrow_v1",
                "--receipt_hash",
                "01020304",
                "--milestones",
                "[1000,2000]",
                "--memo",
                "null",
            ]
        );

        // Verify decoded JSON output contains the u64 result, base64 XDR, and spec return type
        let parsed_out: DecodedInvokeOutput = serde_json::from_str(&rendered).unwrap();
        assert!(parsed_out.simulated);
        assert_eq!(parsed_out.scval_type, "u64");
        assert_eq!(parsed_out.value, json!({ "u64": 7 }));
        assert_eq!(parsed_out.xdr, return_xdr);
        assert_eq!(parsed_out.spec_return_type.as_deref(), Some("u64"));
    }

    #[test]
    fn contract_spec_rejects_unknown_function_wrong_type_and_bad_bytes_n_before_network() {
        let wasm_bytes = build_wasm_with_spec(&sample_escrow_spec_entries());
        let temp = tempfile::NamedTempFile::new().unwrap();
        fs::write(temp.path(), &wasm_bytes).unwrap();

        // 1. Unknown function name
        let runner = MockInvokeRunner::with_stdout("null");
        let unknown_fn = InvokeArgs {
            contract: sample_contract_strkey(),
            function: "non_existent".to_owned(),
            args: Vec::new(),
            wasm: Some(temp.path().to_path_buf()),
            simulate_only: false,
            output: InvokeOutputFormat::Text,
            json: false,
            network: "testnet".to_owned(),
            source: None,
        };
        let err = execute_with_runner(&unknown_fn, &runner).unwrap_err();
        assert!(err.to_string().contains("not found in contract spec"));
        assert!(err.to_string().contains("create_escrow"));
        assert_eq!(runner.call_count(), 0);

        // 2. Wrong argument type (passing `u64` where `i128` is expected, or 2 bytes for `BytesN<4>`)
        let wrong_bytes_n = InvokeArgs {
            contract: sample_contract_strkey(),
            function: "create_escrow".to_owned(),
            args: vec![
                format!("buyer:address={SAMPLE_ACCOUNT}"),
                "amount:i128=100".to_owned(),
                "deadline:u64=60".to_owned(),
                "auto_release:bool=false".to_owned(),
                "tag:symbol=deal".to_owned(),
                "receipt_hash:bytes=0x0102".to_owned(), // 2 bytes instead of 4
                r#"milestones:vec=[{"u32":1}]"#.to_owned(),
                "memo:void=".to_owned(),
            ],
            wasm: Some(temp.path().to_path_buf()),
            simulate_only: false,
            output: InvokeOutputFormat::Text,
            json: false,
            network: "testnet".to_owned(),
            source: None,
        };
        let err = execute_with_runner(&wrong_bytes_n, &runner).unwrap_err();
        let chain = format!("{err:#}");
        assert!(chain.contains("BytesN<4> requires exactly 4 bytes"));
        assert_eq!(runner.call_count(), 0);
    }

    #[test]
    fn simulation_and_contract_errors_retain_actionable_context() {
        let wasm_bytes = build_wasm_with_spec(&sample_escrow_spec_entries());
        let temp = tempfile::NamedTempFile::new().unwrap();
        fs::write(temp.path(), &wasm_bytes).unwrap();

        // 1. Contract error resolved via WASM `ScSpecUdtErrorEnumV0`
        let runner = MockInvokeRunner::with_failure(
            "",
            "error: transaction simulation failed: HostError: Error(Contract, #42)",
        );
        let args = InvokeArgs {
            contract: sample_contract_strkey(),
            function: "create_escrow".to_owned(),
            args: vec![
                format!("buyer:address={SAMPLE_ACCOUNT}"),
                "amount:i128=100".to_owned(),
                "deadline:u64=60".to_owned(),
                "auto_release:bool=false".to_owned(),
                "tag:symbol=deal".to_owned(),
                "receipt_hash:bytes=0x01020304".to_owned(),
                r#"milestones:vec=[{"u32":1}]"#.to_owned(),
                "memo:void=".to_owned(),
            ],
            wasm: Some(temp.path().to_path_buf()),
            simulate_only: true,
            output: InvokeOutputFormat::Text,
            json: false,
            network: "testnet".to_owned(),
            source: None,
        };
        let err = execute_with_runner(&args, &runner).unwrap_err().to_string();
        assert!(err.contains("contract simulation failed"));
        assert!(err.contains("DeadlineExpired (code #42): Escrow deadline has expired"));

        // 2. Host Auth error + fallback ForgeError (#2 = Unauthorized) without WASM spec
        let runner_auth = MockInvokeRunner::with_failure(
            "",
            "HostError: Error(Auth, InvalidAction) / Error(Contract, #2)",
        );
        let args_no_spec = InvokeArgs {
            wasm: None,
            simulate_only: false,
            ..args
        };
        let err_auth = execute_with_runner(&args_no_spec, &runner_auth)
            .unwrap_err()
            .to_string();
        assert!(err_auth.contains("ForgeError::Unauthorized (code #2)"));
        assert!(err_auth.contains("host error: Auth::InvalidAction"));
    }

    #[test]
    fn live_network_invoke_smoke_when_opted_in() {
        if std::env::var("SOROBAN_FORGE_LIVE_NETWORK_TESTS").as_deref() != Ok("1") {
            return;
        }
        let contract = std::env::var("SOROBAN_FORGE_TEST_CONTRACT_ID").expect(
            "SOROBAN_FORGE_TEST_CONTRACT_ID must be set when SOROBAN_FORGE_LIVE_NETWORK_TESTS=1",
        );
        let function = std::env::var("SOROBAN_FORGE_TEST_FUNCTION")
            .unwrap_or_else(|_| "get_tx_count".to_owned());
        let args = InvokeArgs {
            contract,
            function,
            args: Vec::new(),
            wasm: None,
            simulate_only: true,
            output: InvokeOutputFormat::Json,
            json: true,
            network: "testnet".to_owned(),
            source: None,
        };
        let out = execute_with_runner(&args, &SystemInvokeRunner)
            .expect("live testnet simulation should succeed");
        let parsed: DecodedInvokeOutput = serde_json::from_str(&out).unwrap();
        assert!(parsed.simulated);
    }
}
