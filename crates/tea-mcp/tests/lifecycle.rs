#![forbid(unsafe_code)]

use tea_mcp::McpServerState;

#[test]
fn lifecycle_transition_table_is_closed_and_explicit() {
    use McpServerState::{Configured, Ready, Reconnecting, Stale, Starting, Stopped, Unhealthy};

    let states = [
        Configured,
        Starting,
        Ready,
        Stale,
        Unhealthy,
        Reconnecting,
        Stopped,
    ];
    let valid = [
        (Configured, Starting),
        (Configured, Stopped),
        (Starting, Ready),
        (Starting, Unhealthy),
        (Starting, Stopped),
        (Ready, Stale),
        (Ready, Unhealthy),
        (Ready, Stopped),
        (Stale, Reconnecting),
        (Stale, Stopped),
        (Unhealthy, Reconnecting),
        (Unhealthy, Stopped),
        (Reconnecting, Ready),
        (Reconnecting, Unhealthy),
        (Reconnecting, Stopped),
    ];

    for from in states {
        for to in states {
            assert_eq!(
                from.can_transition_to(to),
                valid.contains(&(from, to)),
                "unexpected lifecycle edge {from:?} -> {to:?}"
            );
        }
    }
}

#[test]
fn lifecycle_scenarios_map_to_the_frozen_edges() {
    use McpServerState::{Configured, Ready, Reconnecting, Stale, Starting, Stopped, Unhealthy};

    let scenarios = [
        ("startup begins", Configured, Starting),
        ("startup succeeds", Starting, Ready),
        ("startup fails", Starting, Unhealthy),
        ("server EOF", Ready, Unhealthy),
        ("list changed", Ready, Stale),
        ("reconnect begins after stale", Stale, Reconnecting),
        ("reconnect begins after failure", Unhealthy, Reconnecting),
        ("reconnect matches", Reconnecting, Ready),
        ("reconnect mismatches", Reconnecting, Unhealthy),
        ("configured shutdown", Configured, Stopped),
        ("starting drop", Starting, Stopped),
        ("ready shutdown", Ready, Stopped),
        ("stale shutdown", Stale, Stopped),
        ("unhealthy shutdown", Unhealthy, Stopped),
        ("reconnecting drop", Reconnecting, Stopped),
    ];

    for (scenario, from, to) in scenarios {
        assert!(from.can_transition_to(to), "{scenario}");
    }

    assert!(!Stopped.can_transition_to(Configured));
    assert_eq!(McpServerState::fresh(), Configured);
}
