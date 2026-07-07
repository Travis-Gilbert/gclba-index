mod compliance_graphql;
mod filemaker;
mod site_control;

use crate::models::{AdapterKind, NormalizedRecord};
use anyhow::Result;

pub use compliance_graphql::ComplianceGraphqlAdapter;
pub use filemaker::FilemakerCsvAdapter;
pub use site_control::SiteControlExportAdapter;

pub trait Adapter {
    fn kind(&self) -> AdapterKind;
    fn parse_bytes(&self, bytes: &[u8]) -> Result<Vec<NormalizedRecord>>;
}

pub fn parse_adapter_bytes(kind: AdapterKind, bytes: &[u8]) -> Result<Vec<NormalizedRecord>> {
    match kind {
        AdapterKind::SiteControlExport => SiteControlExportAdapter::default().parse_bytes(bytes),
        AdapterKind::FilemakerCsv => FilemakerCsvAdapter.parse_bytes(bytes),
        AdapterKind::CountyArcgis => SiteControlExportAdapter::county_arcgis().parse_bytes(bytes),
        AdapterKind::ComplianceGraphql => ComplianceGraphqlAdapter.parse_bytes(bytes),
        AdapterKind::BsaTax => Ok(Vec::new()),
    }
}
