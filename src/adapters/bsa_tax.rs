use super::Adapter;
use crate::models::{AdapterKind, NormalizedRecord};
use crate::normalize::{
    is_private_field, normalize_address_key, normalize_header, normalize_parcel_id, parse_bool,
    parse_money,
};
use anyhow::Result;
use csv::StringRecord;
use serde_json::Value;
use std::collections::BTreeMap;

pub struct BsaTaxAdapter;

impl Adapter for BsaTaxAdapter {
    fn kind(&self) -> AdapterKind {
        AdapterKind::BsaTax
    }

    fn parse_bytes(&self, bytes: &[u8]) -> Result<Vec<NormalizedRecord>> {
        let text = std::str::from_utf8(bytes)?;
        let mut reader = csv::ReaderBuilder::new()
            .flexible(true)
            .from_reader(text.as_bytes());
        let headers = reader.headers()?.clone();
        let normalized_headers: Vec<String> = headers.iter().map(normalize_header).collect();
        let mut records = Vec::new();
        for (index, row) in reader.records().enumerate() {
            records.push(record_from_row(&headers, &normalized_headers, &row?, index));
        }
        Ok(records)
    }
}

fn record_from_row(
    headers: &StringRecord,
    normalized_headers: &[String],
    row: &StringRecord,
    index: usize,
) -> NormalizedRecord {
    let value = |aliases: &[&str]| -> Option<String> {
        aliases
            .iter()
            .find_map(|alias| normalized_headers.iter().position(|header| header == alias))
            .and_then(|position| row.get(position))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    };
    let parcel_id = value(&["pid", "parcelid", "parcelnumber", "propertynumber", "pin"]);
    let parcel_id_norm = parcel_id.as_deref().and_then(normalize_parcel_id);
    let address = compose_street_address(&value)
        .or_else(|| value(&["propertyaddress", "siteaddress", "situsaddress", "address"]));
    let source_record_id = value(&["sourcerecordid", "id", "recordid", "propertynumber", "pid"])
        .or_else(|| parcel_id_norm.clone())
        .unwrap_or_else(|| format!("bsa_tax:{index}"));

    NormalizedRecord {
        source_id: AdapterKind::BsaTax.as_str().to_string(),
        source_record_id,
        observed_at: value(&["observedat", "updatedat", "exportedat"])
            .unwrap_or_else(|| "1970-01-01".to_string()),
        parcel_id,
        parcel_id_norm,
        address_key: address.as_deref().and_then(normalize_address_key),
        address,
        owner_norm: None,
        gclba_label: None,
        structure_flag: value(&["propertyclass", "class", "landuse", "usetype"])
            .as_deref()
            .and_then(structure_from_value),
        condition_grade: None,
        prior_condition_grade: None,
        assessed_value: value(&[
            "currentsev",
            "sev",
            "assessedvalue",
            "taxablevalue",
            "minimumbid",
        ])
        .as_deref()
        .and_then(parse_money),
        sale_date: None,
        program: value(&[
            "foreclosurestatus",
            "2023foreclosuresstatus",
            "taxstatus",
            "status",
            "disposition",
        ]),
        facts: public_facts(headers, row),
    }
}

fn compose_street_address(value: &impl Fn(&[&str]) -> Option<String>) -> Option<String> {
    let number = value(&["streetnumber", "address"])?;
    let direction = value(&["dir", "direction"]).unwrap_or_default();
    let street = value(&["streetname", "street", "streetname1"])?;
    let city = value(&["city"]).unwrap_or_default();
    let state = value(&["state"]).unwrap_or_default();
    let zip = value(&["zip", "zipcode"]).unwrap_or_default();
    Some(
        [number, direction, street, city, state, zip]
            .into_iter()
            .filter(|part| !part.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn structure_from_value(value: &str) -> Option<bool> {
    if let Some(parsed) = parse_bool(value) {
        return Some(parsed);
    }
    let lower = value.to_ascii_lowercase();
    let numeric = lower
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if numeric == "402" || lower.contains("vacant") {
        Some(false)
    } else if numeric == "401"
        || [
            "home",
            "house",
            "residential",
            "dwelling",
            "structure",
            "improved",
        ]
        .iter()
        .any(|term| lower.contains(term))
    {
        Some(true)
    } else {
        None
    }
}

fn public_facts(headers: &StringRecord, row: &StringRecord) -> BTreeMap<String, Value> {
    headers
        .iter()
        .enumerate()
        .filter(|(_, header)| !is_private_field(header))
        .filter_map(|(index, header)| {
            let value = row.get(index)?.trim();
            (!value.is_empty()).then(|| (header.to_string(), Value::String(value.to_string())))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bsa_tax_adapter_reads_foreclosure_disposition_csv() {
        let csv = "PID,Property Number,Address,Dir,Street-Name,City,State,Zip,Property Class,Minimum Bid,CURRENT SEV,2023_Foreclosures_Status\n\
                   0209300015,02-09-300-015,9174,,PERRY RD,GRAND BLANC,MI,48411,401 - RESIDENTIAL IMPROVED,7161.65,74900,SEPTEMBER SOLD\n";
        let rows = BsaTaxAdapter.parse_bytes(csv.as_bytes()).unwrap();
        let row = &rows[0];
        assert_eq!(row.source_id, "bsa_tax");
        assert_eq!(row.source_record_id, "02-09-300-015");
        assert_eq!(row.parcel_id_norm.as_deref(), Some("02-09-300-015"));
        assert_eq!(
            row.address.as_deref(),
            Some("9174 PERRY RD GRAND BLANC MI 48411")
        );
        assert_eq!(
            row.address_key.as_deref(),
            Some("9174 PERRY RD GRAND BLANC MI 48411")
        );
        assert_eq!(row.structure_flag, Some(true));
        assert_eq!(row.assessed_value, Some(74_900.0));
        assert_eq!(row.program.as_deref(), Some("SEPTEMBER SOLD"));
    }
}
