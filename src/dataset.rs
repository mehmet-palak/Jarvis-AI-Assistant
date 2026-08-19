//! F6 "Dataset export/versioning".
//!
//! Turns the reviewed, verifier-passed teacher examples in the local store into a versioned,
//! self-describing dataset artifact. Three properties the F6 plan asks for drive the design:
//!
//! - **Only eligible examples leave the machine.** Export is not a database dump: an example
//!   must be human-reviewed, verifier-`PASS` and non-`Sensitive`. Everything else is reported as
//!   excluded, with a reason, rather than silently dropped — a dataset you cannot explain the
//!   contents of is not governable.
//! - **Deletion and poisoning are recorded, not erased.** A removed or poisoned example leaves a
//!   marker carrying its id and reason. Deleting the row instead would let the same content be
//!   re-ingested later as if it had never been rejected, which is exactly how a poisoned example
//!   comes back.
//! - **The manifest is content-addressed.** The manifest hash covers every exported record *and*
//!   every marker, so two exports are identical if and only if their governed content is
//!   identical. That is what makes "which dataset was this model trained on" answerable later.

use crate::{sha256_hex, DataSensitivity, TeacherExample, VerifyStatus};

/// Why an example present in the store did not make it into the export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetExclusion {
    pub example_id: String,
    pub reason: String,
}

/// A durable "this example must never be used" record. Kept in the manifest so a consumer of the
/// dataset can see that the id was deliberately withheld rather than merely absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetMarker {
    pub example_id: String,
    pub kind: DatasetMarkerKind,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatasetMarkerKind {
    /// The user asked for this example to be removed (deletion request).
    Deleted,
    /// The example is believed to be harmful or manipulated training data.
    Poisoned,
}

impl DatasetMarkerKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            DatasetMarkerKind::Deleted => "deleted",
            DatasetMarkerKind::Poisoned => "poisoned",
        }
    }
}

/// One exported dataset version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetExport {
    pub schema_version: u16,
    /// Monotonic dataset version supplied by the caller — the identity a trained model refers to.
    pub dataset_version: u32,
    pub records: Vec<TeacherExample>,
    pub excluded: Vec<DatasetExclusion>,
    pub markers: Vec<DatasetMarker>,
    /// SHA-256 over the canonical serialization of every record and marker.
    pub manifest_hash: String,
}

impl DatasetExport {
    /// Canonical, stable text form. Deliberately built by hand rather than with a serializer so
    /// the hashed bytes cannot change underneath us when a dependency changes its formatting —
    /// a manifest hash that drifts for a non-content reason would be worse than no hash.
    pub fn to_manifest_text(&self) -> String {
        let mut lines = vec![
            format!("dataset_schema_version\t{}", self.schema_version),
            format!("dataset_version\t{}", self.dataset_version),
        ];
        for record in &self.records {
            lines.push(format!(
                "record\t{}\t{}\t{}\t{}\t{}",
                record.example_id,
                record.expected_capability,
                record.sensitivity.as_str(),
                sha256_hex(&record.prompt),
                sha256_hex(&record.response),
            ));
        }
        for marker in &self.markers {
            lines.push(format!(
                "marker\t{}\t{}\t{}",
                marker.example_id,
                marker.kind.as_str(),
                marker.reason
            ));
        }
        lines.join("\n")
    }
}

/// Why an example is or is not eligible for export. Split out so the rule is stated once and can
/// be unit-tested directly, instead of being embedded in the export loop.
fn export_exclusion_reason(example: &TeacherExample) -> Option<String> {
    if !example.human_reviewed {
        return Some("not human-reviewed".into());
    }
    if example.verifier_status != VerifyStatus::Pass {
        return Some("verifier status is not PASS".into());
    }
    if example.sensitivity == DataSensitivity::Sensitive {
        return Some("sensitivity is SENSITIVE".into());
    }
    None
}

/// Builds a dataset export from every stored example plus the caller's markers.
///
/// `markers` win over eligibility: a marked example is always excluded, whatever its other
/// fields say. That ordering is the point — a poisoned example that happens to look
/// well-formed must not be exportable.
pub fn build_dataset_export(
    dataset_version: u32,
    examples: &[TeacherExample],
    markers: &[DatasetMarker],
) -> DatasetExport {
    let mut records = Vec::new();
    let mut excluded = Vec::new();

    for example in examples {
        if let Some(marker) = markers
            .iter()
            .find(|marker| marker.example_id == example.example_id)
        {
            excluded.push(DatasetExclusion {
                example_id: example.example_id.clone(),
                reason: format!("{} marker: {}", marker.kind.as_str(), marker.reason),
            });
            continue;
        }
        match export_exclusion_reason(example) {
            Some(reason) => excluded.push(DatasetExclusion {
                example_id: example.example_id.clone(),
                reason,
            }),
            None => records.push(example.clone()),
        }
    }

    let mut export = DatasetExport {
        schema_version: 1,
        dataset_version,
        records,
        excluded,
        markers: markers.to_vec(),
        manifest_hash: String::new(),
    };
    export.manifest_hash = sha256_hex(&export.to_manifest_text());
    export
}

#[cfg(test)]
#[path = "dataset_tests.rs"]
mod tests;

/// F6 "Old-vs-new regresyonu": the verdict on one configuration change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelConfigComparison {
    pub previous_run_id: String,
    pub current_run_id: String,
    pub previous_passed: u32,
    pub current_passed: u32,
    pub previous_median_latency_ms: u64,
    pub current_median_latency_ms: u64,
    pub verdict: ModelConfigVerdict,
    /// Plain-language reason, so a rollback decision never rests on a bare enum.
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelConfigVerdict {
    Improved,
    Unchanged,
    /// Lost at least one scenario, or got materially slower with no quality gain.
    Regressed,
}

/// The latency increase (as a ratio of the previous median) that counts as a regression on its
/// own when quality did not improve. Well above normal run-to-run noise, so an ordinary
/// fluctuation never triggers a rollback recommendation.
const LATENCY_REGRESSION_RATIO: f64 = 1.5;

pub fn compare_model_config_runs(
    previous: &crate::ModelConfigRun,
    current: &crate::ModelConfigRun,
) -> ModelConfigComparison {
    let latency_ratio = if previous.median_latency_ms == 0 {
        1.0
    } else {
        current.median_latency_ms as f64 / previous.median_latency_ms as f64
    };

    let (verdict, reason) = if current.scenarios_passed < previous.scenarios_passed {
        (
            ModelConfigVerdict::Regressed,
            format!(
                "senaryo kaybı: {} → {} (hız kazancı bunu telafi etmez)",
                previous.scenarios_passed, current.scenarios_passed
            ),
        )
    } else if current.scenarios_passed > previous.scenarios_passed {
        (
            ModelConfigVerdict::Improved,
            format!(
                "daha fazla senaryo geçti: {} → {}",
                previous.scenarios_passed, current.scenarios_passed
            ),
        )
    } else if latency_ratio >= LATENCY_REGRESSION_RATIO {
        (
            ModelConfigVerdict::Regressed,
            format!(
                "kalite aynı ama belirgin yavaşlama: {} ms → {} ms",
                previous.median_latency_ms, current.median_latency_ms
            ),
        )
    } else if current.median_latency_ms < previous.median_latency_ms {
        (
            ModelConfigVerdict::Improved,
            format!(
                "kalite aynı, daha hızlı: {} ms → {} ms",
                previous.median_latency_ms, current.median_latency_ms
            ),
        )
    } else {
        (
            ModelConfigVerdict::Unchanged,
            "kalite ve gecikme pratikte aynı".to_string(),
        )
    };

    ModelConfigComparison {
        previous_run_id: previous.run_id.clone(),
        current_run_id: current.run_id.clone(),
        previous_passed: previous.scenarios_passed,
        current_passed: current.scenarios_passed,
        previous_median_latency_ms: previous.median_latency_ms,
        current_median_latency_ms: current.median_latency_ms,
        verdict,
        reason,
    }
}
