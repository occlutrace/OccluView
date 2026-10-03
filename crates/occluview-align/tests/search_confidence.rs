//! ID29 regression: incomplete evidence cannot certify a physical scale conflict.
#![allow(clippy::unwrap_used)]
mod support;
use occluview_align::{Confidence, SearchControl, SearchSettings};

#[test]
fn incomplete_scale_conflict_cannot_publish_probable() {
    let spec = support::arch::ArchSpec {
        grid: [40, 10],
        asymmetry_seed: 1,
        ..support::arch::ArchSpec::default()
    };
    let fixed = support::arch::dental_arch(&spec);
    let moving =
        support::operators::scale_geometry(&support::operators::resample_density(&fixed, 1.), 1.1);
    let result = occluview_align::search_alignment(
        &support::alignment_input(moving.soup(), fixed.soup()),
        &SearchSettings::default(),
        &SearchControl::default(),
    )
    .unwrap();
    assert!(!result.candidates.is_empty());
    assert!(result.candidates.iter().all(|candidate| {
        candidate.pose.is_finite()
            && matches!(
                candidate.confidence,
                Confidence::Weak | Confidence::Ambiguous
            )
    }));
}
