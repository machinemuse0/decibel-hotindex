use crate::{HotIndexError, Result, TimeWindow};
use std::str::FromStr;

pub fn reverse_ts_us(ts: u64) -> u64 {
    u64::MAX - ts
}

impl FromStr for TimeWindow {
    type Err = HotIndexError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "24h" => Ok(Self::H24),
            "7d" => Ok(Self::D7),
            other => Err(HotIndexError::Parse(format!(
                "unsupported time window: {other}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::reverse_ts_us;
    use crate::TimeWindow;
    use std::str::FromStr;

    #[test]
    fn reverse_timestamp_orders_recent_first() {
        let older = reverse_ts_us(100);
        let newer = reverse_ts_us(200);
        assert!(newer < older);
    }

    #[test]
    fn parses_supported_windows() {
        assert_eq!(TimeWindow::from_str("24h").unwrap(), TimeWindow::H24);
        assert_eq!(TimeWindow::from_str("7d").unwrap(), TimeWindow::D7);
        assert!(TimeWindow::from_str("1h").is_err());
    }

    #[test]
    fn serializes_window_labels() {
        assert_eq!(serde_json::to_string(&TimeWindow::H24).unwrap(), "\"24h\"");
        assert_eq!(serde_json::to_string(&TimeWindow::D7).unwrap(), "\"7d\"");
    }
}
