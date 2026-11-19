use std::env;
use std::time::Duration;

/// The seconds in the environment variable `name`, or `default` when it is
/// unset or not a positive number. The VM test shortens waits this way.
pub fn seconds(name: &str, default: Duration) -> Duration {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .map_or(default, Duration::from_secs_f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_or_odd_value_keeps_the_default() {
        let default = Duration::from_secs(45);
        assert_eq!(seconds("SEMMELTV_TIMING_TEST_UNSET", default), default);
    }
}
