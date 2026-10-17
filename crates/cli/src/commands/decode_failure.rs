use crate::cli::DecodeFailureArgs;
use anyhow::{bail, Context, Result};
use stellar_xdr::curr::{
    ContractEventBody, DiagnosticEvents, InvokeHostFunctionResult, Limits, OperationResult,
    OperationResultTr, ReadXdr, ScError, ScVal, TransactionResult, TransactionResultResult,
};

pub fn run(args: DecodeFailureArgs) -> Result<()> {
    let explanation = if let Some(error) = args.error {
        if error.trim().is_empty() {
            bail!("--error must contain non-whitespace error text");
        }
        explain_error_text(&error)
    } else if let Some(result_xdr) = args.result_xdr {
        decode_transaction_result(&result_xdr, args.diagnostic_events_xdr.as_deref())?
    } else {
        bail!("provide either --error or --result-xdr");
    };

    println!("{explanation}");
    Ok(())
}

fn explain_error_text(error: &str) -> String {
    if let Some((kind, code)) = parse_host_error(error) {
        if kind == "Contract" {
            if let Ok(code) = code.trim_start_matches('#').parse::<u32>() {
                return contract_error_explanation(code);
            }
        }

        return format!(
            "Soroban host abort ({kind}, {code}). Inspect the transaction's diagnostic events \
             for the failing contract call and its cause."
        );
    }

    let lower = error.to_ascii_lowercase();
    if [
        "connection refused",
        "could not connect",
        "failed to connect",
        "error sending request",
        "request timed out",
        "timed out",
        "dns",
        "network is unreachable",
        "failed to fetch",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return format!(
            "RPC/transport failure: {error}. Check the configured RPC endpoint, network \
             connectivity, and service availability."
        );
    }

    format!(
        "Invocation or CLI failure could not be classified as a Soroban host error or \
         recognizable transport failure: {error}. Preserve the complete error output and \
         inspect the RPC/CLI context."
    )
}

fn parse_host_error(error: &str) -> Option<(&str, &str)> {
    let (_, after_prefix) = error.split_once("Error(")?;
    let (fields, _) = after_prefix.split_once(')')?;
    let (kind, code) = fields.split_once(',')?;
    let kind = kind.trim();
    let code = code.trim();
    if kind.is_empty() || code.is_empty() {
        return None;
    }
    Some((kind, code))
}

fn decode_transaction_result(encoded: &str, diagnostics: Option<&str>) -> Result<String> {
    let result = TransactionResult::from_xdr_base64(encoded.trim(), Limits::none())
        .context("invalid --result-xdr: expected base64-encoded Stellar TransactionResult XDR")?;

    if matches!(
        &result.result,
        TransactionResultResult::TxSuccess(_) | TransactionResultResult::TxFeeBumpInnerSuccess(_)
    ) {
        bail!(
            "--result-xdr contains a successful transaction result ({}); provide a failed result",
            result.result.name()
        );
    }

    let diagnostic_detail = diagnostics
        .map(decode_diagnostic_events)
        .transpose()?
        .flatten();
    let prefix = format!(
        "Transaction failed: {} (fee charged: {} stroops).",
        result.result.name(),
        result.fee_charged
    );

    let Some(host_failure) = failed_host_function(&result.result) else {
        return Ok(format!(
            "{prefix} This is a transaction-level failure; check the transaction result code \
             and operation results for the applicable cause."
        ));
    };

    match host_failure {
        InvokeHostFunctionResult::Trapped => {
            if let Some(detail) = diagnostic_detail {
                Ok(format!("{prefix} Soroban host execution trapped. {detail}"))
            } else {
                Ok(format!(
                    "{prefix} Soroban host execution trapped. Supply \
                     --diagnostic-events-xdr from the receipt to identify the contract or host \
                     error."
                ))
            }
        }
        InvokeHostFunctionResult::Malformed => Ok(format!(
            "{prefix} The Soroban invocation was malformed. Check the contract ID, function \
             arguments, and their XDR types."
        )),
        InvokeHostFunctionResult::ResourceLimitExceeded => Ok(format!(
            "{prefix} Soroban exceeded a resource limit. Inspect the simulation's instruction \
             budget, memory, footprint, and transaction resource fee."
        )),
        InvokeHostFunctionResult::EntryArchived => Ok(format!(
            "{prefix} The invocation touched archived ledger data. Restore the required \
             footprint entries and retry."
        )),
        InvokeHostFunctionResult::InsufficientRefundableFee => Ok(format!(
            "{prefix} The refundable resource fee was insufficient. Re-simulate and use the \
             returned resource fee."
        )),
        InvokeHostFunctionResult::Success(_) => Ok(format!(
            "{prefix} The invoke-host-function operation itself succeeded; inspect the other \
             failed operation result."
        )),
    }
}

fn failed_host_function(result: &TransactionResultResult) -> Option<&InvokeHostFunctionResult> {
    let operations = match result {
        TransactionResultResult::TxFailed(operations) => operations,
        TransactionResultResult::TxFeeBumpInnerFailed(inner) => match &inner.result.result {
            stellar_xdr::curr::InnerTransactionResultResult::TxFailed(operations) => operations,
            _ => return None,
        },
        _ => return None,
    };

    operations.iter().find_map(|operation| match operation {
        OperationResult::OpInner(OperationResultTr::InvokeHostFunction(result))
            if !matches!(result, InvokeHostFunctionResult::Success(_)) =>
        {
            Some(result)
        }
        _ => None,
    })
}

fn decode_diagnostic_events(encoded: &str) -> Result<Option<String>> {
    let events = DiagnosticEvents::from_xdr_base64(encoded.trim(), Limits::none())
        .context("invalid --diagnostic-events-xdr: expected base64-encoded DiagnosticEvents XDR")?;

    for event in events.0.iter().filter(|event| {
        matches!(&event.event.body, ContractEventBody::V0(_))
            && event.event.type_ == stellar_xdr::curr::ContractEventType::Diagnostic
    }) {
        let ContractEventBody::V0(body) = &event.event.body;
        for value in body.topics.iter().chain(std::iter::once(&body.data)) {
            if let ScVal::Error(error) = value {
                return Ok(Some(explain_sc_error(error)));
            }
        }
    }

    Ok(None)
}

fn explain_sc_error(error: &ScError) -> String {
    match error {
        ScError::Contract(code) => contract_error_explanation(*code),
        ScError::WasmVm(code)
        | ScError::Context(code)
        | ScError::Storage(code)
        | ScError::Object(code)
        | ScError::Crypto(code)
        | ScError::Events(code)
        | ScError::Budget(code)
        | ScError::Value(code)
        | ScError::Auth(code) => format!(
            "Soroban host error ({}, {:?}). Inspect the contract invocation and diagnostic \
             events for the failing host operation.",
            error.name(),
            code
        ),
    }
}

fn contract_error_explanation(code: u32) -> String {
    let name = match code {
        1 => Some("Unauthorized"),
        2 => Some("NotFound"),
        3 => Some("InvalidInput"),
        4 => Some("InsufficientFunds"),
        5 => Some("AlreadyInitialized"),
        6 => Some("NotInitialized"),
        7 => Some("DeadlineReached"),
        8 => Some("InsufficientAllowance"),
        9 => Some("ArithmeticOverflow"),
        10 => Some("Custom"),
        11 => Some("TokenTransferFailed"),
        12 => Some("ContractInvocationFailed"),
        13 => Some("WithdrawalLimitExceeded"),
        14 => Some("SubscriptionPastDue"),
        15 => Some("ProposerCooldown"),
        _ => None,
    };

    match name {
        Some(name) => format!(
            "Soroban contract error #{code}: ForgeError::{name}. Check the contract's \
             preconditions and authorization requirements."
        ),
        None => format!(
            "Soroban contract error #{code}. The code is not in the shared ForgeError range; \
             check the invoked contract's error enum."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_diagnostic_events, decode_transaction_result, explain_error_text, explain_sc_error,
    };
    use stellar_xdr::curr::{
        ContractEvent, ContractEventBody, ContractEventType, ContractEventV0, DiagnosticEvent,
        DiagnosticEvents, ExtensionPoint, InvokeHostFunctionResult, Limits, OperationResult,
        OperationResultTr, ScError, ScVal, TransactionResult, TransactionResultExt,
        TransactionResultResult, VecM, WriteXdr,
    };

    fn encode<T: WriteXdr>(value: &T) -> String {
        value
            .to_xdr_base64(Limits::none())
            .expect("XDR encoding succeeds")
    }

    #[test]
    fn decodes_shared_forge_contract_error_from_cli_output() {
        let explanation = explain_error_text("HostError: Error(Contract, #3)");
        assert!(explanation.contains("contract error #3"));
        assert!(explanation.contains("ForgeError::InvalidInput"));
    }

    #[test]
    fn explains_host_abort_from_cli_output() {
        let explanation = explain_error_text("HostError: Error(WasmVm, InvalidAction)");
        assert!(explanation.contains("Soroban host abort"));
        assert!(explanation.contains("WasmVm"));
        assert!(explanation.contains("diagnostic events"));
    }

    #[test]
    fn distinguishes_transport_failures() {
        let explanation =
            explain_error_text("error sending request: connection refused by RPC endpoint");
        assert!(explanation.contains("RPC/transport failure"));
        assert!(explanation.contains("configured RPC endpoint"));
    }

    #[test]
    fn leaves_unknown_error_text_actionable_without_panicking() {
        let explanation = explain_error_text("stellar returned an unrecognized failure");
        assert!(explanation.contains("could not be classified"));
        assert!(explanation.contains("unrecognized failure"));
    }

    #[test]
    fn decodes_failed_transaction_host_abort_and_diagnostic_contract_error() {
        let operations = VecM::try_from(vec![OperationResult::OpInner(
            OperationResultTr::InvokeHostFunction(InvokeHostFunctionResult::Trapped),
        )])
        .unwrap();
        let transaction = TransactionResult {
            fee_charged: 120,
            result: TransactionResultResult::TxFailed(operations),
            ext: TransactionResultExt::V0,
        };
        let diagnostics = DiagnosticEvents(
            VecM::try_from(vec![DiagnosticEvent {
                in_successful_contract_call: false,
                event: ContractEvent {
                    ext: ExtensionPoint::V0,
                    contract_id: None,
                    type_: ContractEventType::Diagnostic,
                    body: ContractEventBody::V0(ContractEventV0 {
                        topics: VecM::default(),
                        data: ScVal::Error(ScError::Contract(3)),
                    }),
                },
            }])
            .unwrap(),
        );

        let explanation =
            decode_transaction_result(&encode(&transaction), Some(&encode(&diagnostics))).unwrap();
        assert!(explanation.contains("TxFailed"));
        assert!(explanation.contains("120 stroops"));
        assert!(explanation.contains("Soroban host execution trapped"));
        assert!(explanation.contains("ForgeError::InvalidInput"));
    }

    #[test]
    fn rejects_malformed_result_xdr_with_context() {
        let error = decode_transaction_result("not-xdr", None).unwrap_err();
        assert!(error.to_string().contains("invalid --result-xdr"));
    }

    #[test]
    fn diagnostic_decoder_rejects_malformed_xdr_with_context() {
        let error = decode_diagnostic_events("not-xdr").unwrap_err();
        assert!(error
            .to_string()
            .contains("invalid --diagnostic-events-xdr"));
    }

    #[test]
    fn preserves_unknown_contract_error_codes() {
        let explanation = explain_sc_error(&ScError::Contract(77));
        assert!(explanation.contains("contract error #77"));
        assert!(explanation.contains("not in the shared ForgeError range"));
    }
}
