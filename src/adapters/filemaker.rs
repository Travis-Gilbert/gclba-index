use super::Adapter;
use crate::models::{AdapterKind, NormalizedRecord};
use crate::normalize::{
    is_gclba_owner, is_private_field, normalize_address_key, normalize_header, normalize_owner_opt,
    normalize_parcel_id, parse_bool, parse_money,
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
    let address = value(&["address", "propertyaddress", "siteaddress", "name"]);
    let source_record_id = value(&["sourcerecordid", "id", "recordid"])
        .or_else(|| parcel_id_norm.clone())
        .unwrap_or_else(|| format!("filemaker_csv:{index}"));
    let owner_norm = value(&["owner", "ownername", "ownerofrecord", "ownerrecord"])
        .as_deref()
        .and_then(normalize_owner_opt);
    let explicit_gclba = value(&["gclbalabel", "gclbaowned", "landbankowned"])
        .as_deref()
        .and_then(parse_bool);
    let structure_flag = value(&[
        "structureflag",
        "hasstructure",
        "propertyclass",
        "propertytype",
        "landuse",
    ])
    .as_deref()
    .and_then(structure_from_value)
    .or(Some(true));

    NormalizedRecord {
        source_id: AdapterKind::FilemakerCsv.as_str().to_string(),
        source_record_id,
        observed_at: value(&[
            "observedat",
            "updatedat",
            "exportedat",
            "reviewedat",
            "portalsurveydate",
            "lastoutreachdate",
        ])
        .unwrap_or_else(|| "1970-01-01".to_string()),
        parcel_id,
        parcel_id_norm,
        address_key: address.as_deref().and_then(normalize_address_key),
        address,
        gclba_label: explicit_gclba.or_else(|| owner_norm.as_deref().map(is_gclba_owner)),
        owner_norm,
        structure_flag,
        condition_grade: value(&[
            "condition",
            "conditiongrade",
            "portalcondition",
            "surveycondition",
        ]),
        prior_condition_grade: value(&["priorcondition", "previouscondition"]),
        assessed_value: value(&[
            "assessedvalue",
            "assessedvaluation",
            "saleprice",
            "purchaseprice",
        ])
        .as_deref()
        .and_then(parse_money),
        sale_date: value(&[
            "saledate",
            "datesold",
            "dateofsale",
            "closingdate",
            "dispositiondate",
        ]),
        program: value(&[
            "program",
            "programtype",
            "dispositionprogram",
            "caseprogram",
        ]),
        facts: public_facts(headers, row),
    }
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

    #[test]
    fn filemaker_adapter_reads_current_export_headers() {
        let csv = "parcelId,address,buyerName,buyerEmail,programType,dateSold,enforcementLevel,currentAction\n\
                   40-12-345-067,123 North Saginaw Street,Jane Buyer,jane@example.com,Homebuyer,2026-02-12,High,Inspect";
        let rows = FilemakerCsvAdapter.parse_bytes(csv.as_bytes()).unwrap();
        let row = &rows[0];
        assert_eq!(row.parcel_id_norm.as_deref(), Some("40-12-345-067"));
        assert_eq!(row.program.as_deref(), Some("Homebuyer"));
        assert_eq!(row.sale_date.as_deref(), Some("2026-02-12"));
        assert!(!row.facts.contains_key("buyerName"));
        assert!(!row.facts.contains_key("buyerEmail"));
    }

    #[test]
    fn filemaker_adapter_reads_crm_export_columns_without_contact_fields() {
        let csv = "Id,Name,Buyer,Address,Date of sale,Contact email,Contact phone,Organization,Program,Owner of record,Assessed value,Parcel ID,Property class,Portal condition,Reviewed at,Review notes\n\
                   abc,123 N SAGINAW ST,Jane Buyer,123 N SAGINAW ST,2026-02-12,jane@example.com,555-1212,Buyer LLC,Homebuyer,Genesee County Land Bank Authority,25000,40-12-345-067,401,Good,2026-07-08,private note";
        let rows = FilemakerCsvAdapter.parse_bytes(csv.as_bytes()).unwrap();
        let row = &rows[0];
        assert_eq!(row.source_record_id, "abc");
        assert_eq!(row.parcel_id_norm.as_deref(), Some("40-12-345-067"));
        assert_eq!(
            row.owner_norm.as_deref(),
            Some("GENESEE COUNTY LAND BANK AUTHORITY")
        );
        assert_eq!(row.gclba_label, Some(true));
        assert_eq!(row.structure_flag, Some(true));
        assert_eq!(row.sale_date.as_deref(), Some("2026-02-12"));
        assert_eq!(row.condition_grade.as_deref(), Some("Good"));
        assert_eq!(row.assessed_value, Some(25_000.0));
        assert!(!row.facts.contains_key("Buyer"));
        assert!(!row.facts.contains_key("Contact email"));
        assert!(!row.facts.contains_key("Contact phone"));
        assert!(!row.facts.contains_key("Organization"));
        assert!(!row.facts.contains_key("Review notes"));
    }
}
