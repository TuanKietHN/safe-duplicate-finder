use dedupe_core::model::TransactionState;

const ALL_STATES: [TransactionState; 13] = [
    TransactionState::Planned,
    TransactionState::PreflightValidated,
    TransactionState::Moving,
    TransactionState::MovedUnverified,
    TransactionState::Verified,
    TransactionState::PreflightFailed,
    TransactionState::MoveFailed,
    TransactionState::VerifyFailed,
    TransactionState::RecoveryRequired,
    TransactionState::Cancelled,
    TransactionState::ReconciledSourceOnly,
    TransactionState::ReconciledBoth,
    TransactionState::ReconciledMissing,
];

#[test]
fn every_state_rejects_self_loops_and_terminal_history_rewrites() {
    let terminal = [
        TransactionState::Verified,
        TransactionState::PreflightFailed,
        TransactionState::Cancelled,
        TransactionState::ReconciledSourceOnly,
        TransactionState::ReconciledBoth,
        TransactionState::ReconciledMissing,
    ];

    for state in ALL_STATES {
        assert!(
            !state.can_transition_to(state),
            "self-loop accepted for {state:?}"
        );
    }
    for state in terminal {
        for next in ALL_STATES {
            assert!(
                !state.can_transition_to(next),
                "terminal state {state:?} rewrote history to {next:?}"
            );
        }
    }
}

#[test]
fn verified_is_reachable_only_after_move_or_explicit_recovery() {
    for state in ALL_STATES {
        let allowed = matches!(
            state,
            TransactionState::MovedUnverified | TransactionState::RecoveryRequired
        );
        assert_eq!(
            state.can_transition_to(TransactionState::Verified),
            allowed,
            "unexpected verified predecessor {state:?}"
        );
    }
}
