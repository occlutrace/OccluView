//! Launch-time update check and the "update available" notice.
//!
//! Policy: OFFER, never install silently. On launch a
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct DownloadDrain {
    finished: bool,
    repaint: bool,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UpdateCheckStatus {
    Idle,
    Disabled,
    Checking,
    Current,
    Available(String),
    Skipped(String),
    Failed(String),
}

type CheckResult = Result<Option<AvailableUpdate>, String>;

/// Launch-time update notice state; owned by the app, drawn every frame.
pub(crate) struct UpdateNotice {
    phase: Phase,
    check_status: UpdateCheckStatus,
    check_rx: Option<mpsc::Receiver<CheckResult>>,
    download_rx: Option<mpsc::Receiver<DownloadEvent>>,
}

/// Where a downloaded installer is kept until the operator installs it.
///
/// Under this account's own state directory, never a shared one. The verified
/// installer waits here for a click, and on a shared workstation -- a clinic
/// reception machine is one -- a world-writable directory means the file that
/// was verified and the file handed to a privileged installer need not be the
/// same file.
fn update_download_dir() -> Option<PathBuf> {
    crate::app_paths::app_state_dir().map(|dir| dir.join("updates"))
}

impl UpdateNotice {
    /// Start the once-per-launch background check. The env var is a hard
    /// override (CI, packaging); the setting only applies when it is unset.
    pub(crate) fn begin_check(setting_enabled: bool) -> Self {
        let mut notice = Self::idle();
        if std::env::var_os("OCCLUVIEW_NO_UPDATE_CHECK").is_some() {
            notice.check_status = UpdateCheckStatus::Disabled;
            return notice;
        }
        if setting_enabled {
            notice.spawn_check();
        }
        notice
    }

    fn idle() -> Self {
        Self {
            phase: Phase::Idle,
            check_status: UpdateCheckStatus::Idle,
            check_rx: None,
            download_rx: None,
        }
    }

    fn spawn_check(&mut self) {
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("occluview-update-check".to_string())
            .spawn(move || {
                let result = occluview_update::check_for_update(env!("CARGO_PKG_VERSION"))
                    .map_err(|error| error.to_string());
                let _ = tx.send(result);
            }) {
            Ok(_) => {
                self.check_status = UpdateCheckStatus::Checking;
                self.check_rx = Some(rx);
            }
            Err(error) => {
                self.check_status = UpdateCheckStatus::Failed(error.to_string());
            }
        }
    }

    pub(crate) fn request_check(&mut self) {
        if std::env::var_os("OCCLUVIEW_NO_UPDATE_CHECK").is_some() {
            self.check_status = UpdateCheckStatus::Disabled;
        } else if self.check_rx.is_none() {
            self.spawn_check();
        }
    }

    pub(crate) fn check_status(&self) -> &UpdateCheckStatus {
        &self.check_status
    }

    fn offer(&mut self, update: AvailableUpdate) {
        if matches!(
            self.phase,
            Phase::Idle | Phase::Dismissed | Phase::Failed(_)
        ) {
            self.phase = Phase::Available(update);
        }
    }

    fn finish_check(&mut self, result: CheckResult) {
        match result {
            Err(error) => self.check_status = UpdateCheckStatus::Failed(error),
            Ok(None) => self.check_status = UpdateCheckStatus::Current,
            Ok(Some(update)) => {
                let version = update.version.to_string();
                if load_skipped_version().as_deref() == Some(version.as_str()) {
                    self.check_status = UpdateCheckStatus::Skipped(version);
                } else {
                    self.check_status = UpdateCheckStatus::Available(version);
                    self.offer(update);
                }
            }
        }
    }

    /// Drain worker events and draw the notice when there is one.
    pub(crate) fn show(&mut self, ctx: &egui::Context) {
        self.drain_events(ctx);
        if self.check_status == UpdateCheckStatus::Checking {
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
        }
        match &self.phase {
            Phase::Idle | Phase::Dismissed => {}
            _ => self.draw_window(ctx),
        }
    }

    fn drain_events(&mut self, ctx: &egui::Context) {
        use std::sync::mpsc::TryRecvError;
        if let Some(rx) = &self.check_rx {
            match rx.try_recv() {
                Ok(result) => {
                    self.check_rx = None;
                    self.finish_check(result);
                    ctx.request_repaint();
                }
                Err(TryRecvError::Disconnected) => {
                    self.check_rx = None;
                    self.check_status = UpdateCheckStatus::Failed(
                        "the update-check worker stopped unexpectedly".to_string(),
                    );
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some(rx) = &self.download_rx {
            let outcome = drain_download_events(&mut self.phase, rx);
            if outcome.repaint {
                ctx.request_repaint();
            }
            if outcome.finished {
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
        let worker_update = update.clone();
        let spawn = std::thread::Builder::new()
            .name("occluview-update-download".to_string())
            .spawn(move || {
                let Some(dest) = update_download_dir() else {
                    let _ = tx.send(DownloadEvent::Failed(
                        "no private directory to download into".to_string(),
                    ));
                    return;
                };
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
            });
        match spawn {
            Ok(_) => {
                self.download_rx = Some(rx);
                Phase::Downloading {
                    update,
                    received: 0,
                    total: None,
                }
            }
            Err(error) => Phase::Failed(format!("could not start update download: {error}")),
        }
    }
}

fn drain_download_events(
    phase: &mut Phase,
    events: &mpsc::Receiver<DownloadEvent>,
) -> DownloadDrain {
    use std::sync::mpsc::TryRecvError;
    let mut outcome = DownloadDrain::default();
    loop {
        match events.try_recv() {
            Ok(DownloadEvent::Progress(received, total)) => {
                if let Phase::Downloading {
                    received: current_received,
                    total: current_total,
                    ..
                } = phase
                {
                    *current_received = received;
                    *current_total = total;
                }
                outcome.repaint = true;
            }
            Ok(DownloadEvent::Done(installer)) => {
                if let Phase::Downloading { update, .. } = phase {
                    *phase = Phase::Ready {
                        update: update.clone(),
                        installer,
                    };
                }
                outcome.finished = true;
                outcome.repaint = true;
                break;
            }
            Ok(DownloadEvent::Failed(message)) => {
                *phase = Phase::Failed(message);
                outcome.finished = true;
                outcome.repaint = true;
                break;
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                *phase = Phase::Failed("the update-download worker stopped unexpectedly".into());
                outcome.finished = true;
                outcome.repaint = true;
                break;
            }
        }
    }
    outcome
}

fn draw_available(
    ui: &mut egui::Ui,
    update: &AvailableUpdate,
    start_download: &mut Option<AvailableUpdate>,
    next_phase: &mut Option<Phase>,
) {
    let (icon_rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
    crate::icons::paint(
        ui.painter(),
        icon_rect,
        crate::icons::AppIcon::InstallUpdate,
        crate::ui_theme::TEXT,
    );
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
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
            // Verified again here, not only at download time: what was
            // checked and what is about to reach a privileged installer are
            // separated by this click.
            match occluview_update::verify_and_launch_installer(update, installer) {
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

#[cfg(test)]
mod settings_status_tests {
    use super::*;

    #[test]
    fn a_failed_manual_check_remains_a_failure() {
        let mut notice = UpdateNotice::idle();
        notice.finish_check(Err("network unavailable".to_string()));

        assert_eq!(
            notice.check_status(),
            &UpdateCheckStatus::Failed("network unavailable".to_string())
        );
    }

    #[test]
    fn a_disconnected_download_worker_becomes_a_failure() {
        let (sender, receiver) = mpsc::channel();
        drop(sender);
        let mut phase = Phase::Idle;

        let outcome = drain_download_events(&mut phase, &receiver);

        assert!(outcome.finished);
        assert!(matches!(phase, Phase::Failed(_)));
    }
}
