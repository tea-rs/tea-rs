use std::str::FromStr;

use serde_json::json;
use tea_protocol::{CurrencyCode, DecimalAmount, ExactCost, MAX_SAFE_INTEGER, TokenCount, Usage};

#[test]
fn token_counts_are_json_numbers_bounded_for_javascript() {
    let maximum = TokenCount::new(MAX_SAFE_INTEGER).unwrap();
    assert_eq!(maximum.get(), MAX_SAFE_INTEGER);
    assert_eq!(
        serde_json::to_value(maximum).unwrap(),
        json!(MAX_SAFE_INTEGER)
    );
    assert!(TokenCount::new(MAX_SAFE_INTEGER + 1).is_err());
    assert!(serde_json::from_value::<TokenCount>(json!(MAX_SAFE_INTEGER + 1)).is_err());
}

#[test]
fn usage_serializes_with_optional_breakdowns_and_computed_total() {
    let usage = Usage::new(
        TokenCount::new(1200).unwrap(),
        TokenCount::new(340).unwrap(),
    )
    .with_cache_read(TokenCount::new(100).unwrap())
    .with_cache_write(TokenCount::new(20).unwrap())
    .with_reasoning(TokenCount::new(40).unwrap())
    .unwrap();

    assert_eq!(usage.total_tokens().unwrap().get(), 1660);
    assert_eq!(
        serde_json::to_value(&usage).unwrap(),
        json!({
            "inputTokens":1200,
            "outputTokens":340,
            "cacheReadTokens":100,
            "cacheWriteTokens":20,
            "reasoningTokens":40
        })
    );
    assert_eq!(
        serde_json::from_value::<Usage>(serde_json::to_value(&usage).unwrap()).unwrap(),
        usage
    );
}

#[test]
fn reasoning_tokens_must_be_a_subset_of_output_tokens() {
    let usage = Usage::new(TokenCount::new(10).unwrap(), TokenCount::new(5).unwrap());
    assert!(usage.with_reasoning(TokenCount::new(6).unwrap()).is_err());
    assert!(
        serde_json::from_value::<Usage>(json!({
            "inputTokens":10,
            "outputTokens":5,
            "reasoningTokens":6
        }))
        .is_err()
    );
}

#[test]
fn exact_cost_uses_canonical_decimal_text_and_explicit_unit() {
    let cost = ExactCost::new(
        DecimalAmount::from_str("0.0042").unwrap(),
        CurrencyCode::from_str("USD").unwrap(),
    );
    assert_eq!(
        serde_json::to_value(&cost).unwrap(),
        json!({"amount":"0.0042","currency":"USD","unit":"major_currency"})
    );
    assert_eq!(
        serde_json::from_value::<ExactCost>(serde_json::to_value(&cost).unwrap()).unwrap(),
        cost
    );
}

#[test]
fn malformed_or_noncanonical_cost_values_are_rejected() {
    for value in ["", "-1", "+1", "01", ".1", "1.", "1.20", "1e-3"] {
        assert!(
            DecimalAmount::from_str(value).is_err(),
            "accepted {value:?}"
        );
    }
    for currency in ["", "usd", "US", "USDD", "U1D"] {
        assert!(
            CurrencyCode::from_str(currency).is_err(),
            "accepted {currency:?}"
        );
    }
}
