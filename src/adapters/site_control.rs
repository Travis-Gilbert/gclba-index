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
            let Some(properties) = feature
                .get("properties")
                .or_else(|| feature.get("attributes"))
                .and_then(Value::as_object)
            else {
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
        let parcel_id = first_parcel_id(
            normalized,
            &[
                "parcid",
                "paridshort",
                "paridpack",
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
                "addressfullc",
                "siteaddress",
                "propertyaddress",
                "address",
                "situsaddress",
                "location",
                "propcomb",
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
                "ownname",
                "taxname",
                "ownerofrecord",
            ],
        );
        let owner_norm = owner.as_deref().and_then(normalize_owner_opt);
        let explicit_gclba = first_value(
            normalized,
            &[
                "gclbalabel",
                "gclbaowned",
                "gclbowned",
                "landbankowned",
                "ownedbygclba",
            ],
        )
        .as_deref()
        .and_then(parse_bool);
        let source_inventory_label = self.source == AdapterKind::SiteControlExport
            && ["fhstatus", "r4rstatus", "parclrcsold", "tblpropcatmeaning"]
                .iter()
                .any(|name| normalized.contains_key(*name));
        let gclba_label = explicit_gclba
            .or_else(|| owner_norm.as_deref().map(is_gclba_owner))
            .or(source_inventory_label.then_some(true));
        let structure_flag = first_value(
            normalized,
            &[
                "structureflag",
                "hasstructure",
                "improved",
                "parcpropclass",
                "propertyclass",
                "propclass",
                "classtype",
                "tblpropcatmeaning",
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
                "portalcondition",
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
                "parcpropassmt",
                "assessedvalue",
                "assessment",
                "sev",
                "currentsev",
                "cap",
                "taxablevalue",
                "stateequalizedvalue",
                "fhaskingprice",
                "parcaskingprice",
                "minimumbid",
            ],
        )
        .as_deref()
        .and_then(parse_money);
        let observed_at = first_value(
            normalized,
            &[
                "observedat",
                "updatedat",
                "exportedat",
                "websitepublishenddate",
                "flyercreated",
                "flyerpublished",
                "portalsurveydate",
                "reviewedat",
            ],
        )
        .unwrap_or_else(|| "1970-01-01".to_string());
        let source_record_id = first_value(
            normalized,
            &[
                "sourcerecordid",
                "id",
                "objectid",
                "fid",
                "parcid",
                "parcelid",
            ],
        )
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
            sale_date: first_value(
                normalized,
                &["saledate", "lastsaledate", "dateofsale", "datesold"],
            ),
            program: first_value(
                normalized,
                &[
                    "program",
                    "dispositionprogram",
                    "fhstatus",
                    "r4rstatus",
                    "tblpropcatmeaning",
                    "compliance",
                    "2023foreclosuresstatus",
                ],
            ),
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

fn first_parcel_id(map: &Map<String, Value>, names: &[&str]) -> Option<String> {
    let mut fallback = None;
    for name in names {
        let Some(value) = map
            .get(*name)
            .map(value_to_string)
            .filter(|value| !value.trim().is_empty())
        else {
            continue;
        };
        if fallback.is_none() {
            fallback = Some(value.clone());
        }
        if normalize_parcel_id(&value).is_some() {
            return Some(value);
        }
    }
    fallback
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

fn public_facts(object: &Map<String, Value>) -> BTreeMap<String, Value> {
    object
        .iter()
        .filter(|(key, _)| !is_private_field(key))
        .filter(|(_, value)| !value.is_null())
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn county_arcgis_adapter_reads_land_bank_layer_fields() {
        let payload = r#"{
          "features": [
            {
              "attributes": {
                "FID": 1,
                "ParIDShort": "53-35-580-031",
                "PARIDPACK": "5335580031",
                "PropComb": "11598 ORCHARDVIEW DR",
                "PropClass": "402-RESIDENTIAL VACANT",
                "OwnName": "GENESEE COUNTY LAND BANK AUTHORITY",
                "Assessment": 0
              }
            }
          ]
        }"#;
        let rows = SiteControlExportAdapter::county_arcgis()
            .parse_bytes(payload.as_bytes())
            .unwrap();
        let row = &rows[0];
        assert_eq!(row.source_id, "county_arcgis");
        assert_eq!(row.source_record_id, "1");
        assert_eq!(row.parcel_id_norm.as_deref(), Some("53-35-580-031"));
        assert_eq!(row.address.as_deref(), Some("11598 ORCHARDVIEW DR"));
        assert_eq!(
            row.owner_norm.as_deref(),
            Some("GENESEE COUNTY LAND BANK AUTHORITY")
        );
        assert_eq!(row.structure_flag, Some(false));
        assert_eq!(row.assessed_value, Some(0.0));
    }

    #[test]
    fn county_arcgis_prefers_normalizable_short_parcel_id() {
        let csv = "FID,ParIDShort,ParcelID,PARIDPACK,PropComb,PropClass,OwnName,Assessment\n\
                   1,53-35-580-031,25-53-35-580-031,5335580031,11598 ORCHARDVIEW DR,402-RESIDENTIAL VACANT,GENESEE COUNTY LAND BANK AUTHORITY,0\n";
        let rows = SiteControlExportAdapter::county_arcgis()
            .parse_bytes(csv.as_bytes())
            .unwrap();
        assert_eq!(rows[0].parcel_id.as_deref(), Some("53-35-580-031"));
        assert_eq!(rows[0].parcel_id_norm.as_deref(), Some("53-35-580-031"));
    }

    #[test]
    fn featured_homes_export_fields_imply_gclba_inventory() {
        let csv = "Parc_Id,Address Full_c,Parc_Prop_Class,Parc_Prop_Assmt,FH_Status,GCLB Owned,Tbl_Prop_Cat_Meaning\n\
                   4002426001,\"1009 W DAYTON ST\nFLINT, MI 48504\",401,7200,Needs Cleanout,,Sell\n";
        let rows = SiteControlExportAdapter::default()
            .parse_bytes(csv.as_bytes())
            .unwrap();
        let row = &rows[0];
        assert_eq!(row.source_record_id, "4002426001");
        assert_eq!(row.parcel_id_norm.as_deref(), Some("40-02-426-001"));
        assert_eq!(row.gclba_label, Some(true));
        assert_eq!(row.structure_flag, Some(true));
        assert_eq!(row.assessed_value, Some(7200.0));
        assert_eq!(row.program.as_deref(), Some("Needs Cleanout"));
    }
}
