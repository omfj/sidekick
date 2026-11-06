use std::fmt;
use std::str::FromStr;
use std::time::Duration;

/// A more human readable duration like `10s`, `30secs`, `1m` or `5mins`. `0` is zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeDuration(Duration);

impl TimeDuration {
    pub fn get(self) -> Duration {
        self.0
    }
}

impl FromStr for TimeDuration {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s == "0" {
            return Ok(Self(Duration::ZERO));
        }
        let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
        let (number, unit) = s.split_at(split);
        let n: u64 = number
            .parse()
            .map_err(|_| format!("`{s}` should start with a whole number, like `10s` or `1m`"))?;
        if unit.starts_with('.') {
            return Err(format!(
                "`{s}` must be a whole number; try seconds, e.g. `90s`"
            ));
        }
        let multiplier = match unit.trim() {
            "s" | "sec" | "secs" => 1,
            "m" | "min" | "mins" => 60,
            "" => return Err(format!("`{s}` needs a unit, like `{s}s` or `{s}m`")),
            other => {
                return Err(format!(
                    "unknown unit `{other}`; use s/sec/secs or m/min/mins"
                ));
            }
        };
        let secs = n
            .checked_mul(multiplier)
            .ok_or_else(|| format!("`{s}` is too large"))?;
        Ok(Self(Duration::from_secs(secs)))
    }
}

impl fmt::Display for TimeDuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let secs = self.0.as_secs();
        if secs != 0 && secs.is_multiple_of(60) {
            write!(f, "{}m", secs / 60)
        } else {
            write!(f, "{secs}s")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_to_secs(s: &str) -> Result<u64, String> {
        s.parse::<TimeDuration>().map(|d| d.get().as_secs())
    }

    #[test]
    fn parses_units() {
        // Input - Expected output in seconds.
        for (input, expected) in [
            ("10s", 10),
            ("10sec", 10),
            ("10secs", 10),
            ("1m", 60),
            ("1min", 60),
            ("5mins", 300),
            (" 2 m ", 120),
            ("0", 0),
            ("0s", 0),
        ] {
            assert_eq!(parse_to_secs(input), Ok(expected), "{input}");
        }
    }

    #[test]
    fn rejects_bad_input() {
        for input in ["10", "", "m", "1.5m", "-1s", "10h", "10 seconds"] {
            assert!(parse_to_secs(input).is_err(), "{input} should be rejected");
        }
    }

    #[test]
    fn displays_compactly() {
        assert_eq!(TimeDuration(Duration::from_secs(600)).to_string(), "10m");
        assert_eq!(TimeDuration(Duration::from_secs(90)).to_string(), "90s");
    }
}
