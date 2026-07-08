use super::Adapter;
use crate::models::{AdapterKind, NormalizedRecord};
use crate::normalize::{
    is_gclba_owner, is_private_field, normalize_address_key, normalize_header, normalize_owner_opt,
    normalize_parcel_id, parse_bool, parse_money, value_to_string,
};
use anyhow::{anyhow, Result};
use csv::StringRecord;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub struct SiteControlExportAdapter {
    source: AdapterKind,
}

impl SiteControlExportAdapter {
    pub const fn county_arcgis() -> Self {
        Self {
            source: AdapterKind::CountyArcgis,
        }
    }

    fn source_id(&self) -> &'static str {
        self.source.as_str()
    }
}

impl Default for SiteControlExportAdapter {
    fn default() -> Self {
        Self {
            source: AdapterKind::SiteControlExport,
        }
    }
}

impl Adapter for SiteControlExportAdapter {
    fn kind(&self) -> AdapterKind {
        self.source
    }

    fn parse_bytes(&self, bytes: &[u8]) -> Result<Vec<NormalizedRecord>> {
        let text = std::str::from_utf8(bytes)?;
        if text.trim_start().starts_with('{') {
            self.parse_geojson(text)
        } else {
            self.parse_csv(text)
        }
    }
}

impl SiteControlExportAdapter {
    fn parse_csv(&self, text: &str) -> Result<Vec<NormalizedRecord>> {
        let mut reader = csv::ReaderBuilder::new()
            .flexible(true)
            .from_reader(text.as_bytes());
        let headers = reader.headers()?.clone();
        let normalized_headers: Vec<String> = headers.iter().map(normalize_header).collect();
        let mut records = Vec::new();
        for (index, row) in reader.records().enumerate() {
            records.push(self.record_from_csv(&headers, &normalized_headers, &row?, index)?);
        }
        Ok(records)
    }

    fn parse_geojson(&self, text: &str) -> Result<Vec<NormalizedRecord>> {
        let value: Value = serde_json::from_str(text)?;
        let features = value
            .get("features")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("geojson export must contain a features array"))?;
        let mut records = Vec::new();
        for (index, feature) in features.iter().enumerate() {
            let Some(properties) = feature.get("properties").and_then(Value::as_object) else {
                continue;
            };
            records.push(self.record_from_object(properties, index)?);
        }
        Ok(records)
    }

    fn record_from_csv(
        &self,
        headers: &StringRecord,
        normalized_headers: &[String],
        row: &StringRecord,
        index: usize,
    ) -> Result<NormalizedRecord> {
        let mut object = Map::new();
        for (i, header) in headers.iter().enumerate() {
            let value = row.get(i).unwrap_or("").trim();
            if !value.is_empty() {
                object.insert(header.to_string(), Value::String(value.to_string()));
            }
        }
        let mut normalized_object = Map::new();
        for (i, header) in normalized_headers.iter().enumerate() {
            let value = row.get(i).unwrap_or("").trim();
            if !value.is_empty() {
                normalized_object.insert(header.clone(), Value::String(value.to_string()));
            }
        }
        self.record_from_maps(&object, &normalized_object, index)
    }

    fn record_from_object(
        &self,
        object: &Map<String, Value>,
        index: usize,
    ) -> Result<NormalizedRecord> {
        let mut normalized_object = Map::new();
        for (key, value) in object {
            normalized_object.insert(normalize_header(key), value.clone());
        }
        self.record_from_maps(object, &normalized_object, index)
    }

    fn record_from_maps(
        &self,
        original: &Map<String, Value>,
        normalized: &Map<String, Value>,
        index: usize,
    ) -> Result<NormalizedRecord> {
        let parcel_id = first_value(
            normalized,
            &[
                "parcelid",
                "parcelnumber",
                "parcelno",
                "parid",
                "propertyid",
                "pin",
            ],
        );
        let parcel_id_norm = parcel_id.as_deref().and_then(normalize_parcel_id);
        let address = first_value(
            normalized,
            &[
                "siteaddress",
                "propertyaddress",
                "address",
                "situsaddress",
                "location",
            ],
        );
        let owner = first_value(
            normalized,
            &[
                "owner",
                "ownername",
                "ownernorm",
                "taxpayer",
                "taxpayername",
            ],
        );
        let owner_norm = owner.as_deref().and_then(normalize_owner_opt);
        let explicit_gclba = first_value(
            normalized,
            &["gclbalabel", "gclbaowned", "landbankowned", "ownedbygclba"],
        )
        .as_deref()
        .and_then(parse_bool);
        let gclba_label = explicit_gclba.or_else(|| owner_norm.as_deref().map(is_gclba_owner));
        let structure_flag = first_value(
            normalized,
            &[
                "structureflag",
                "hasstructure",
                "improved",
                "propertyclass",
                "landuse",
                "usetype",
            ],
        )
        .as_deref()
        .and_then(structure_from_value);
        let condition_grade = first_value(
            normalized,
            &[
                "condition",
                "conditiongrade",
                "surveycondition",
                "buildingcondition",
            ],
        );
        let prior_condition_grade = first_value(
            normalized,
            &[
                "priorcondition",
                "priorconditiongrade",
                "previouscondition",
                "previousconditiongrade",
            ],
        );
        let assessed_value = first_value(
            normalized,
            &[
                "assessedvalue",
                "assessment",
                "taxablevalue",
                "stateequalizedvalue",
            ],
        )
        .as_deref()
        .and_then(parse_money);
        let observed_at = first_value(normalized, &["observedat", "updatedat", "exportedat"])
            .unwrap_or_else(|| "1970-01-01".to_string());
        let source_record_id = first_value(normalized, &["sourcerecordid", "id", "objectid"])
            .or_else(|| parcel_id_norm.clone())
            .unwrap_or_else(|| format!("{}:{index}", self.source_id()));

        Ok(NormalizedRecord {
            source_id: self.source_id().to_string(),
            source_record_id,
            observed_at,
            parcel_id,
            parcel_id_norm,
            address_key: address.as_deref().and_then(normalize_address_key),
            address,
            owner_norm,
            gclba_label,
            structure_flag,
            condition_grade,
            prior_condition_grade,
            assessed_value,
            sale_date: first_value(normalized, &["saledate", "lastsaledate"]),
            program: first_value(normalized, &["program", "dispositionprogram"]),
            facts: public_facts(original),
        })
    }
}

fn first_value(map: &Map<String, Value>, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| map.get(*name))
        .map(value_to_string)
        .filter(|value| !value.trim().is_empty())
}

fn structure_from_value(value: &str) -> Option<bool> {
    if let Some(parsed) = parse_bool(value) {
        return Some(parsed);
    }
    let lower = value.to_ascii_lowercase();
    if lower.contains("vacant") && lower.contains("lot") {
        Some(false)
    } else if [
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

fn public_facts(object: &Map<String, Value>) -> BTreeMap<String, Value> {
    object
        .iter()
        .filter(|(key, _)| !is_private_field(key))
        .filter(|(_, value)| !value.is_null())
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
