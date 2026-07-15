//! Launch-time update check and the "update available" notice.
//!
//! Policy (owner decision): OFFER, never install silently. On launch a
//! background thread fetches the signed manifest once; if a newer release
//! exists, a quiet corner notice offers Download → Install & restart. All
//! network and verification work lives in `occluview-update`; this module is
//! only the state machine + egui glue. `OCCLUVIEW_NO_UPDATE_CHECK=1` disables
//! the check entirely (packagers/clinics).

use std::path::PathBuf;
use std::sync::mpsc;

use eframe::egui;
use occluview_update::AvailableUpdate;

/// Path of the "skip this version" marker; one semver string, plain text.
fn skipped_version_path() -> Option<PathBuf> {
    crate::app_paths::app_state_dir().map(|dir| dir.join("skipped-update"))
}

fn load_skipped_version() -> Option<String> {
    let path = skipped_version_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn store_skipped_version(version: &str) {
    let Some(path) = skipped_version_path() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, version);
}

enum DownloadEvent {
    Progress(u64, Option<u64>),
    Done(PathBuf),
    Failed(String),
}

enum Phase {
    Idle,
    Available(AvailableUpdate),
    Downloading {
        update: AvailableUpdate,
        received: u64,
        total: Option<u64>,
    },
    Ready {
        update: AvailableUpdate,
        installer: PathBuf,
    },
    Failed(String),
    Dismissed,
}

/// Launch-time update notice state; owned by the app, drawn every frame.
pub(crate) struct UpdateNotice {
    phase: Phase,
    check_rx: Option<mpsc::Receiver<Option<AvailableUpdate>>>,
    download_rx: Option<mpsc::Receiver<DownloadEvent>>,
}

impl UpdateNotice {
    /// Start the once-per-launch background check (unless disabled by env).
    pub(crate) fn begin_check() -> Self {
        let mut notice = Self {
            phase: Phase::Idle,
            check_rx: None,
            download_rx: None,
        };
        if std::env::var_os("OCCLUVIEW_NO_UPDATE_CHECK").is_some() {
            return notice;
        }
        let (tx, rx) = mpsc::channel();
        notice.check_rx = Some(rx);
        std::thread::Builder::new()
            .name("occluview-update-check".to_string())
            .spawn(move || {
                // Errors mean "no update today": never bother the operator
                // over a flaky network or a missing manifest.
                let found = occluview_update::check_for_update(env!("CARGO_PKG_VERSION"))
                    .unwrap_or(None)
                    // An explicitly skipped version stays quiet; anything NEWER
                    // than the skipped one is offered again.
                    .filter(|update| load_skipped_version().as_deref() != Some(update.version.to_string().as_str()));
                let _ = tx.send(found);
            })
            .ok();
        notice
    }

    /// Drain worker events and draw the notice when there is one.
    pub(crate) fn show(&mut self, ctx: &egui::Context) {
        self.drain_events(ctx);
        match &self.phase {
            Phase::Idle | Phase::Dismissed => {}
            _ => self.draw_window(ctx),
        }
    }

    fn drain_events(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.check_rx {
            if let Ok(result) = rx.try_recv() {
                self.check_rx = None;
                if let Some(update) = result {
                    self.phase = Phase::Available(update);
                    ctx.request_repaint();
                }
            }
        }
        if let Some(rx) = &self.download_rx {
            let mut finished = false;
            while let Ok(event) = rx.try_recv() {
                match event {
                    DownloadEvent::Progress(received, total) => {
                        if let Phase::Downloading {
                            received: current_received,
                            total: current_total,
                            ..
                        } = &mut self.phase
                        {
                            *current_received = received;
                            *current_total = total;
                        }
                    }
                    DownloadEvent::Done(installer) => {
                        if let Phase::Downloading { update, .. } = &self.phase {
                            self.phase = Phase::Ready {
                                update: update.clone(),
                                installer,
                            };
                        }
                        finished = true;
                    }
                    DownloadEvent::Failed(message) => {
                        self.phase = Phase::Failed(message);
                        finished = true;
                    }
                }
                ctx.request_repaint();
            }
            if finished {
                self.download_rx = None;
            }
        }
        // A download in flight animates a progress bar: keep frames coming.
        if matches!(self.phase, Phase::Downloading { .. }) {
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
        }
    }

    fn draw_window(&mut self, ctx: &egui::Context) {
        let mut next_phase: Option<Phase> = None;
        let mut start_download: Option<AvailableUpdate> = None;
        egui::Window::new("occluview-update-notice")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -16.0))
            .show(ctx, |ui| {
                ui.set_max_width(300.0);
                match &self.phase {
                    Phase::Available(update) => {
                        draw_available(ui, update, &mut start_download, &mut next_phase);
                    }
                    Phase::Downloading {
                        update,
                        received,
                        total,
                    } => draw_downloading(ui, update, *received, *total),
                    Phase::Ready { update, installer } => {
                        draw_ready(ui, ctx, update, installer, &mut next_phase);
                    }
                    Phase::Failed(message) => draw_failed(ui, message, &mut next_phase),
                    Phase::Idle | Phase::Dismissed => {}
                }
            });
        if let Some(update) = start_download {
            self.phase = self.start_download(update);
        } else if let Some(phase) = next_phase {
            self.phase = phase;
        }
    }

    fn start_download(&mut self, update: AvailableUpdate) -> Phase {
        let (tx, rx) = mpsc::channel();
        self.download_rx = Some(rx);
        let worker_update = update.clone();
        std::thread::Builder::new()
            .name("occluview-update-download".to_string())
            .spawn(move || {
                let dest = std::env::temp_dir().join("occluview-updates");
                let mut report = |received, total| {
                    let _ = tx.send(DownloadEvent::Progress(received, total));
                };
                match occluview_update::download_update(&worker_update, &dest, &mut report) {
                    Ok(installer) => {
                        let _ = tx.send(DownloadEvent::Done(installer));
                    }
                    Err(error) => {
                        let _ = tx.send(DownloadEvent::Failed(error.to_string()));
                    }
                }
            })
            .ok();
        Phase::Downloading {
            update,
            received: 0,
            total: None,
        }
    }
}

fn draw_available(
    ui: &mut egui::Ui,
    update: &AvailableUpdate,
    start_download: &mut Option<AvailableUpdate>,
    next_phase: &mut Option<Phase>,
) {
    ui.label(egui::RichText::new(format!("OccluView {} is available", update.version)).strong());
    ui.label(
        egui::RichText::new(format!("You are on {}.", env!("CARGO_PKG_VERSION")))
            .weak()
            .size(11.0),
    );
    if let Some(notes) = update.notes.as_deref() {
        if !notes.trim().is_empty() {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(notes.trim()).size(11.0));
        }
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if update.downloadable() {
            if ui.button("Download update").clicked() {
                *start_download = Some(update.clone());
            }
        } else {
            // The release exists but publishes no installer for this
            // platform: point at the release page instead of pretending.
            ui.hyperlink_to(
                "Open release page",
                "https://github.com/occlutrace/OccluView/releases/latest",
            );
        }
        if ui.button("Later").clicked() {
            *next_phase = Some(Phase::Dismissed);
        }
        if ui
            .button("Skip this version")
            .on_hover_text("Do not offer this version again; the next release will be offered")
            .clicked()
        {
            store_skipped_version(&update.version.to_string());
            *next_phase = Some(Phase::Dismissed);
        }
    });
}

fn draw_downloading(
    ui: &mut egui::Ui,
    update: &AvailableUpdate,
    received: u64,
    total: Option<u64>,
) {
    ui.label(egui::RichText::new(format!("Downloading OccluView {}", update.version)).strong());
    ui.add(
        egui::ProgressBar::new(progress_fraction(received, total))
            .desired_width(280.0)
            .show_percentage(),
    );
}

fn draw_ready(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    update: &AvailableUpdate,
    installer: &std::path::Path,
    next_phase: &mut Option<Phase>,
) {
    ui.label(
        egui::RichText::new(format!("OccluView {} is ready to install", update.version)).strong(),
    );
    let handoff_hint = if cfg!(target_os = "windows") {
        "The installer was verified. OccluView will close while Windows applies the update."
    } else {
        "The package was verified. Your system's package installer will open — confirm the update there."
    };
    ui.label(egui::RichText::new(handoff_hint).weak().size(11.0));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.button("Install and close").clicked() {
            match occluview_update::launch_installer(installer) {
                Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Err(error) => *next_phase = Some(Phase::Failed(error.to_string())),
            }
        }
        if ui.button("Later").clicked() {
            *next_phase = Some(Phase::Dismissed);
        }
    });
}

fn draw_failed(ui: &mut egui::Ui, message: &str, next_phase: &mut Option<Phase>) {
    ui.label(egui::RichText::new("Update failed").strong());
    ui.label(egui::RichText::new(message).weak().size(11.0));
    ui.add_space(6.0);
    if ui.button("Dismiss").clicked() {
        *next_phase = Some(Phase::Dismissed);
    }
}

/// Progress in permille keeps the division in integer space (installer sizes
/// are far below u64/1000), so no float-precision lint gymnastics are needed.
fn progress_fraction(received: u64, total: Option<u64>) -> f32 {
    let Some(total) = total.filter(|&total| total > 0) else {
        return 0.0;
    };
    let permille = received.saturating_mul(1000) / total;
    f32::from(u16::try_from(permille.min(1000)).unwrap_or(1000)) / 1000.0
}
