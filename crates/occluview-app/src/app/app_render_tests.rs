#![allow(clippy::expect_used, clippy::float_cmp, clippy::panic)]

/// A deviation map must reach the screen unlit and in its own measured colors,
/// while the brush's paint must keep the scan's own material underneath.
#[test]
fn a_deviation_overlay_forces_unlit_vertex_colors() {
    use glam::Vec3;
    use occluview_core::scene::SceneMesh;
    use occluview_core::{Mesh, OverlayKind, Vertex};

    let mesh = Mesh::new(
        None,
        vec![
            Vertex::at(Vec3::ZERO),
            Vertex::at(Vec3::new(1.0, 0.0, 0.0)),
            Vertex::at(Vec3::new(0.0, 1.0, 0.0)),
        ],
        vec![0, 1, 2],
    )
    .expect("valid mesh");

    let mut entry = SceneMesh::new(mesh);
    entry.show_vertex_colors = false;
    entry.show_texture = true;

    let plain = super::scene_mesh_uniform(&entry);
    assert_eq!(plain.measured_map, 0);
    assert_eq!(plain.overlay_paint, 0);
    assert_eq!(plain.show_vertex_colors, 0);

    let colors = std::sync::Arc::new(vec![[0u8, 0, 0, 255]; 3]);
    let measured = entry
        .clone()
        .with_overlay(OverlayKind::Measured, Some(std::sync::Arc::clone(&colors)));
    let mapped = super::scene_mesh_uniform(&measured);
    assert_eq!(mapped.measured_map, 1, "a deviation map must draw unlit");
    assert_eq!(mapped.overlay_paint, 0);
    assert_eq!(mapped.show_vertex_colors, 1);
    assert_eq!(mapped.show_texture, 0);

    // Paint is not a measurement: the scan keeps its texture and tint, and the
    // marking is mixed over that material instead of replacing the whole scan.
    let paint = entry.with_overlay(OverlayKind::Paint, Some(colors));
    let painted = super::scene_mesh_uniform(&paint);
    assert_eq!(painted.measured_map, 0, "paint must stay lit");
    assert_eq!(painted.overlay_paint, 1);
    assert_eq!(painted.show_vertex_colors, 1);
    assert_eq!(
        painted.show_texture, 1,
        "paint must not hide the scan's texture"
    );
    assert_eq!(
        painted.tint, plain.tint,
        "paint must not drop the scan's tint"
    );
}

/// A failed frame must consume the redraw that asked for it.
///
/// The pending redraw is what makes egui call back; leaving it set after a
/// failure re-enters the same failed submit on every repaint, so a dead GPU
/// spins the UI at 100% and buries the original cause behind the loop. The
/// frame loop must consume the request and short-circuit a latched path rather
/// than asking a device it already knows is broken for another frame.
#[test]
fn a_failed_offscreen_frame_cannot_start_a_repaint_storm() {
    let mut app = crate::app::app_test_support::test_app("offscreen-failure-does-not-spin");
    app.document.scene = Some(crate::app::app_test_support::named_scene("scan", 0.0).into());
    // The state a terminal graphics fault leaves behind.
    app.render.offscreen_failed = true;
    assert!(!app.offscreen_available());

    let ctx = egui::Context::default();
    for frame in 0..3 {
        app.render.invalidation.request_redraw();
        assert!(
            app.render.invalidation.redraw_pending(),
            "frame {frame}: the loop was asked to paint"
        );
        app.render_pending_frame(&ctx);

        assert!(
            !app.render.invalidation.redraw_pending(),
            "frame {frame}: the failed frame must consume its redraw, or the next frame repeats it"
        );
        assert!(
            app.render.offscreen_failed,
            "frame {frame}: the fault stays latched until the operator retries"
        );
        assert!(
            app.ui.status_message.is_none() && app.ui.app_error.is_none(),
            "frame {frame}: the loop short-circuited instead of asking the dead device for another frame"
        );
    }
    assert!(
        app.render.rendered.is_none(),
        "no frame reached the renderer, so nothing was produced from a broken device"
    );
}

/// A readback deadline defers the offscreen path; it does not kill it.
///
/// The deadline measures how long this process was willing to wait, not the
/// health of the device, so it must never latch the path off for the session
/// (that left the section panel showing a previous plane and, with no live
/// viewport, stopped the viewport repainting at all). It is still a failure:
/// the next attempt waits out a backoff so a loaded machine is not asked to
/// fail on every repaint, and then the path must be usable again on its own.
#[test]
fn a_readback_deadline_defers_the_offscreen_path_instead_of_killing_it() {
    let mut app = crate::app::app_test_support::test_app("offscreen-deadline-defers");

    app.note_offscreen_failure(&super::RenderError::ReadbackTimeout {
        timeout: super::APP_OFFSCREEN_RENDER_TIMEOUT,
    });

    assert!(
        !app.render.offscreen_failed,
        "a missed deadline is not a device verdict and must not latch the path off"
    );
    let deadline = app.render.offscreen_retry_after.expect(
        "the next attempt must be deferred, not silently dropped",
    );
    let now = std::time::Instant::now();
    assert!(
        deadline > now,
        "a deferral in the past is no deferral: the failed submit is retried at once"
    );
    assert!(
        deadline <= now + super::OFFSCREEN_RETRY_DELAY,
        "the backoff is bounded by the retry delay"
    );
    assert!(
        !app.offscreen_available(),
        "no frame may use the path while the backoff is running"
    );

    // The wait ends on its own: recovery must not need the operator to restart
    // the viewer or find a dialog.
    app.render.offscreen_retry_after = Some(std::time::Instant::now());
    assert!(
        app.offscreen_available(),
        "once the backoff has elapsed the offscreen path must be usable again"
    );
}

/// The offscreen fault latch must be clearable by the retry the UI offers.
///
/// This replaces a source-text guard that only checked the words in the
/// function. The behaviour it protected is the one that matters and had no
/// other check: on a machine where the offscreen path IS the viewport (a live
/// viewport that failed to come up), the fault dialog is the only surface the
/// operator sees, so a retry that cannot clear the latch leaves a blank
/// viewport for the rest of the session.
#[test]
fn retrying_a_graphics_fault_clears_the_offscreen_latch() {
    let mut app = crate::app::app_test_support::test_app("offscreen-retry-clears-latch");
    // The state the latch is in on a machine whose offscreen path died.
    app.render.offscreen_failed = true;
    app.render.offscreen_retry_after = Some(std::time::Instant::now());
    assert!(
        !app.offscreen_available(),
        "a latched path must be unavailable before the retry"
    );

    let ctx = app.ui.repaint_ctx.clone();
    app.retry_gpu_after_fault(&ctx);

    assert!(
        !app.render.offscreen_failed,
        "the retry must clear the terminal offscreen latch, not leave it set"
    );
    assert!(
        app.render.offscreen_retry_after.is_none(),
        "and the retry backoff with it, or the next attempt is deferred"
    );
    assert!(
        app.offscreen_available(),
        "so the offscreen path is usable again"
    );
}

/// A frame that arrives inside the retry wait must not turn the wait into a
/// session-long latch.
///
/// The wait is exactly the state a frame lands in after a missed deadline, and
/// the decision the frame makes about its own failure has to be the retryable
/// one. A frame that classified "the path is not available yet" as an
/// unclassifiable error latched the path off permanently on the first repaint
/// after the deadline: the section panel showed a previous plane and, with no
/// live viewport, the viewport stopped repainting at all. The operator also
/// must not get a modal per attempt for a failure that is transient by
/// definition, but must still be told the frame failed.
#[test]
fn a_frame_during_the_retry_wait_cannot_latch_the_offscreen_path_off() {
    let mut app = crate::app::app_test_support::test_app("offscreen-retry-wait-no-latch");
    app.document.scene =
        Some(crate::app::app_test_support::named_scene("scan", 0.0).into());
    app.note_offscreen_failure(&super::RenderError::ReadbackTimeout {
        timeout: super::APP_OFFSCREEN_RENDER_TIMEOUT,
    });
    assert!(!app.offscreen_available(), "the retry wait is armed");

    let ctx = egui::Context::default();
    for frame in 0..3 {
        app.render.invalidation.request_redraw();
        assert!(app.render.invalidation.redraw_pending());
        app.render_now(&ctx);

        assert!(
            !app.render.offscreen_failed,
            "frame {frame}: a frame inside the wait must not latch the path off"
        );
        assert!(
            app.render.offscreen_retry_after.is_some(),
            "frame {frame}: and the retry must stay armed"
        );
        assert!(
            app.ui.app_error.is_none(),
            "frame {frame}: a transient failure must not bury the viewport in a modal per attempt"
        );
        assert!(
            app.ui.status_message.is_some(),
            "frame {frame}: the operator is still told the frame failed"
        );
        assert!(
            !app.render.invalidation.redraw_pending(),
            "frame {frame}: the failed frame consumed its redraw"
        );
    }

    app.render.offscreen_retry_after = Some(std::time::Instant::now());
    assert!(
        app.offscreen_available(),
        "and the wait still ends on its own, so nothing is latched off"
    );
}

/// A terminal offscreen fault must raise a dialog that offers the retry.
///
/// On a machine whose offscreen path IS the viewport (a live viewport that
/// failed to come up) this dialog is the only surface the operator sees. A
/// dialog that only reports leaves the latch unreachable from the UI, so the
/// documented recovery is to close the viewer and lose the scene — the same
/// dead end the latch exists to avoid. The action must be the one the dialog
/// handler dispatches, too: the button and the recovery are the same thing.
#[test]
fn the_graphics_fault_dialog_offers_the_retry_action() {
    let mut app = crate::app::app_test_support::test_app("graphics-fault-dialog-offers-retry");
    app.document.scene =
        Some(crate::app::app_test_support::named_scene("scan", 0.0).into());
    // The state a terminal graphics fault leaves behind with no live viewport.
    app.render.offscreen_failed = true;
    assert!(!app.offscreen_available());

    let ctx = egui::Context::default();
    app.render_now(&ctx);

    let dialog = app
        .ui
        .app_error
        .as_ref()
        .expect("a terminal fault with no live viewport must raise the fault dialog");
    assert_eq!(
        dialog.action,
        super::AppErrorAction::RetryGraphics,
        "the dialogue must offer the retry, or the latch has no way out that keeps the scene"
    );
    assert!(
        !dialog.title.is_empty() && !dialog.summary.is_empty() && !dialog.details.is_empty(),
        "the dialogue must say what failed, not only offer the button"
    );

    // What the dialog handler calls when that button is clicked.
    app.retry_gpu_after_fault(&ctx);
    assert!(
        app.offscreen_available(),
        "so pressing it gives the offscreen path back"
    );
}
