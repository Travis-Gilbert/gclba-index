use super::Adapter;
use crate::models::{AdapterKind, NormalizedRecord};
use crate::normalize::{
    is_private_field, normalize_address_key, normalize_header, normalize_owner,
    normalize_parcel_id, parse_money,
};
use anyhow::Result;
use csv::StringRecord;
use serde_json::Value;
use std::collections::BTreeMap;

pub struct FilemakerCsvAdapter;

impl Adapter for FilemakerCsvAdapter {
    fn kind(&self) -> AdapterKind {
        AdapterKind::FilemakerCsv
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
    let parcel_id = value(&["parcelid", "parcelnumber", "parcelno", "parid", "pin"]);
    let parcel_id_norm = parcel_id.as_deref().and_then(normalize_parcel_id);
    let address = value(&["address", "propertyaddress", "siteaddress"]);
    let source_record_id = value(&["sourcerecordid", "id", "recordid"])
        .or_else(|| parcel_id_norm.clone())
        .unwrap_or_else(|| format!("filemaker_csv:{index}"));

    NormalizedRecord {
        source_id: AdapterKind::FilemakerCsv.as_str().to_string(),
        source_record_id,
        observed_at: value(&["observedat", "updatedat", "exportedat"])
            .unwrap_or_else(|| "1970-01-01".to_string()),
        parcel_id,
        parcel_id_norm,
        address_key: address.as_deref().and_then(normalize_address_key),
        address,
        owner_norm: value(&["owner", "ownername"])
            .as_deref()
            .and_then(normalize_owner),
        gclba_label: None,
        structure_flag: Some(true),
        condition_grade: value(&["condition", "conditiongrade"]),
        prior_condition_grade: value(&["priorcondition", "previouscondition"]),
        assessed_value: value(&["assessedvalue", "saleprice", "purchaseprice"])
            .as_deref()
            .and_then(parse_money),
        sale_date: value(&["saledate", "closingdate", "dispositiondate"]),
        program: value(&["program", "dispositionprogram", "caseprogram"]),
        facts: public_facts(headers, row),
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
    fn filemaker_adapter_drops_buyer_and_contact_pii() {
        let csv = "parcel_id,address,program,buyerName,email,phone,organization,notes,reviewedBy\n\
                   40-12-345-067,123 North Saginaw Street,Side Lot,Jane Buyer,jane@example.com,555-1212,Buyer LLC,call after 5,Staff";
        let rows = FilemakerCsvAdapter.parse_bytes(csv.as_bytes()).unwrap();
        let facts = &rows[0].facts;
        assert!(facts.contains_key("program"));
        for blocked in [
            "buyerName",
            "email",
            "phone",
            "organization",
            "notes",
            "reviewedBy",
        ] {
            assert!(!facts.contains_key(blocked), "{blocked} leaked into facts");
        }
        let serialized = serde_json::to_string(&rows).unwrap();
        for blocked_value in [
            "Jane Buyer",
            "jane@example.com",
            "555-1212",
            "Buyer LLC",
            "call after 5",
            "Staff",
        ] {
            assert!(!serialized.contains(blocked_value));
        }
    }
}
