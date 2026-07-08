use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterKind {
    SiteControlExport,
    FilemakerCsv,
    CountyArcgis,
    ComplianceGraphql,
    BsaTax,
}

impl AdapterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AdapterKind::SiteControlExport => "site_control_export",
            AdapterKind::FilemakerCsv => "filemaker_csv",
            AdapterKind::CountyArcgis => "county_arcgis",
            AdapterKind::ComplianceGraphql => "compliance_graphql",
            AdapterKind::BsaTax => "bsa_tax",
        }
    }
}

impl FromStr for AdapterKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "site_control" | "site_control_export" | "regrid" | "portal" => {
                Ok(Self::SiteControlExport)
            }
            "filemaker" | "filemaker_csv" | "disposition" => Ok(Self::FilemakerCsv),
            "county" | "county_arcgis" | "arcgis" => Ok(Self::CountyArcgis),
            "compliance" | "compliance_graphql" | "django" => Ok(Self::ComplianceGraphql),
            "bsa" | "bsa_tax" => Ok(Self::BsaTax),
            other => Err(format!("unknown adapter {other:?}")),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedRecord {
    pub source_id: String,
    pub source_record_id: String,
    pub observed_at: String,
    pub parcel_id: Option<String>,
    pub parcel_id_norm: Option<String>,
    pub address: Option<String>,
    pub address_key: Option<String>,
    pub owner_norm: Option<String>,
    pub gclba_label: Option<bool>,
    pub structure_flag: Option<bool>,
    pub condition_grade: Option<String>,
    pub prior_condition_grade: Option<String>,
    pub assessed_value: Option<f64>,
    pub sale_date: Option<String>,
    pub program: Option<String>,
    #[serde(default)]
    pub facts: BTreeMap<String, Value>,
}

impl NormalizedRecord {
    pub fn source_kind(&self) -> Option<AdapterKind> {
        self.source_id.parse().ok()
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexPayload {
    pub coverage: CoverageSummary,
    pub parcels: Vec<ParcelExport>,
    pub conflicts: Vec<SourceConflict>,
    pub candidates: Vec<CandidateProperty>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageSummary {
    pub parcels_indexed: usize,
    pub home_count: usize,
    pub active_program_count: usize,
    pub source_count: usize,
    pub open_conflict_count: usize,
    pub candidate_count: usize,
    pub per_source: BTreeMap<String, usize>,
    pub datawave_ingested: usize,
    pub datawave_skipped: usize,
    pub datawave_fields_written: usize,
    pub datawave_edges_written: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParcelExport {
    pub dossier: Dossier,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dossier {
    pub parcel_id: String,
    pub address: String,
    pub canonical: CanonicalParcel,
    pub records: Vec<SourceRecord>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalParcel {
    pub owner: String,
    pub gclba_label: bool,
    pub structure: String,
    pub program: String,
    pub condition: String,
    pub assessed_value: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRecord {
    pub source_id: String,
    pub source_record_id: String,
    pub observed_at: String,
    pub facts: Vec<SourceFact>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SourceFact {
    pub label: String,
    pub value: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    Candidate,
    OwnerMismatch,
    ReverseMismatch,
    PidOrphan,
    ConditionRegression,
    ValueDisagreement,
}

impl ConflictKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ConflictKind::Candidate => "candidate",
            ConflictKind::OwnerMismatch => "owner_mismatch",
            ConflictKind::ReverseMismatch => "reverse_mismatch",
            ConflictKind::PidOrphan => "pid_orphan",
            ConflictKind::ConditionRegression => "condition_regression",
            ConflictKind::ValueDisagreement => "value_disagreement",
        }
    }
}

impl FromStr for ConflictKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "candidate" => Ok(Self::Candidate),
            "owner_mismatch" => Ok(Self::OwnerMismatch),
            "reverse_mismatch" => Ok(Self::ReverseMismatch),
            "pid_orphan" => Ok(Self::PidOrphan),
            "condition_regression" => Ok(Self::ConditionRegression),
            "value_disagreement" => Ok(Self::ValueDisagreement),
            other => Err(format!("unknown conflict kind {other:?}")),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceConflict {
    pub id: String,
    pub parcel_id: String,
    pub kind: ConflictKind,
    pub severity: String,
    pub title: String,
    pub plain_language: String,
    pub evidence: Vec<String>,
    pub observed_at: String,
    pub status: String,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateProperty {
    pub id: String,
    pub parcel_id: String,
    pub reason: String,
    pub source_conflict_id: String,
    pub address: String,
    pub evidence: Vec<String>,
    pub status: String,
}
