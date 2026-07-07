use serde_json::Value;

const PRIVATE_MARKERS: &[&str] = &[
    "buyer",
    "email",
    "phone",
    "organization",
    "notes",
    "reviewedby",
    "reviewed_by",
];

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

pub fn normalize_parcel_id(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let parts: Vec<String> = trimmed
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(|part| part.to_ascii_uppercase())
        .collect();
    if parts.len() == 4
        && parts
            .iter()
            .all(|part| part.chars().all(|ch| ch.is_ascii_digit()))
    {
        return Some(format!(
            "{}-{}-{}-{}",
            pad_left(&parts[0], 2),
            pad_left(&parts[1], 2),
            pad_left(&parts[2], 3),
            pad_left(&parts[3], 3)
        ));
    }

    let compact: String = trimmed
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_uppercase)
        .collect();
    if compact.len() == 10 && compact.chars().all(|ch| ch.is_ascii_digit()) {
        return Some(format!(
            "{}-{}-{}-{}",
            &compact[0..2],
            &compact[2..4],
            &compact[4..7],
            &compact[7..10]
        ));
    }
    if compact.len() < 10 && compact.chars().all(|ch| ch.is_ascii_digit()) {
        let padded = pad_left(&compact, 10);
        return Some(format!(
            "{}-{}-{}-{}",
            &padded[0..2],
            &padded[2..4],
            &padded[4..7],
            &padded[7..10]
        ));
    }
    if compact.is_empty() {
        None
    } else {
        Some(compact)
    }
}

pub fn normalize_address_key(value: &str) -> Option<String> {
    let without_unit = value
        .split(|ch| ch == ',' || ch == ';')
        .next()
        .unwrap_or(value);
    let mut tokens = Vec::new();
    for raw in without_unit
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
    {
        let token = raw.to_ascii_uppercase();
        if matches!(
            token.as_str(),
            "APT" | "APARTMENT" | "UNIT" | "STE" | "SUITE" | "FL" | "FLOOR"
        ) {
            break;
        }
        tokens.push(standardize_address_token(&token).to_string());
    }

    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" "))
    }
}

pub fn normalize_owner(value: &str) -> Option<String> {
    let normalized = value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase();
    if normalized.is_empty() {
        None
    } else if is_gclba_owner(&normalized) {
        Some("GCLBA".to_string())
    } else {
        Some(normalized)
    }
}

pub fn is_gclba_owner(value: &str) -> bool {
    let owner = value.to_ascii_uppercase();
    owner.contains("GCLBA")
        || owner.contains("GENESEE COUNTY LAND BANK")
        || owner.contains("LAND BANK AUTHORITY")
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

fn pad_left(value: &str, width: usize) -> String {
    if value.len() >= width {
        value[value.len() - width..].to_string()
    } else {
        format!("{value:0>width$}")
    }
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
        "TERRACE" => "TER",
        "PARKWAY" => "PKWY",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parcel_ids_are_reformatted() {
        let cases = [
            ("40-12-345-067", "40-12-345-067"),
            ("4012345067", "40-12-345-067"),
            ("40.12.345.67", "40-12-345-067"),
            ("4-2-3-7", "04-02-003-007"),
        ];
        for (raw, expected) in cases {
            assert_eq!(normalize_parcel_id(raw).as_deref(), Some(expected));
        }
    }

    #[test]
    fn address_keys_standardize_suffixes_and_drop_units() {
        let cases = [
            ("123 North Saginaw Street Apt 4", "123 N SAGINAW ST"),
            ("10 w court road, unit 2", "10 W CT RD"),
            ("77 East Fifth Avenue", "77 E FIFTH AVE"),
        ];
        for (raw, expected) in cases {
            assert_eq!(normalize_address_key(raw).as_deref(), Some(expected));
        }
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
