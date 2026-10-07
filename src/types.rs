use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

macro_rules! choice {
    ($name:ident, $error:literal, {$($variant:ident => $label:literal),+ $(,)?}) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "lowercase")]
        pub enum $name { $($variant),+ }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(match self { $(Self::$variant => $label),+ })
            }
        }

        impl FromStr for $name {
            type Err = String;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $($label => Ok(Self::$variant),)+
                    _ => Err($error.into()),
                }
            }
        }
    };
}

choice!(Unit, "unit must be usd|tokens|requests", {
    Usd => "usd", Tokens => "tokens", Requests => "requests",
});
choice!(Period, "period must be day|week|month", {
    Day => "day", Week => "week", Month => "month",
});
choice!(By, "--by must be project|model|day", {
    Project => "project", Model => "model", Day => "day",
});

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn choices_roundtrip_and_reject_unknown_values() {
        for unit in [Unit::Usd, Unit::Tokens, Unit::Requests] {
            assert_eq!(unit.to_string().parse::<Unit>().unwrap(), unit);
            assert_eq!(
                serde_json::from_str::<Unit>(&serde_json::to_string(&unit).unwrap()).unwrap(),
                unit
            );
        }
        for period in [Period::Day, Period::Week, Period::Month] {
            assert_eq!(period.to_string().parse::<Period>().unwrap(), period);
            assert_eq!(
                serde_json::from_str::<Period>(&serde_json::to_string(&period).unwrap()).unwrap(),
                period
            );
        }
        for by in [By::Project, By::Model, By::Day] {
            assert_eq!(by.to_string().parse::<By>().unwrap(), by);
        }
        assert!("dollars".parse::<Unit>().is_err());
        assert!("year".parse::<Period>().is_err());
        assert!("session".parse::<By>().is_err());
        assert!(serde_json::from_str::<Unit>("\"dollars\"").is_err());
        assert!(serde_json::from_str::<Period>("\"year\"").is_err());
        assert!(serde_json::from_str::<By>("\"session\"").is_err());
    }
}
