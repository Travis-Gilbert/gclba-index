use crate::models::NormalizedRecord;
use anyhow::Result;
use rustyred_thg_core::InMemoryGraphStore;
use rustyred_thg_datawave::{
    DatawaveIngest, FieldConfig, FieldMapRule, FieldType, IndexPolicy, IngestStats, MappedHelper,
    MaterializeConfig, RawRecord,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatawaveBatchSummary {
    pub ingested: usize,
    pub skipped: usize,
    pub fields_written: usize,
    pub edges_written: usize,
}

pub fn ingest_datawave_records(records: &[NormalizedRecord]) -> Result<DatawaveBatchSummary> {
    let config = FieldConfig::new()
        .with_field("PARCEL_ID", FieldType::Text, IndexPolicy::INDEXED)
        .with_field("PARCEL_ID_NORM", FieldType::Text, IndexPolicy::INDEXED)
        .with_field("ADDRESS_KEY", FieldType::Text, IndexPolicy::INDEXED)
        .with_field("OWNER_NORM", FieldType::Text, IndexPolicy::INDEXED)
        .with_field("SOURCE", FieldType::Text, IndexPolicy::INDEXED)
        .with_field("SOURCE_RECORD_ID", FieldType::Text, IndexPolicy::INDEXED);
    let rules = vec![
        FieldMapRule::new("parcelId", "PARCEL_ID"),
        FieldMapRule::new("parcelIdNorm", "PARCEL_ID_NORM"),
        FieldMapRule::new("addressKey", "ADDRESS_KEY"),
        FieldMapRule::new("ownerNorm", "OWNER_NORM"),
        FieldMapRule::new("sourceId", "SOURCE"),
        FieldMapRule::new("sourceRecordId", "SOURCE_RECORD_ID"),
    ];
    let helper = MappedHelper::new("gclba_property_source", rules, config);
    let mut ingest = DatawaveIngest::new(MaterializeConfig::default());
    ingest.register(Box::new(helper));

    let raw_records: Vec<RawRecord> = records
        .iter()
        .map(|record| {
            RawRecord::json(
                "gclba_property_source",
                serde_json::to_value(record).expect("NormalizedRecord serializes"),
                0,
            )
            .with_external_id(record.source_record_id.clone())
        })
        .collect();

    let mut store = InMemoryGraphStore::default();
    let mut stats = IngestStats::new();
    let mut ingested = 0;
    let mut skipped = 0;
    let mut fields_written = 0;
    let mut edges_written = 0;
    for record in &raw_records {
        match ingest.ingest_record(&mut store, record, &mut stats) {
            Ok(outcome) => {
                ingested += 1;
                fields_written += outcome.fields_written;
                edges_written += outcome.edges_written;
            }
            Err(_) => skipped += 1,
        }
    }
    Ok(DatawaveBatchSummary {
        ingested,
        skipped,
        fields_written,
        edges_written,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::AdapterKind;

    #[test]
    fn datawave_crate_ingests_normalized_record() {
        let records = vec![NormalizedRecord {
            source_id: AdapterKind::SiteControlExport.as_str().to_string(),
            source_record_id: "portal-1".to_string(),
            observed_at: "2026-07-07".to_string(),
            parcel_id: Some("4012345067".to_string()),
            parcel_id_norm: Some("40-12-345-067".to_string()),
            address: Some("123 North Saginaw Street".to_string()),
            address_key: Some("123 N SAGINAW ST".to_string()),
            owner_norm: Some("GCLBA".to_string()),
            gclba_label: Some(true),
            structure_flag: Some(true),
            condition_grade: Some("Good".to_string()),
            prior_condition_grade: None,
            assessed_value: Some(25000.0),
            sale_date: None,
            program: None,
            facts: Default::default(),
        }];

        let summary = ingest_datawave_records(&records).unwrap();
        assert_eq!(summary.ingested, 1);
        assert_eq!(summary.skipped, 0);
        assert!(summary.fields_written >= 5);
    }
}
