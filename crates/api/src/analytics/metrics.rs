//! Metric math. Rates come from counts, and the functions are pure so boundary cases have tests.
//!
//! - Mention rate: answers that mention the brand, as a percent of the answers.
//! - Citation rate: answers that cite our domain, as a percent of the answers.
//! - Share of voice: our answer mentions, as a percent of all answer mentions (ours and competitors').
//! - Rank position: the mean `position` of the answers that have one. `null` positions are left out.

use serde::Serialize;
use utoipa::ToSchema;

/// Counts for one group of answers in a window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnswerTally {
    pub answers: u64,
    pub mentioned: u64,
    pub recommended: u64,
    pub cited: u64,
    pub position_sum: i64,
    pub position_count: u64,
}

/// A metric in percent (`0` to `100`), or a mean position. The delta compares the window before.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MetricValue {
    /// `null` when the window has no answers to measure.
    pub value: Option<f64>,
    /// The change against the previous window. `null` when a side has no data.
    pub delta: Option<f64>,
    /// False when the current or the previous window has no data.
    pub delta_available: bool,
}

/// Answers that mention the brand, as a percent.
pub fn mention_rate(tally: &AnswerTally) -> Option<f64> {
    percent(tally.mentioned, tally.answers)
}

/// Answers that cite our domain, as a percent.
pub fn citation_rate(tally: &AnswerTally) -> Option<f64> {
    percent(tally.cited, tally.answers)
}

/// Our answer mentions as a percent of all answer mentions.
pub fn share_of_voice(own_mentions: u64, competitor_mentions: u64) -> Option<f64> {
    percent(own_mentions, own_mentions + competitor_mentions)
}

/// The mean position of the answers that have one.
pub fn average_position(tally: &AnswerTally) -> Option<f64> {
    if tally.position_count == 0 {
        return None;
    }
    Some(round2(
        tally.position_sum as f64 / tally.position_count as f64,
    ))
}

/// A metric for the window, with its delta against the window before.
pub fn metric_value(current: Option<f64>, previous: Option<f64>) -> MetricValue {
    match (current, previous) {
        (Some(current), Some(previous)) => MetricValue {
            value: Some(current),
            delta: Some(round2(current - previous)),
            delta_available: true,
        },
        (Some(current), None) => MetricValue {
            value: Some(current),
            delta: None,
            delta_available: false,
        },
        (None, _) => MetricValue {
            value: None,
            delta: None,
            delta_available: false,
        },
    }
}

/// Rank 1 for the highest value. Equal values share a rank and the next rank is skipped.
/// `None` has no rank.
pub fn rank_desc(values: &[Option<f64>]) -> Vec<Option<i64>> {
    values
        .iter()
        .map(|value| {
            let value = (*value)?;
            let higher = values
                .iter()
                .flatten()
                .filter(|other| **other > value)
                .count();
            Some(higher as i64 + 1)
        })
        .collect()
}

/// `part` as a percent of `total`. `None` when `total` is zero.
pub fn percent(part: u64, total: u64) -> Option<f64> {
    if total == 0 {
        None
    } else {
        Some(round2(part as f64 * 100.0 / total as f64))
    }
}

/// Two decimals, so a payload and its test agree exactly.
pub fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tally(answers: u64, mentioned: u64, cited: u64, positions: &[i64]) -> AnswerTally {
        AnswerTally {
            answers,
            mentioned,
            cited,
            position_sum: positions.iter().sum(),
            position_count: positions.len() as u64,
            ..AnswerTally::default()
        }
    }

    #[test]
    fn rates_are_percentages_and_zero_answers_have_no_value() {
        let t = tally(3, 2, 1, &[]);
        assert_eq!(mention_rate(&t), Some(66.67));
        assert_eq!(citation_rate(&t), Some(33.33));
        assert_eq!(mention_rate(&AnswerTally::default()), None);
        assert_eq!(citation_rate(&AnswerTally::default()), None);
        assert_eq!(mention_rate(&tally(1, 1, 1, &[])), Some(100.0));
        assert_eq!(mention_rate(&tally(1, 0, 0, &[])), Some(0.0));
    }

    #[test]
    fn average_position_ignores_answers_without_a_position() {
        let t = tally(4, 0, 0, &[1, 3, 2]);
        assert_eq!(average_position(&t), Some(2.0));
        assert_eq!(average_position(&tally(2, 0, 0, &[])), None);
        assert_eq!(average_position(&tally(1, 0, 0, &[2])), Some(2.0));
    }

    #[test]
    fn share_of_voice_splits_our_mentions_against_competitors() {
        assert_eq!(share_of_voice(3, 3), Some(50.0));
        assert_eq!(share_of_voice(0, 2), Some(0.0));
        assert_eq!(share_of_voice(4, 0), Some(100.0));
        assert_eq!(share_of_voice(0, 0), None);
        assert_eq!(share_of_voice(1, 2), Some(33.33));
    }

    #[test]
    fn deltas_need_both_windows() {
        let both = metric_value(Some(50.0), Some(40.0));
        assert_eq!(both.value, Some(50.0));
        assert_eq!(both.delta, Some(10.0));
        assert!(both.delta_available);

        let only_current = metric_value(Some(50.0), None);
        assert_eq!(only_current.delta, None);
        assert!(!only_current.delta_available);

        let no_data = metric_value(None, Some(40.0));
        assert_eq!(no_data.value, None);
        assert!(!no_data.delta_available);

        let down = metric_value(Some(33.33), Some(66.67));
        assert_eq!(down.delta, Some(-33.34));
    }

    #[test]
    fn ranks_use_competition_ranking_and_skip_missing_values() {
        let ranks = rank_desc(&[Some(50.0), None, Some(80.0), Some(50.0), Some(10.0)]);
        assert_eq!(ranks, vec![Some(2), None, Some(1), Some(2), Some(4)]);
        assert_eq!(rank_desc(&[]), Vec::<Option<i64>>::new());
        assert_eq!(rank_desc(&[None]), vec![None]);
    }
}
