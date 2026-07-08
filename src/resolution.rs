use crate::datawave::DatawaveBatchSummary;
use crate::models::{
    AdapterKind, CandidateProperty, CanonicalParcel, ConflictKind, CoverageSummary, Dossier,
    IndexPayload, NormalizedRecord, ParcelExport, SourceConflict, SourceFact, SourceRecord,
};
use crate::normalize::{
    condition_grade_rank, is_gclba_owner, normalize_parcel_id, value_to_string,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub fn build_index(records: Vec<NormalizedRecord>) -> IndexPayload {
    let mut groups: BTreeMap<String, Vec<NormalizedRecord>> = BTreeMap::new();
    for record in records {
        let key = resolution_key(&record);
        groups.entry(key).or_default().push(record);
    }

    let county_parcels: BTreeSet<String> = groups
        .values()
        .flatten()
        .filter(|record| {
            record.source_kind() == Some(AdapterKind::CountyArcgis)
                && (record.gclba_label.unwrap_or(false) || owner_is_gclba(record))
        })
        .filter_map(|record| record.parcel_id_norm.clone())
        .collect();
    let tracked_parcels: BTreeSet<String> = groups
        .values()
        .flatten()
        .filter(|record| record.source_kind() == Some(AdapterKind::ComplianceGraphql))
        .filter_map(|record| record.parcel_id_norm.clone())
        .collect();

    let mut parcels = Vec::new();
    let mut conflicts = Vec::new();
    let mut candidates = Vec::new();

    for (key, mut group) in groups {
        group.sort_by(|a, b| {
            a.source_id
                .cmp(&b.source_id)
                .then(a.source_record_id.cmp(&b.source_record_id))
        });
        let dossier = dossier_for_group(&key, &group);
        let mut group_conflicts =
            conflicts_for_group(&dossier, &group, &county_parcels, &tracked_parcels);
        for conflict in &group_conflicts {
            if conflict.kind == ConflictKind::Candidate {
                candidates.push(candidate_for_conflict(conflict, &dossier));
            }
        }
        conflicts.append(&mut group_conflicts);
        parcels.push(ParcelExport { dossier });
    }

    conflicts.sort_by(|a, b| a.id.cmp(&b.id));
    candidates.sort_by(|a, b| a.id.cmp(&b.id));

    let coverage = coverage_for(&parcels, &conflicts, &candidates);
    IndexPayload {
        coverage,
        parcels,
        conflicts,
        candidates,
    }
}

pub fn attach_datawave_summary(payload: &mut IndexPayload, summary: &DatawaveBatchSummary) {
    payload.coverage.datawave_ingested = summary.ingested;
    payload.coverage.datawave_skipped = summary.skipped;
    payload.coverage.datawave_fields_written = summary.fields_written;
    payload.coverage.datawave_edges_written = summary.edges_written;
}

fn resolution_key(record: &NormalizedRecord) -> String {
    if let Some(parcel) = record.parcel_id.as_deref().and_then(normalize_parcel_id) {
        return format!("parcel:{parcel}");
    }
    if let Some(parcel) = &record.parcel_id_norm {
        return format!("parcel:{parcel}");
    }
    if let Some(address) = &record.address_key {
        return format!("address:{address}");
    }
    format!("source:{}:{}", record.source_id, record.source_record_id)
}

fn dossier_for_group(key: &str, records: &[NormalizedRecord]) -> Dossier {
    let parcel_id = records
        .iter()
        .find_map(|record| record.parcel_id_norm.clone())
        .unwrap_or_else(|| key.to_string());
    let address = records
        .iter()
        .find_map(|record| record.address.clone())
        .unwrap_or_default();
    let canonical = CanonicalParcel {
        owner: records
            .iter()
            .find_map(|record| record.owner_norm.clone())
            .unwrap_or_default(),
        gclba_label: records
            .iter()
            .any(|record| record.gclba_label.unwrap_or(false) || owner_is_gclba(record)),
        structure: if records
            .iter()
            .any(|record| record.structure_flag.unwrap_or(false))
        {
            "home".to_string()
        } else {
            "lot".to_string()
        },
        program: records
            .iter()
            .find_map(|record| record.program.clone())
            .unwrap_or_default(),
        condition: records
            .iter()
            .find_map(|record| record.condition_grade.clone())
            .unwrap_or_default(),
        assessed_value: records.iter().find_map(|record| record.assessed_value),
    };
    Dossier {
        parcel_id,
        address,
        canonical,
        records: records.iter().map(source_record).collect(),
    }
}

fn source_record(record: &NormalizedRecord) -> SourceRecord {
    let mut facts = Vec::new();
    push_fact(&mut facts, "Parcel ID", record.parcel_id_norm.as_deref());
    push_fact(&mut facts, "Address", record.address.as_deref());
    push_fact(&mut facts, "Address Key", record.address_key.as_deref());
    push_fact(&mut facts, "Owner", record.owner_norm.as_deref());
    push_fact(
        &mut facts,
        "GCLBA Label",
        record.gclba_label.map(bool_string).as_deref(),
    );
    push_fact(
        &mut facts,
        "Structure",
        record.structure_flag.map(structure_string).as_deref(),
    );
    push_fact(&mut facts, "Condition", record.condition_grade.as_deref());
    push_fact(
        &mut facts,
        "Prior Condition",
        record.prior_condition_grade.as_deref(),
    );
    push_fact(
        &mut facts,
        "Assessed Value",
        record
            .assessed_value
            .map(|value| format!("{value:.2}"))
            .as_deref(),
    );
    push_fact(&mut facts, "Sale Date", record.sale_date.as_deref());
    push_fact(&mut facts, "Program", record.program.as_deref());

    for (key, value) in &record.facts {
        let rendered = value_to_string(value);
        push_fact(&mut facts, key, Some(rendered.as_str()));
    }
    facts.sort_by(|a, b| a.label.cmp(&b.label).then(a.value.cmp(&b.value)));
    facts.dedup();

    SourceRecord {
        source_id: record.source_id.clone(),
        source_record_id: record.source_record_id.clone(),
        observed_at: record.observed_at.clone(),
        facts,
    }
}

fn conflicts_for_group(
    dossier: &Dossier,
    records: &[NormalizedRecord],
    county_parcels: &BTreeSet<String>,
    tracked_parcels: &BTreeSet<String>,
) -> Vec<SourceConflict> {
    let mut conflicts = Vec::new();
    if let Some(conflict) = candidate_conflict(dossier, records, county_parcels, tracked_parcels) {
        conflicts.push(conflict);
    }
    if let Some(conflict) = owner_mismatch(dossier, records) {
        conflicts.push(conflict);
    }
    if let Some(conflict) = reverse_mismatch(dossier, records) {
        conflicts.push(conflict);
    }
    conflicts.extend(pid_orphans(records, county_parcels));
    if let Some(conflict) = condition_regression(dossier, records) {
        conflicts.push(conflict);
    }
    if let Some(conflict) = value_disagreement(dossier, records) {
        conflicts.push(conflict);
    }
    conflicts
}

fn candidate_conflict(
    dossier: &Dossier,
    records: &[NormalizedRecord],
    county_parcels: &BTreeSet<String>,
    tracked_parcels: &BTreeSet<String>,
) -> Option<SourceConflict> {
    if county_parcels.is_empty() {
        return None;
    }
    let portal = records.iter().find(|record| {
        record.source_kind() == Some(AdapterKind::SiteControlExport)
            && record.parcel_id_norm.is_some()
            && (record.gclba_label.unwrap_or(false) || owner_is_gclba(record))
    })?;
    let parcel_id = portal.parcel_id_norm.as_ref()?;
    if county_parcels.contains(parcel_id) || tracked_parcels.contains(parcel_id) {
        return None;
    }
    Some(conflict(
        dossier,
        ConflictKind::Candidate,
        "review",
        "Portal parcel is not in county or compliance tracking",
        "The portal labels this parcel as land bank owned, but it is absent from the current county land bank layer and absent from compliance tracking.",
        vec![
            format!("Portal source record {}", portal.source_record_id),
            format!("Parcel {}", parcel_id),
        ],
    ))
}

fn owner_mismatch(dossier: &Dossier, records: &[NormalizedRecord]) -> Option<SourceConflict> {
    let portal = records.iter().find(|record| {
        record.source_kind() == Some(AdapterKind::SiteControlExport)
            && (record.gclba_label.unwrap_or(false) || owner_is_gclba(record))
    })?;
    let county = records.iter().find(|record| {
        record.source_kind() == Some(AdapterKind::CountyArcgis) && !owner_is_gclba(record)
    })?;
    Some(conflict(
        dossier,
        ConflictKind::OwnerMismatch,
        "review",
        "Portal and county ownership disagree",
        "The portal labels this parcel as land bank owned, while the county record lists a private owner.",
        vec![
            format!("Portal source record {}", portal.source_record_id),
            format!("County owner {}", county.owner_norm.clone().unwrap_or_default()),
        ],
    ))
}

fn reverse_mismatch(dossier: &Dossier, records: &[NormalizedRecord]) -> Option<SourceConflict> {
    let county = records.iter().find(|record| {
        record.source_kind() == Some(AdapterKind::CountyArcgis) && owner_is_gclba(record)
    })?;
    let portal = records.iter().find(|record| {
        record.source_kind() == Some(AdapterKind::SiteControlExport)
            && !record.gclba_label.unwrap_or(false)
            && !owner_is_gclba(record)
    })?;
    Some(conflict(
        dossier,
        ConflictKind::ReverseMismatch,
        "review",
        "County and portal ownership disagree",
        "The county record lists land bank ownership, while the portal does not.",
        vec![
            format!("County source record {}", county.source_record_id),
            format!("Portal source record {}", portal.source_record_id),
        ],
    ))
}

fn pid_orphans(
    records: &[NormalizedRecord],
    county_parcels: &BTreeSet<String>,
) -> Vec<SourceConflict> {
    if county_parcels.is_empty() {
        return Vec::new();
    }
    records
        .iter()
        .filter(|record| record.source_kind() != Some(AdapterKind::CountyArcgis))
        .filter_map(|record| {
            let parcel_id = record.parcel_id_norm.as_ref()?;
            (!county_parcels.contains(parcel_id)).then(|| {
                let dossier = Dossier {
                    parcel_id: parcel_id.clone(),
                    address: record.address.clone().unwrap_or_default(),
                    ..Dossier::default()
                };
                conflict(
                    &dossier,
                    ConflictKind::PidOrphan,
                    "review",
                    "Parcel id was not found in county records",
                    "A source record has a normalized parcel id that is absent from the county parcel set.",
                    vec![format!(
                        "{} source record {}",
                        record.source_id, record.source_record_id
                    )],
                )
            })
        })
        .collect()
}

fn condition_regression(dossier: &Dossier, records: &[NormalizedRecord]) -> Option<SourceConflict> {
    let record = records.iter().find(|record| {
        let Some(current) = record.condition_grade.as_deref() else {
            return false;
        };
        let Some(prior) = record.prior_condition_grade.as_deref() else {
            return false;
        };
        match (condition_grade_rank(current), condition_grade_rank(prior)) {
            (Some(current_rank), Some(prior_rank)) => current_rank > prior_rank,
            _ => false,
        }
    })?;
    Some(conflict(
        dossier,
        ConflictKind::ConditionRegression,
        "review",
        "Condition appears to have worsened",
        "A source reports a worse condition grade than the prior recorded grade.",
        vec![format!(
            "{} changed from {} to {}",
            record.source_id,
            record.prior_condition_grade.clone().unwrap_or_default(),
            record.condition_grade.clone().unwrap_or_default()
        )],
    ))
}

fn value_disagreement(dossier: &Dossier, records: &[NormalizedRecord]) -> Option<SourceConflict> {
    let values: Vec<f64> = records
        .iter()
        .filter_map(|record| record.assessed_value)
        .filter(|value| *value > 0.0)
        .collect();
    if values.len() < 2 {
        return None;
    }
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let diff = max - min;
    if diff >= 10_000.0 && max / min.max(1.0) >= 1.2 {
        Some(conflict(
            dossier,
            ConflictKind::ValueDisagreement,
            "review",
            "Assessed values disagree across sources",
            "Two source records report materially different assessed values for this parcel.",
            vec![
                format!("Lowest value {min:.2}"),
                format!("Highest value {max:.2}"),
            ],
        ))
    } else {
        None
    }
}

fn conflict(
    dossier: &Dossier,
    kind: ConflictKind,
    severity: &str,
    title: &str,
    plain_language: &str,
    evidence: Vec<String>,
) -> SourceConflict {
    let id = stable_id(&[
        dossier.parcel_id.as_str(),
        kind.as_str(),
        title,
        evidence.first().map(String::as_str).unwrap_or(""),
    ]);
    SourceConflict {
        id,
        parcel_id: dossier.parcel_id.clone(),
        kind,
        severity: severity.to_string(),
        title: title.to_string(),
        plain_language: plain_language.to_string(),
        evidence,
        observed_at: "1970-01-01".to_string(),
        status: "open".to_string(),
        metadata: BTreeMap::from([(
            "address".to_string(),
            Value::String(dossier.address.clone()),
        )]),
    }
}

fn candidate_for_conflict(conflict: &SourceConflict, dossier: &Dossier) -> CandidateProperty {
    CandidateProperty {
        id: stable_id(&[conflict.id.as_str(), "candidate"]),
        parcel_id: conflict.parcel_id.clone(),
        reason: "Portal parcel is absent from county and compliance tracking".to_string(),
        source_conflict_id: conflict.id.clone(),
        address: dossier.address.clone(),
        evidence: conflict.evidence.clone(),
        status: "queued".to_string(),
    }
}

fn coverage_for(
    parcels: &[ParcelExport],
    conflicts: &[SourceConflict],
    candidates: &[CandidateProperty],
) -> CoverageSummary {
    let mut sources = BTreeMap::new();
    for parcel in parcels {
        for record in &parcel.dossier.records {
            *sources.entry(record.source_id.clone()).or_insert(0) += 1;
        }
    }
    CoverageSummary {
        parcels_indexed: parcels.len(),
        home_count: parcels
            .iter()
            .filter(|parcel| parcel.dossier.canonical.structure == "home")
            .count(),
        active_program_count: parcels
            .iter()
            .filter(|parcel| !parcel.dossier.canonical.program.is_empty())
            .count(),
        source_count: sources.len(),
        open_conflict_count: conflicts
            .iter()
            .filter(|conflict| conflict.status == "open")
            .count(),
        candidate_count: candidates.len(),
        per_source: sources,
        datawave_ingested: 0,
        datawave_skipped: 0,
        datawave_fields_written: 0,
        datawave_edges_written: 0,
    }
}

fn push_fact(facts: &mut Vec<SourceFact>, label: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        facts.push(SourceFact {
            label: label.to_string(),
            value: value.to_string(),
        });
    }
}

fn owner_is_gclba(record: &NormalizedRecord) -> bool {
    record.owner_norm.as_deref().is_some_and(is_gclba_owner)
}

fn bool_string(value: bool) -> String {
    if value {
        "true".to_string()
    } else {
        "false".to_string()
    }
}

fn structure_string(value: bool) -> String {
    if value {
        "home".to_string()
    } else {
        "lot".to_string()
    }
}

fn stable_id(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.trim().to_ascii_lowercase().as_bytes());
        hasher.update(b"|");
    }
    let digest = hasher.finalize();
    digest
        .iter()
        .take(12)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::AdapterKind;

    fn record(
        source: AdapterKind,
        parcel: &str,
        owner: &str,
        gclba: Option<bool>,
    ) -> NormalizedRecord {
        NormalizedRecord {
            source_id: source.as_str().to_string(),
            source_record_id: format!("{}:{parcel}", source.as_str()),
            observed_at: "2026-07-07".to_string(),
            parcel_id: Some(parcel.to_string()),
            parcel_id_norm: normalize_parcel_id(parcel),
            address: Some("123 North Saginaw Street".to_string()),
            address_key: Some("123 N SAGINAW ST".to_string()),
            owner_norm: Some(owner.to_string()),
            gclba_label: gclba,
            structure_flag: Some(true),
            condition_grade: Some("Good".to_string()),
            prior_condition_grade: None,
            assessed_value: Some(20_000.0),
            sale_date: None,
            program: None,
            facts: BTreeMap::new(),
        }
    }

    #[test]
    fn seeded_owner_mismatch_yields_owner_conflict_without_candidate() {
        let payload = build_index(vec![
            record(
                AdapterKind::SiteControlExport,
                "40-12-345-067",
                "GCLBA",
                Some(true),
            ),
            record(
                AdapterKind::CountyArcgis,
                "40-12-345-067",
                "PRIVATE OWNER",
                Some(false),
            ),
        ]);
        let owner_mismatches: Vec<_> = payload
            .conflicts
            .iter()
            .filter(|conflict| conflict.kind == ConflictKind::OwnerMismatch)
            .collect();
        assert_eq!(owner_mismatches.len(), 1);
        assert_eq!(payload.candidates.len(), 0);
        assert_eq!(payload.coverage.parcels_indexed, 1);
        assert_eq!(payload.coverage.home_count, 1);
    }

    #[test]
    fn portal_parcel_absent_from_county_and_compliance_yields_candidate() {
        let portal_candidate = record(
            AdapterKind::SiteControlExport,
            "40-12-345-067",
            "GCLBA",
            Some(true),
        );
        let county_other = record(
            AdapterKind::CountyArcgis,
            "40-12-345-068",
            "GCLBA",
            Some(true),
        );

        let payload = build_index(vec![portal_candidate, county_other]);
        let candidates: Vec<_> = payload
            .conflicts
            .iter()
            .filter(|conflict| conflict.kind == ConflictKind::Candidate)
            .collect();
        assert_eq!(candidates.len(), 1);
        assert_eq!(payload.candidates.len(), 1);
        assert_eq!(payload.coverage.candidate_count, 1);
    }

    #[test]
    fn all_conflict_kinds_are_detectable() {
        let mut portal_owner = record(
            AdapterKind::SiteControlExport,
            "40-12-345-067",
            "GCLBA",
            Some(true),
        );
        portal_owner.condition_grade = Some("Poor".to_string());
        portal_owner.prior_condition_grade = Some("Good".to_string());
        portal_owner.assessed_value = Some(10_000.0);

        let mut county_private = record(
            AdapterKind::CountyArcgis,
            "40-12-345-067",
            "PRIVATE OWNER",
            Some(false),
        );
        county_private.assessed_value = Some(50_000.0);

        let county_gclba = record(
            AdapterKind::CountyArcgis,
            "40-12-345-068",
            "GCLBA",
            Some(true),
        );
        let portal_private = record(
            AdapterKind::SiteControlExport,
            "40-12-345-068",
            "PRIVATE OWNER",
            Some(false),
        );
        let orphan = record(
            AdapterKind::FilemakerCsv,
            "40-12-345-099",
            "PRIVATE OWNER",
            Some(false),
        );
        let portal_candidate = record(
            AdapterKind::SiteControlExport,
            "40-12-345-100",
            "GCLBA",
            Some(true),
        );

        let payload = build_index(vec![
            portal_owner,
            county_private,
            county_gclba,
            portal_private,
            orphan,
            portal_candidate,
        ]);
        let kinds: BTreeSet<_> = payload
            .conflicts
            .iter()
            .map(|conflict| conflict.kind)
            .collect();
        for kind in [
            ConflictKind::OwnerMismatch,
            ConflictKind::Candidate,
            ConflictKind::ReverseMismatch,
            ConflictKind::PidOrphan,
            ConflictKind::ConditionRegression,
            ConflictKind::ValueDisagreement,
        ] {
            assert!(kinds.contains(&kind), "{kind:?} was not detected");
        }
    }
}
