use super::Adapter;
use crate::models::{AdapterKind, NormalizedRecord};
use crate::normalize::{
    is_private_field, normalize_address_key, normalize_owner_opt, normalize_parcel_id, parse_bool,
    parse_money, value_to_string,
};
use anyhow::Result;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub struct ComplianceGraphqlAdapter;

impl Adapter for ComplianceGraphqlAdapter {
    fn kind(&self) -> AdapterKind {
        AdapterKind::ComplianceGraphql
    }

    fn parse_bytes(&self, bytes: &[u8]) -> Result<Vec<NormalizedRecord>> {
        let value: Value = serde_json::from_slice(bytes)?;
        let mut objects = Vec::new();
        collect_property_objects(&value, &mut objects);
        Ok(objects
            .iter()
            .enumerate()
            .map(|(index, object)| record_from_object(object, index))
            .collect())
    }
}

fn collect_property_objects<'a>(value: &'a Value, out: &mut Vec<&'a Map<String, Value>>) {
    match value {
        Value::Object(map) => {
            if map.contains_key("parcelId") || map.contains_key("parcel_id") {
                out.push(map);
                return;
            }
            if let Some(node) = map.get("node") {
                collect_property_objects(node, out);
            }
            for key in [
                "properties",
                "propertyIntelligence",
                "parcels",
                "candidateProperties",
            ] {
                if let Some(child) = map.get(key) {
                    collect_property_objects(child, out);
                }
            }
            if let Some(data) = map.get("data") {
                collect_property_objects(data, out);
            }
            if let Some(edges) = map.get("edges") {
                collect_property_objects(edges, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_property_objects(item, out);
            }
        }
        _ => {}
    }
}

fn record_from_object(object: &Map<String, Value>, index: usize) -> NormalizedRecord {
    let get = |names: &[&str]| -> Option<String> {
        names
            .iter()
            .find_map(|name| object.get(*name))
            .map(value_to_string)
            .filter(|value| !value.trim().is_empty())
    };
    let parcel_id = get(&["parcelId", "parcel_id"]);
    let parcel_id_norm = parcel_id.as_deref().and_then(normalize_parcel_id);
    let address = get(&["address", "streetAddress", "propertyAddress"]);
    let source_record_id = get(&["sourceRecordId", "id"])
        .or_else(|| parcel_id_norm.clone())
        .unwrap_or_else(|| format!("compliance_graphql:{index}"));

    NormalizedRecord {
        source_id: AdapterKind::ComplianceGraphql.as_str().to_string(),
        source_record_id,
        observed_at: get(&["observedAt", "updatedAt", "createdAt"])
            .unwrap_or_else(|| "1970-01-01".to_string()),
        parcel_id,
        parcel_id_norm,
        address_key: address.as_deref().and_then(normalize_address_key),
        address,
        owner_norm: get(&["owner", "ownerName"])
            .as_deref()
            .and_then(normalize_owner_opt),
        gclba_label: get(&["gclbaLabel", "gclbaOwned"])
            .as_deref()
            .and_then(parse_bool),
        structure_flag: Some(true),
        condition_grade: get(&["condition", "conditionGrade"]),
        prior_condition_grade: get(&["priorCondition", "previousCondition"]),
        assessed_value: get(&["assessedValue", "salePrice"])
            .as_deref()
            .and_then(parse_money),
        sale_date: get(&["saleDate", "closingDate"]),
        program: get(&["program", "caseProgram"]),
        facts: public_facts(object),
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
    fn compliance_graphql_adapter_reads_nested_edges() {
        let payload = serde_json::json!({
            "data": {
                "properties": {
                    "edges": [
                        {"node": {"parcelId": "4012345067", "address": "123 North Saginaw Street", "program": "Homebuyer"}}
                    ]
                }
            }
        });
        let rows = ComplianceGraphqlAdapter
            .parse_bytes(serde_json::to_vec(&payload).unwrap().as_slice())
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].parcel_id_norm.as_deref(), Some("40-12-345-067"));
        assert_eq!(rows[0].address_key.as_deref(), Some("123 N SAGINAW ST"));
    }
}
