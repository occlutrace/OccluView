use super::*;

fn frame(center: Vec3, radius_mm: f32) -> SectionViewFrame {
    SectionViewFrame {
        pose: DiscPose {
            center,
            plane_normal: Vec3::Z,
            radius_mm,
        },
        normal: Vec3::Z,
    }
}

#[test]
fn passive_section_tracks_external_pose_without_owning_a_manipulator() {
    let mut view = SectionView::default();
    let first = frame(Vec3::ZERO, 4.0);
    assert!(view.sync(Some(first)));
    assert!(
        view.slice_visible(),
        "Lines mode is live without a GPU render"
    );
    assert_eq!(view.frame(), Some(first));

    let moved = frame(Vec3::new(1.0, 0.0, 0.0), 4.0);
    assert!(view.sync(Some(moved)));
    assert_eq!(view.frame(), Some(moved));
}

#[test]
fn passive_section_clears_its_pose_when_the_owner_has_none() {
    let mut view = SectionView::default();
    view.sync(Some(frame(Vec3::ZERO, 4.0)));
    assert!(view.sync(None));
    assert!(!view.slice_visible());
    assert!(view.frame().is_none());
}
