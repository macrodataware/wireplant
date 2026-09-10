/// Evaluates a trip condition against a threshold level.
///
/// * `inverted = false`: standard trip alarm, triggers when `source_val >= threshold`.
/// * `inverted = true`: low-level / permissive trip, triggers when `source_val < threshold`.
#[inline]
pub fn evaluate_threshold(source_val: f64, threshold: f64, inverted: bool) -> bool {
    let tripped = source_val >= threshold;
    if inverted {
        !tripped
    } else {
        tripped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trip_is_inclusive_at_the_threshold_and_inverts_cleanly() {
        assert!(evaluate_threshold(185.0, 180.0, false));
        assert!(evaluate_threshold(180.0, 180.0, false));
        assert!(!evaluate_threshold(175.0, 180.0, false));

        assert!(evaluate_threshold(175.0, 180.0, true));
        assert!(!evaluate_threshold(180.0, 180.0, true));
        assert!(!evaluate_threshold(185.0, 180.0, true));
    }
}
