//! Structured contract-event introspection for contract tests.
//!
//! `env.events().all()` exposes events as raw XDR. Tests that assert on a
//! lifecycle event usually re-implement the same topic scan and payload
//! decode, and a failed assertion reports only that a boolean was false. This
//! module decodes events once into [`DecodedEvent`] and provides
//! [`assert_contract_event`], whose failure diagnostic lists the topics and
//! payload of every observed event.
//!
//! Every helper here is test-only: the module is compiled only under
//! `cfg(test)` or the `testutils` feature.

use soroban_sdk::testutils::Events as _;
use soroban_sdk::xdr::{ContractEventBody, ScVal};
use soroban_sdk::{Address, Env, IntoVal, Symbol, TryFromVal, Val, Vec};

/// One contract event with its topics and payload decoded to XDR values.
///
/// `topics` preserves the published order, so `topics.first()` is the event
/// name for `#[contractevent]` types (the macro prepends the snake-cased
/// struct name) followed by the fields marked `#[topic]`.
#[derive(Clone, Debug)]
pub struct DecodedEvent {
    /// Topics in publication order.
    pub topics: Vec<ScVal>,
    /// Contract-defined payload.
    pub data: ScVal,
}

impl DecodedEvent {
    /// The event-name topic, or `None` for an event published without topics.
    pub fn first_topic(&self) -> Option<ScVal> {
        self.topics.first()
    }

    /// Whether any topic of this event equals `topic`.
    pub fn has_topic(&self, env: &Env, topic: &Symbol) -> bool {
        let expected = symbol_to_scval(env, topic);
        self.topics.iter().any(|candidate| candidate == expected)
    }
}

/// A scoped view over one contract's events from the last invocation.
///
/// Create one sink per contract when a test asserts on several contracts at
/// once; each sink decodes only its own contract's events.
pub struct MockEventSink<'a> {
    env: &'a Env,
    contract: Address,
}

impl<'a> MockEventSink<'a> {
    /// Observe `contract`'s events for the remainder of the test.
    pub fn new(env: &'a Env, contract: &Address) -> Self {
        Self {
            env,
            contract: contract.clone(),
        }
    }

    /// Decode this contract's events of the last invocation.
    pub fn events(&self) -> std::vec::Vec<DecodedEvent> {
        filter_contract_events(self.env, &self.contract)
    }

    /// Number of events this contract emitted in the last invocation.
    pub fn count(&self) -> usize {
        self.events().len()
    }

    /// Assert on this contract's events; see [`assert_contract_event`].
    #[track_caller]
    pub fn assert_event(&self, expected_topic: &Symbol, validator: impl Fn(&ScVal) -> bool) {
        assert_contract_event(self.env, &self.contract, expected_topic, validator);
    }
}

/// Decode every event emitted by `contract` during the last invocation.
///
/// Events published by other contracts (including nested token calls) are
/// filtered out, so an assertion never depends on how many cross-contract
/// calls a flow happened to make.
pub fn filter_contract_events(env: &Env, contract: &Address) -> std::vec::Vec<DecodedEvent> {
    let all = env.events().all();
    let scoped = all.filter_by_contract(contract);
    let mut decoded = std::vec::Vec::new();
    for event in scoped.events() {
        let ContractEventBody::V0(body) = &event.body;
        let mut topics = Vec::new(env);
        for topic in body.topics.iter() {
            topics.push_back(topic.clone());
        }
        decoded.push(DecodedEvent {
            topics,
            data: body.data.clone(),
        });
    }
    decoded
}

/// Assert that `contract` emitted an event whose topics contain
/// `expected_topic` and whose payload satisfies `validator`.
///
/// If several events carry the topic (for example one per escrow leg), the
/// assertion passes as soon as any payload is accepted. When nothing matches,
/// the panic reports the expected topic, whether any event carried it, and the
/// full topics and payload of every observed event, so a failing test shows
/// the actual event shape instead of only "assertion failed".
#[track_caller]
pub fn assert_contract_event(
    env: &Env,
    contract: &Address,
    expected_topic: &Symbol,
    validator: impl Fn(&ScVal) -> bool,
) {
    let expected = symbol_to_scval(env, expected_topic);
    let events = filter_contract_events(env, contract);
    let mut topic_matched = false;
    for event in &events {
        if !event.topics.iter().any(|candidate| candidate == expected) {
            continue;
        }
        topic_matched = true;
        if validator(&event.data) {
            return;
        }
    }
    panic!("{}", describe_failure(&expected, &events, topic_matched));
}

/// Build the failure diagnostic: the expected topic plus every observed event.
fn describe_failure(
    expected: &ScVal,
    events: &[DecodedEvent],
    topic_matched: bool,
) -> std::string::String {
    let verdict = if topic_matched {
        "topic matched at least one event, but every data validator rejected its payload"
    } else {
        "no event carried the expected topic"
    };
    let mut report = std::format!(
        "contract event assertion failed: expected topic {expected:?}; {verdict}; observed {} event(s)",
        events.len()
    );
    for (index, event) in events.iter().enumerate() {
        report.push_str(&std::format!(
            "\n  [{index}] topics={:?} data={:?}",
            event.topics,
            event.data
        ));
    }
    report
}

/// The XDR form of a symbol, so topics decoded from an event can be compared
/// against a topic declared in the test without re-encoding the event.
fn symbol_to_scval(env: &Env, symbol: &Symbol) -> ScVal {
    let value: Val = symbol.clone().into_val(env);
    match ScVal::try_from_val(env, &value) {
        Ok(scval) => scval,
        // A `Symbol` always has an XDR representation; reaching this branch
        // would mean the SDK's conversion contract changed.
        Err(_) => panic!("symbol could not be converted to ScVal"),
    }
}

#[cfg(test)]
mod tests {
    use super::{assert_contract_event, filter_contract_events, MockEventSink};
    use soroban_sdk::xdr::ScVal;
    use soroban_sdk::{contract, contractevent, contractimpl, symbol_short, Address, Env, Symbol};

    /// Event with a fixed `ping` topic plus one caller-chosen topic.
    #[contractevent(data_format = "single-value")]
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct Ping {
        #[topic]
        pub kind: Symbol,
        pub value: u32,
    }

    #[contract]
    pub struct EventSource;

    #[contractimpl]
    impl EventSource {
        pub fn ping(env: Env, kind: Symbol, value: u32) {
            Ping { kind, value }.publish(&env);
        }

        pub fn silent(_env: Env) {}
    }

    fn source(env: &Env) -> (Address, EventSourceClient<'_>) {
        let address = env.register(EventSource, ());
        let client = EventSourceClient::new(env, &address);
        (address, client)
    }

    #[test]
    fn decodes_topics_and_single_value_payload() {
        let env = Env::default();
        let (address, client) = source(&env);
        client.ping(&symbol_short!("release"), &7);

        let events = filter_contract_events(&env, &address);

        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.topics.len(), 2, "event name plus the #[topic] field");
        assert_eq!(event.data, ScVal::U32(7));
        assert!(event.has_topic(&env, &symbol_short!("ping")));
        assert!(event.has_topic(&env, &symbol_short!("release")));
        assert!(!event.has_topic(&env, &symbol_short!("refund")));
    }

    #[test]
    fn assertion_matches_event_name_topic_and_payload() {
        let env = Env::default();
        let (address, client) = source(&env);
        client.ping(&symbol_short!("release"), &7);

        assert_contract_event(&env, &address, &symbol_short!("ping"), |data| {
            *data == ScVal::U32(7)
        });
    }

    #[test]
    fn assertion_matches_field_topic_and_payload() {
        let env = Env::default();
        let (address, client) = source(&env);
        client.ping(&symbol_short!("dispute"), &3);

        assert_contract_event(&env, &address, &symbol_short!("dispute"), |data| {
            *data == ScVal::U32(3)
        });
    }

    #[test]
    fn filter_isolates_the_requested_contract() {
        let env = Env::default();
        let (first, first_client) = source(&env);
        let (second, second_client) = source(&env);

        first_client.ping(&symbol_short!("release"), &1);
        let first_events = filter_contract_events(&env, &first);
        assert_eq!(first_events.len(), 1);
        assert!(first_events[0].has_topic(&env, &symbol_short!("release")));
        // The other contract emitted nothing in this invocation.
        assert!(filter_contract_events(&env, &second).is_empty());

        second_client.ping(&symbol_short!("refund"), &2);
        let second_events = filter_contract_events(&env, &second);
        assert_eq!(second_events.len(), 1);
        assert!(second_events[0].has_topic(&env, &symbol_short!("refund")));
    }

    #[test]
    fn mock_event_sink_scopes_each_contract() {
        let env = Env::default();
        let (first, first_client) = source(&env);
        let (second, _) = source(&env);
        first_client.ping(&symbol_short!("release"), &5);

        let first_sink = MockEventSink::new(&env, &first);
        let second_sink = MockEventSink::new(&env, &second);

        assert_eq!(first_sink.count(), 1);
        assert_eq!(second_sink.count(), 0);
        first_sink.assert_event(&symbol_short!("release"), |data| *data == ScVal::U32(5));
    }

    #[test]
    fn no_events_for_silent_invocation() {
        let env = Env::default();
        let (address, client) = source(&env);
        client.silent();

        assert!(filter_contract_events(&env, &address).is_empty());
    }

    #[test]
    #[should_panic(expected = "no event carried the expected topic")]
    fn unmatched_topic_reports_the_expected_topic() {
        let env = Env::default();
        let (address, client) = source(&env);
        client.ping(&symbol_short!("release"), &7);

        assert_contract_event(&env, &address, &symbol_short!("refund"), |_| true);
    }

    #[test]
    #[should_panic(expected = "every data validator rejected")]
    fn rejected_payload_reports_the_observed_event() {
        let env = Env::default();
        let (address, client) = source(&env);
        client.ping(&symbol_short!("release"), &7);

        assert_contract_event(&env, &address, &symbol_short!("ping"), |data| {
            *data == ScVal::U32(8)
        });
    }

    #[test]
    #[should_panic(expected = "observed 1 event(s)")]
    fn failing_diagnostic_lists_every_observed_event() {
        let env = Env::default();
        let (address, client) = source(&env);
        client.ping(&symbol_short!("release"), &7);

        assert_contract_event(&env, &address, &symbol_short!("refund"), |_| true);
    }
}
