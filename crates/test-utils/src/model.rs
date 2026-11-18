//! Common bounded state-machine model-runner interface.
//!
//! Provides an independent lifecycle model checker for smart contracts:
//! - [`LifecycleModel`]: defines the abstract specification, valid/invalid transitions,
//!   and domain invariants (conservation, terminal states).
//! - [`ModelDriver`]: executes operations against the Soroban contract environment and
//!   captures on-chain snapshots (balances, statuses, counters).
//! - [`OpGenerator`]: generates deterministic sequences of valid and invalid operations.
//! - [`ModelRng`]: seeded deterministic PRNG for reproducible runs.
//! - [`run_model_check`]: compares actual execution against the model, verifies that
//!   invalid operations cannot mutate on-chain state, validates invariants, and provides
//!   shrinking and replayable trace output upon failure.

extern crate alloc;
extern crate std;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;
use soroban_sdk::Env;

/// Seeded, deterministic PRNG for reproducible test sequences.
#[derive(Clone, Copy, Debug)]
pub struct ModelRng {
    state: u64,
}

impl ModelRng {
    /// Create a new generator with the given seed.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Advance the generator and return the next `u64`.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005_u64)
            .wrapping_add(1_442_695_040_888_963_407_u64);
        self.state
    }

    /// Return a pseudo-random `u32` in `[0, max)`.
    pub fn bounded(&mut self, max: u32) -> u32 {
        if max == 0 {
            return 0;
        }
        (self.next_u64() >> 32) as u32 % max
    }

    /// Return a pseudo-random boolean.
    pub fn gen_bool(&mut self) -> bool {
        (self.next_u64() & 1) == 1
    }

    /// Return a pseudo-random `u64` in `[min, max]`.
    pub fn gen_range_u64(&mut self, min: u64, max: u64) -> u64 {
        if min >= max {
            return min;
        }
        let span = max - min + 1;
        min + (self.next_u64() % span)
    }

    /// Return a pseudo-random `i128` in `[min, max]`.
    pub fn gen_range_i128(&mut self, min: i128, max: i128) -> i128 {
        if min >= max {
            return min;
        }
        let span = (max - min) as u128;
        let r = ((u128::from(self.next_u64())) << 64) | u128::from(self.next_u64());
        min + (r % (span + 1)) as i128
    }

    /// Select an element uniformly from a non-empty slice.
    pub fn choose<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        assert!(!items.is_empty(), "cannot choose from empty slice");
        let idx = self.bounded(items.len() as u32) as usize;
        &items[idx]
    }
}

/// Abstract lifecycle model specification.
pub trait LifecycleModel {
    /// Modeled state.
    type State: Clone + fmt::Debug + PartialEq;
    /// Modeled operation.
    type Op: Clone + fmt::Debug + fmt::Display;
    /// Concrete on-chain state snapshot.
    type Snapshot: Clone + fmt::Debug + PartialEq;

    /// Initial state of the model before operations begin.
    fn initial_state(&self) -> Self::State;

    /// Predict the outcome of applying `op` from `state`.
    ///
    /// - If the operation is valid: returns `Ok(next_state)`.
    /// - If the operation is invalid under the model: returns `Err(rejection_reason)`.
    fn transition(&self, state: &Self::State, op: &Self::Op) -> Result<Self::State, String>;

    /// Verify domain invariants (e.g. conservation of value, terminal properties)
    /// between the model state and the observed on-chain snapshot.
    fn check_invariants(
        &self,
        state: &Self::State,
        snapshot: &Self::Snapshot,
    ) -> Result<(), String>;

    /// Whether the model state represents a terminal lifecycle state.
    fn is_terminal(&self, state: &Self::State) -> bool;
}

/// Driver interface for interacting with the actual Soroban contract.
pub trait ModelDriver<M: LifecycleModel> {
    /// Execute `op` against the on-chain contract in `env`.
    ///
    /// - Returns `Ok(())` if the call succeeded on-chain.
    /// - Returns `Err(error_string)` if the contract rejected the call.
    fn execute_op(&mut self, env: &Env, op: &M::Op) -> Result<(), String>;

    /// Capture a snapshot of on-chain balances, statuses, and counters.
    fn snapshot(&self, env: &Env) -> M::Snapshot;

    /// Reset the contract and accounts to initial state (used for shrinking / replay).
    fn reset(&mut self, env: &Env);
}

/// Generator producing deterministic sequences of operations.
pub trait OpGenerator<M: LifecycleModel> {
    /// Generate the next operation, producing both valid and invalid candidates.
    fn generate_op(&mut self, state: &M::State, rng: &mut ModelRng) -> M::Op;
}

/// Record of a single step executed during model checking.
#[derive(Clone, Debug)]
pub struct ModelStepRecord<Op, State, Snapshot> {
    /// Zero-based step index.
    pub step: usize,
    /// Operation attempted.
    pub op: Op,
    /// Model state prior to operation.
    pub pre_state: State,
    /// Model state after operation (if valid).
    pub post_state: Option<State>,
    /// Whether the model predicted this operation should succeed.
    pub expected_valid: bool,
    /// Reason if predicted invalid.
    pub expected_rejection: Option<String>,
    /// Whether the contract call actually succeeded.
    pub actual_success: bool,
    /// Contract error detail if the call was rejected.
    pub contract_error: Option<String>,
    /// Snapshot before operation.
    pub pre_snapshot: Snapshot,
    /// Snapshot after operation.
    pub post_snapshot: Snapshot,
    /// Violation detail if this step failed an assertion.
    pub violation: Option<String>,
}

/// Trace of an entire model checking execution.
#[derive(Clone, Debug)]
pub struct ModelTrace<Op, State, Snapshot> {
    /// Seed used for this run.
    pub seed: u64,
    /// Total steps configured.
    pub total_steps: usize,
    /// Executed step records.
    pub steps: Vec<ModelStepRecord<Op, State, Snapshot>>,
    /// Failure message if the run failed.
    pub failure: Option<String>,
    /// Minimal shrunk operation sequence if shrinking was performed.
    pub shrunk_ops: Option<Vec<Op>>,
}

impl<Op: fmt::Display + fmt::Debug, State: fmt::Debug, Snapshot: fmt::Debug> fmt::Display
    for ModelTrace<Op, State, Snapshot>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "============================================================"
        )?;
        writeln!(
            f,
            "ModelTrace (seed = {}, steps_executed = {}/{})",
            self.seed,
            self.steps.len(),
            self.total_steps
        )?;
        writeln!(
            f,
            "============================================================"
        )?;
        for step in &self.steps {
            let status = if step.violation.is_none() {
                "OK"
            } else {
                "FAILED"
            };
            let expected_str = if step.expected_valid {
                "Valid"
            } else {
                "Invalid"
            };
            let actual_str = if step.actual_success {
                "Success"
            } else {
                "Rejected"
            };

            writeln!(
                f,
                "[{:>3}] op: {} | expected: {} | actual: {} -> {}",
                step.step, step.op, expected_str, actual_str, status
            )?;
            if let Some(ref rej) = step.expected_rejection {
                writeln!(f, "      model rejection reason: {rej}")?;
            }
            if let Some(ref err) = step.contract_error {
                writeln!(f, "      contract rejection: {err}")?;
            }
            if let Some(ref viol) = step.violation {
                writeln!(f, "      VIOLATION: {viol}")?;
                writeln!(f, "      pre_state:      {:?}", step.pre_state)?;
                writeln!(f, "      pre_snapshot:   {:?}", step.pre_snapshot)?;
                writeln!(f, "      post_snapshot:  {:?}", step.post_snapshot)?;
            }
        }

        if let Some(ref fail) = self.failure {
            writeln!(
                f,
                "------------------------------------------------------------"
            )?;
            writeln!(f, "FAILURE SUMMARY: {fail}")?;
        }

        if let Some(ref shrunk) = self.shrunk_ops {
            writeln!(
                f,
                "------------------------------------------------------------"
            )?;
            writeln!(
                f,
                "MINIMAL REPLAYABLE SHRUNK SEQUENCE ({} ops):",
                shrunk.len()
            )?;
            for (i, op) in shrunk.iter().enumerate() {
                writeln!(f, "  [{i}] {op}")?;
            }
            writeln!(
                f,
                "------------------------------------------------------------"
            )?;
        }

        Ok(())
    }
}

/// Type alias for the trace produced by model checking `M`.
pub type ModelTraceFor<M> = ModelTrace<
    <M as LifecycleModel>::Op,
    <M as LifecycleModel>::State,
    <M as LifecycleModel>::Snapshot,
>;

/// Execute a bounded sequence of operations and verify model compliance.
///
/// # Arguments
/// - `env`: Soroban test environment.
/// - `model`: The independent lifecycle model.
/// - `driver`: Contract adapter under test.
/// - `generator`: Operation generator producing valid and invalid operations.
/// - `seed`: Seed for deterministic execution and reproduction.
/// - `steps`: Number of bounded steps to execute.
///
/// # Returns
/// - `Ok(trace)` if all operations, invariants, and non-mutation rules hold.
/// - `Err(trace)` if any step violates expected model behavior, with a shrunk trace.
#[allow(clippy::type_complexity)]
pub fn run_model_check<M, D, G>(
    env: &Env,
    model: &M,
    driver: &mut D,
    generator: &mut G,
    seed: u64,
    steps: usize,
) -> Result<ModelTraceFor<M>, ModelTraceFor<M>>
where
    M: LifecycleModel,
    D: ModelDriver<M>,
    G: OpGenerator<M>,
{
    driver.reset(env);

    let mut rng = ModelRng::new(seed);
    let mut state = model.initial_state();
    let mut step_records = Vec::with_capacity(steps);
    let mut operations = Vec::with_capacity(steps);

    // Initial invariant check on initial state
    let initial_snapshot = driver.snapshot(env);
    if let Err(err) = model.check_invariants(&state, &initial_snapshot) {
        let trace = ModelTrace {
            seed,
            total_steps: steps,
            steps: vec![],
            failure: Some(format!("Initial invariant check failed: {err}")),
            shrunk_ops: None,
        };
        return Err(trace);
    }

    for step_idx in 0..steps {
        let op = generator.generate_op(&state, &mut rng);
        operations.push(op.clone());

        let pre_state = state.clone();
        let pre_snapshot = driver.snapshot(env);

        let transition_res = model.transition(&state, &op);
        let expected_valid = transition_res.is_ok();
        let expected_rejection = transition_res.as_ref().err().cloned();

        let contract_res = driver.execute_op(env, &op);
        let actual_success = contract_res.is_ok();
        let contract_error = contract_res.as_ref().err().cloned();

        let post_snapshot = driver.snapshot(env);

        let mut violation = None;
        let mut post_state = None;

        match transition_res {
            Ok(next_state) => {
                if !actual_success {
                    violation = Some(format!(
                        "Model predicted op '{op}' is valid, but contract rejected it with: {:?}",
                        contract_error
                    ));
                } else {
                    // Update state and verify invariants
                    state = next_state.clone();
                    post_state = Some(next_state);

                    if let Err(inv_err) = model.check_invariants(&state, &post_snapshot) {
                        violation = Some(format!("Invariant violation after '{op}': {inv_err}"));
                    }
                }
            }
            Err(ref rej_reason) => {
                if actual_success {
                    violation = Some(format!(
                        "Model predicted op '{op}' is INVALID ({rej_reason}), but contract succeeded unexpectedly!"
                    ));
                } else {
                    // Critical requirement: Invalid operations cannot mutate modeled or on-chain state!
                    if post_snapshot != pre_snapshot {
                        violation = Some(format!(
                            "Invalid op '{op}' was rejected, but MUTATED on-chain state!\nPre-snapshot:  {pre_snapshot:?}\nPost-snapshot: {post_snapshot:?}"
                        ));
                    }
                    // Model state remains pre_state
                }
            }
        }

        let failed = violation.is_some();
        step_records.push(ModelStepRecord {
            step: step_idx,
            op,
            pre_state,
            post_state,
            expected_valid,
            expected_rejection,
            actual_success,
            contract_error,
            pre_snapshot,
            post_snapshot,
            violation: violation.clone(),
        });

        if failed {
            let failure_msg = violation.unwrap_or_else(|| "Unknown failure".to_string());
            // Attempt to shrink failing sequence
            let shrunk = shrink_failure(env, model, driver, &operations[..=step_idx]);

            let trace = ModelTrace {
                seed,
                total_steps: steps,
                steps: step_records,
                failure: Some(failure_msg),
                shrunk_ops: Some(shrunk),
            };
            return Err(trace);
        }
    }

    Ok(ModelTrace {
        seed,
        total_steps: steps,
        steps: step_records,
        failure: None,
        shrunk_ops: None,
    })
}

/// Attempt to find a smaller subsequence of operations that still triggers a failure.
fn shrink_failure<M, D>(env: &Env, model: &M, driver: &mut D, failing_ops: &[M::Op]) -> Vec<M::Op>
where
    M: LifecycleModel,
    D: ModelDriver<M>,
{
    let mut current = failing_ops.to_vec();
    if current.len() <= 1 {
        return current;
    }

    // Try delta-debugging: try removing individual operations from the prefix
    let mut changed = true;
    while changed && current.len() > 1 {
        changed = false;
        for i in 0..current.len() - 1 {
            let mut candidate = current.clone();
            candidate.remove(i);

            if sequence_fails(env, model, driver, &candidate) {
                current = candidate;
                changed = true;
                break;
            }
        }
    }

    current
}

/// Check if a specific sequence of operations causes a model check failure.
fn sequence_fails<M, D>(env: &Env, model: &M, driver: &mut D, ops: &[M::Op]) -> bool
where
    M: LifecycleModel,
    D: ModelDriver<M>,
{
    driver.reset(env);
    let mut state = model.initial_state();

    let initial_snap = driver.snapshot(env);
    if model.check_invariants(&state, &initial_snap).is_err() {
        return true;
    }

    for op in ops {
        let pre_snap = driver.snapshot(env);
        let trans = model.transition(&state, op);
        let exec = driver.execute_op(env, op);
        let post_snap = driver.snapshot(env);

        match trans {
            Ok(next_state) => {
                if exec.is_err() {
                    return true;
                }
                state = next_state;
                if model.check_invariants(&state, &post_snap).is_err() {
                    return true;
                }
            }
            Err(_) => {
                if exec.is_ok() {
                    return true;
                }
                if post_snap != pre_snap {
                    return true;
                }
            }
        }
    }

    false
}

/// Replay a specific sequence of operations and verify it passes.
pub fn replay_ops<M, D>(env: &Env, model: &M, driver: &mut D, ops: &[M::Op]) -> Result<(), String>
where
    M: LifecycleModel,
    D: ModelDriver<M>,
{
    driver.reset(env);
    let mut state = model.initial_state();

    let initial_snap = driver.snapshot(env);
    model
        .check_invariants(&state, &initial_snap)
        .map_err(|e| format!("Initial invariant failure: {e}"))?;

    for (step, op) in ops.iter().enumerate() {
        let pre_snap = driver.snapshot(env);
        let trans = model.transition(&state, op);
        let exec = driver.execute_op(env, op);
        let post_snap = driver.snapshot(env);

        match trans {
            Ok(next) => {
                if let Err(err) = exec {
                    return Err(format!(
                        "Replay failed at step {step}: op '{op}' expected valid, rejected with: {err}"
                    ));
                }
                state = next;
                model
                    .check_invariants(&state, &post_snap)
                    .map_err(|e| format!("Replay invariant failed at step {step} ({op}): {e}"))?;
            }
            Err(rej) => {
                if exec.is_ok() {
                    return Err(format!(
                        "Replay failed at step {step}: op '{op}' expected invalid ({rej}), but succeeded"
                    ));
                }
                if post_snap != pre_snap {
                    return Err(format!(
                        "Replay failed at step {step}: invalid op '{op}' mutated on-chain state"
                    ));
                }
            }
        }
    }

    Ok(())
}

/// Assert that the model check passes, printing the full trace on failure.
///
/// # Panics
/// Panics with a formatted trace if the model check fails.
pub fn assert_model_check<M, D, G>(
    env: &Env,
    model: &M,
    driver: &mut D,
    generator: &mut G,
    seed: u64,
    steps: usize,
) where
    M: LifecycleModel,
    D: ModelDriver<M>,
    G: OpGenerator<M>,
{
    if let Err(trace) = run_model_check(env, model, driver, generator, seed, steps) {
        std::panic!("\n\n{}\n\nModel check failed for seed {seed}!\n", trace);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum MockState {
        Inactive,
        Active { counter: u32 },
        Closed,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum MockOp {
        Activate,
        Increment { amount: u32 },
        Close,
        InvalidOp,
    }

    impl fmt::Display for MockOp {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Activate => write!(f, "Activate"),
                Self::Increment { amount } => write!(f, "Increment({amount})"),
                Self::Close => write!(f, "Close"),
                Self::InvalidOp => write!(f, "InvalidOp"),
            }
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct MockSnapshot {
        active: bool,
        counter: u32,
        closed: bool,
    }

    struct MockModel;

    impl LifecycleModel for MockModel {
        type State = MockState;
        type Op = MockOp;
        type Snapshot = MockSnapshot;

        fn initial_state(&self) -> Self::State {
            MockState::Inactive
        }

        fn transition(&self, state: &Self::State, op: &Self::Op) -> Result<Self::State, String> {
            match (state, op) {
                (MockState::Inactive, MockOp::Activate) => Ok(MockState::Active { counter: 0 }),
                (MockState::Active { counter }, MockOp::Increment { amount }) => {
                    if *amount == 0 {
                        Err("amount must be > 0".into())
                    } else {
                        Ok(MockState::Active {
                            counter: counter + amount,
                        })
                    }
                }
                (MockState::Active { .. }, MockOp::Close) => Ok(MockState::Closed),
                _ => Err("invalid operation for state".into()),
            }
        }

        fn check_invariants(
            &self,
            state: &Self::State,
            snapshot: &Self::Snapshot,
        ) -> Result<(), String> {
            match state {
                MockState::Inactive => {
                    if snapshot.active || snapshot.counter != 0 || snapshot.closed {
                        return Err("snapshot does not match Inactive".into());
                    }
                }
                MockState::Active { counter } => {
                    if !snapshot.active || snapshot.counter != *counter || snapshot.closed {
                        return Err("snapshot does not match Active".into());
                    }
                }
                MockState::Closed => {
                    if !snapshot.closed {
                        return Err("snapshot does not match Closed".into());
                    }
                }
            }
            Ok(())
        }

        fn is_terminal(&self, state: &Self::State) -> bool {
            matches!(state, MockState::Closed)
        }
    }

    struct MockDriver {
        snapshot: MockSnapshot,
    }

    impl MockDriver {
        fn new() -> Self {
            Self {
                snapshot: MockSnapshot {
                    active: false,
                    counter: 0,
                    closed: false,
                },
            }
        }
    }

    impl ModelDriver<MockModel> for MockDriver {
        fn execute_op(&mut self, _env: &Env, op: &MockOp) -> Result<(), String> {
            match op {
                MockOp::Activate => {
                    if self.snapshot.active || self.snapshot.closed {
                        Err("already active or closed".into())
                    } else {
                        self.snapshot.active = true;
                        Ok(())
                    }
                }
                MockOp::Increment { amount } => {
                    if !self.snapshot.active || self.snapshot.closed || *amount == 0 {
                        Err("cannot increment".into())
                    } else {
                        self.snapshot.counter += amount;
                        Ok(())
                    }
                }
                MockOp::Close => {
                    if !self.snapshot.active || self.snapshot.closed {
                        Err("cannot close".into())
                    } else {
                        self.snapshot.active = false;
                        self.snapshot.closed = true;
                        Ok(())
                    }
                }
                MockOp::InvalidOp => Err("unrecognized op".into()),
            }
        }

        fn snapshot(&self, _env: &Env) -> MockSnapshot {
            self.snapshot.clone()
        }

        fn reset(&mut self, _env: &Env) {
            self.snapshot = MockSnapshot {
                active: false,
                counter: 0,
                closed: false,
            };
        }
    }

    struct MockGenerator;

    impl OpGenerator<MockModel> for MockGenerator {
        fn generate_op(&mut self, state: &MockState, rng: &mut ModelRng) -> MockOp {
            let choice = rng.bounded(4);
            match (state, choice) {
                (MockState::Inactive, 0) => MockOp::Activate,
                (MockState::Active { .. }, 0 | 1) => MockOp::Increment {
                    amount: rng.bounded(5) + 1,
                },
                (MockState::Active { .. }, 2) => MockOp::Close,
                _ => MockOp::InvalidOp,
            }
        }
    }

    #[test]
    fn test_model_runner_passes_on_compliant_system() {
        let env = Env::default();
        let model = MockModel;
        let mut driver = MockDriver::new();
        let mut generator = MockGenerator;

        assert_model_check(&env, &model, &mut driver, &mut generator, 12345, 20);
    }

    #[test]
    fn test_model_runner_catches_invalid_mutation_and_shrinks() {
        struct MutatingBadDriver {
            snapshot: MockSnapshot,
        }

        impl ModelDriver<MockModel> for MutatingBadDriver {
            fn execute_op(&mut self, _env: &Env, op: &MockOp) -> Result<(), String> {
                if *op == MockOp::InvalidOp {
                    // Rejects the call, BUT mutates the counter (bug!)
                    self.snapshot.counter += 1;
                    return Err("invalid op".into());
                }
                // Delegate normal behavior
                match op {
                    MockOp::Activate => {
                        self.snapshot.active = true;
                        Ok(())
                    }
                    _ => Ok(()),
                }
            }

            fn snapshot(&self, _env: &Env) -> MockSnapshot {
                self.snapshot.clone()
            }

            fn reset(&mut self, _env: &Env) {
                self.snapshot = MockSnapshot {
                    active: false,
                    counter: 0,
                    closed: false,
                };
            }
        }

        struct ForceInvalidGen;
        impl OpGenerator<MockModel> for ForceInvalidGen {
            fn generate_op(&mut self, _state: &MockState, _rng: &mut ModelRng) -> MockOp {
                MockOp::InvalidOp
            }
        }

        let env = Env::default();
        let model = MockModel;
        let mut driver = MutatingBadDriver {
            snapshot: MockSnapshot {
                active: false,
                counter: 0,
                closed: false,
            },
        };
        let mut gen = ForceInvalidGen;

        let res = run_model_check(&env, &model, &mut driver, &mut gen, 999, 5);
        assert!(
            res.is_err(),
            "must detect mutation during invalid operation"
        );
        let trace = res.unwrap_err();
        assert!(trace.shrunk_ops.is_some());
        let trace_str = trace.to_string();
        assert!(trace_str.contains("MUTATED"));
    }
}
