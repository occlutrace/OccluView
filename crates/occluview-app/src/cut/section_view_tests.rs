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

#[test]
fn section_basis_follows_primary_camera_axes() {
    let main_view = SectionMainView {
        right: -Vec3::X,
        up: Vec3::Y,
    };
    let basis = main_view.slice_basis(Vec3::Z);

    assert!((basis.right + Vec3::X).length() < 1.0e-5);
    assert!((basis.up - Vec3::Y).length() < 1.0e-5);
    assert!(basis.right.dot(Vec3::Z).abs() < 1.0e-5);
    assert!(basis.up.dot(Vec3::Z).abs() < 1.0e-5);
}

#[test]
fn mesh_section_requeues_the_same_panel_render_after_camera_rotation() {
    let mut view = SectionView::default();
    view.sync(Some(frame(Vec3::ZERO, 4.0)));
    view.set_display_mode(SectionDisplay::Mesh);
    assert!(view.needs_render());
    let _ = view.take_needs_render();

    let rotated = SectionMainView {
        right: Vec3::Y,
        up: Vec3::X,
    };
    assert!(view.sync_main_view(rotated));
    assert!(view.needs_render());
}
