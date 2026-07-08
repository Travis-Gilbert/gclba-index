use serde_json::Value;
use std::fmt;

const PRIVATE_MARKERS: &[&str] = &[
    "buyer",
    "email",
    "phone",
    "organization",
    "notes",
    "reviewedby",
    "reviewed_by",
];

pub const GCLBA_OWNER_NAMES: &[&str] = &[
    "GENESEE COUNTY LAND BANK",
    "GENESEE COUNTY LAND BANK AUTHORITY",
    "GCLBA",
];

pub const CONDITION_GRADE_ORDER: &[&str] = &["EXCELLENT", "GOOD", "FAIR", "POOR", "DEMO"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PidError {
    Empty,
    WrongDigitCount { digits: usize },
}

impl fmt::Display for PidError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(formatter, "parcel id is empty"),
            Self::WrongDigitCount { digits } => {
                write!(formatter, "parcel id has {digits} digits, expected 10")
            }
        }
    }
}

impl std::error::Error for PidError {}

pub fn normalize_header(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub fn is_private_field(value: &str) -> bool {
    let normalized = normalize_header(value);
    PRIVATE_MARKERS
        .iter()
        .any(|marker| normalized.contains(&normalize_header(marker)))
}

pub fn normalize_pid(value: &str) -> Result<String, PidError> {
    let digits: String = value.chars().filter(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        return Err(PidError::Empty);
    }
    if digits.len() != 10 {
        return Err(PidError::WrongDigitCount {
            digits: digits.len(),
        });
    }

    Ok(format!(
        "{}-{}-{}-{}",
        &digits[0..2],
        &digits[2..4],
        &digits[4..7],
        &digits[7..10]
    ))
}

pub fn normalize_parcel_id(value: &str) -> Option<String> {
    normalize_pid(value).ok()
}

pub fn normalize_address(value: &str) -> String {
    let scrubbed = value
        .chars()
        .map(|ch| {
            if ch == '#' {
                '#'
            } else if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                ' '
            }
        })
        .collect::<String>();

    let mut tokens = Vec::new();
    for raw in scrubbed.split_whitespace() {
        if is_unit_designator(raw) {
            break;
        }
        tokens.push(standardize_address_token(raw).to_string());
    }
    tokens.join(" ")
}

pub fn normalize_address_key(value: &str) -> Option<String> {
    let normalized = normalize_address(value);
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

pub fn normalize_owner(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase()
}

pub fn normalize_owner_opt(value: &str) -> Option<String> {
    let normalized = normalize_owner(value);
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

pub fn is_gclba_owner(value: &str) -> bool {
    let owner = normalize_owner(value);
    GCLBA_OWNER_NAMES.iter().any(|name| owner == *name)
}

pub fn condition_grade_rank(value: &str) -> Option<usize> {
    let normalized = normalize_owner(value);
    match normalized.as_str() {
        "A" => Some(0),
        "B" => Some(1),
        "C" => Some(2),
        "D" => Some(3),
        "E" => Some(4),
        "DEMOLITION" => Some(4),
        _ => CONDITION_GRADE_ORDER
            .iter()
            .position(|grade| normalized == *grade),
    }
}

pub fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "t" | "yes" | "y" | "1" | "owned" | "gclba" => Some(true),
        "false" | "f" | "no" | "n" | "0" | "private" => Some(false),
        _ => None,
    }
}

pub fn parse_money(value: &str) -> Option<f64> {
    let cleaned: String = value
        .chars()
        .filter(|ch| ch.is_ascii_digit() || *ch == '.' || *ch == '-')
        .collect();
    if cleaned.is_empty() {
        None
    } else {
        cleaned.parse().ok()
    }
}

pub fn value_to_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}

fn is_unit_designator(token: &str) -> bool {
    if token.starts_with('#') {
        return true;
    }
    matches!(
        token,
        "APT" | "APARTMENT" | "UNIT" | "STE" | "SUITE" | "LOT" | "#"
    )
}

fn standardize_address_token(token: &str) -> &str {
    match token {
        "NORTH" => "N",
        "SOUTH" => "S",
        "EAST" => "E",
        "WEST" => "W",
        "NORTHEAST" => "NE",
        "NORTHWEST" => "NW",
        "SOUTHEAST" => "SE",
        "SOUTHWEST" => "SW",
        "STREET" => "ST",
        "ROAD" => "RD",
        "AVENUE" => "AVE",
        "DRIVE" => "DR",
        "BOULEVARD" => "BLVD",
        "LANE" => "LN",
        "COURT" => "CT",
        "PLACE" => "PL",
        "CIRCLE" => "CIR",
        "HIGHWAY" => "HWY",
        "TERRACE" => "TER",
        "PARKWAY" => "PKWY",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parcel_ids_are_strictly_reformatted() {
        let cases = [
            ("14-34-577-050", "14-34-577-050"),
            ("1434577050", "14-34-577-050"),
            ("14 34 577 050", "14-34-577-050"),
            ("14.34.577.050", "14-34-577-050"),
            ("40-10-227-044", "40-10-227-044"),
            ("4001411023", "40-01-411-023"),
        ];
        for (raw, expected) in cases {
            assert_eq!(normalize_pid(raw).as_deref(), Ok(expected));
            assert_eq!(normalize_parcel_id(raw).as_deref(), Some(expected));
        }
        assert_eq!(
            normalize_pid("143457705").unwrap_err(),
            PidError::WrongDigitCount { digits: 9 }
        );
        assert_eq!(normalize_parcel_id("143457705"), None);
    }

    #[test]
    fn address_keys_standardize_suffixes_and_drop_units() {
        let cases = [
            ("123 North Saginaw Street Apt 4", "123 N SAGINAW ST"),
            ("10 w court road, unit 2", "10 W CT RD"),
            ("77 East Fifth Avenue", "77 E FIFTH AVE"),
            ("8 southeast circle #12", "8 SE CIR"),
            ("9 Northwest Highway Lot 3", "9 NW HWY"),
            ("14 west court boulevard", "14 W CT BLVD"),
            ("22 northeast lane suite 8", "22 NE LN"),
            ("3 Southwest Place", "3 SW PL"),
            ("4 South Drive", "4 S DR"),
        ];
        for (raw, expected) in cases {
            assert_eq!(normalize_address(raw), expected);
            assert_eq!(normalize_address_key(raw).as_deref(), Some(expected));
        }
    }

    #[test]
    fn owners_are_normalized_without_collapsing_identity() {
        assert_eq!(
            normalize_owner("Genesee County Land Bank Authority, Inc."),
            "GENESEE COUNTY LAND BANK AUTHORITY INC"
        );
        assert_eq!(
            normalize_owner(" Genesee County Land Bank Authority "),
            "GENESEE COUNTY LAND BANK AUTHORITY"
        );
        assert!(is_gclba_owner("Genesee County Land Bank Authority"));
        assert!(is_gclba_owner("GCLBA"));
        assert!(!is_gclba_owner("Private Land Bank Holdings"));
    }

    #[test]
    fn condition_grade_order_is_fixed() {
        assert_eq!(
            CONDITION_GRADE_ORDER,
            ["EXCELLENT", "GOOD", "FAIR", "POOR", "DEMO"]
        );
        assert!(condition_grade_rank("Poor") > condition_grade_rank("Good"));
        assert_eq!(
            condition_grade_rank("demolition"),
            condition_grade_rank("E")
        );
        assert_eq!(condition_grade_rank("unknown"), None);
    }

    #[test]
    fn private_fields_are_detected_before_ingest() {
        for name in [
            "buyerName",
            "email_address",
            "Phone 1",
            "reviewedBy",
            "staff notes",
        ] {
            assert!(is_private_field(name), "{name} should be private");
        }
        assert!(!is_private_field("condition_grade"));
    }
}
