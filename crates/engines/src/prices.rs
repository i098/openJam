//! Model prices in US dollars per million tokens, as `(input, output)`. Public list prices.
//!
//! shortcut: search tool fees are not priced, only tokens. Add a per-search fee when cost reports
//! must match a provider invoice.

const PRICES: &[(&str, f64, f64)] = &[
    // The longest prefix wins, so list `gpt-5-mini` before `gpt-5`.
    ("gpt-5-mini", 0.25, 2.0),
    ("gpt-5-nano", 0.05, 0.40),
    ("gpt-5", 1.25, 10.0),
    ("gpt-4.1-mini", 0.40, 1.60),
    ("gpt-4.1", 2.0, 8.0),
    ("gpt-4o-mini", 0.15, 0.60),
    ("gpt-4o", 2.5, 10.0),
    ("claude-haiku-4-5", 1.0, 5.0),
    ("claude-sonnet-4-5", 3.0, 15.0),
    ("claude-opus-4", 15.0, 75.0),
    ("sonar-pro", 3.0, 15.0),
    ("sonar", 1.0, 1.0),
    ("gemini-2.5-flash-lite", 0.10, 0.40),
    ("gemini-2.5-flash", 0.30, 2.50),
    ("gemini-2.5-pro", 1.25, 10.0),
];

/// Cost in cents, or `None` when the model has no price.
pub fn cost_cents(model: &str, tokens_in: i32, tokens_out: i32) -> Option<f64> {
    let (_, price_in, price_out) = PRICES
        .iter()
        .filter(|(prefix, ..)| model.starts_with(prefix))
        .max_by_key(|(prefix, ..)| prefix.len())?;
    // dollars per million tokens -> cents per token is price / 10_000
    Some((f64::from(tokens_in) * price_in + f64::from(tokens_out) * price_out) / 10_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_longest_prefix_wins_and_unknown_models_have_no_price() {
        // 1M in + 1M out of gpt-5-mini = $2.25 = 225 cents, not the gpt-5 price.
        assert_eq!(
            cost_cents("gpt-5-mini-2025-08-07", 1_000_000, 1_000_000),
            Some(225.0)
        );
        assert_eq!(cost_cents("mystery-model", 10, 10), None);
    }
}
