use crate::{
    ui::{ACCENT, LINE, MUTED, PANEL, WARNING, caption, medium, panel, rule},
    update::{self, Release, Staged},
    worker,
};
use eframe::egui::{self, RichText};
use std::{
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};

const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

enum State {
    Idle,
    Checking(Receiver<Result<Option<Release>, String>>),
    Current,
    Available(Release),
    Downloading(Receiver<Result<Staged, String>>),
    Ready(Staged),
    Restarting(Receiver<Result<(), String>>),
    Error(String),
}

pub(super) struct Updates {
    pub auto: bool,
    pub open: bool,
    state: State,
    next_check: Option<Instant>,
    restart_error: Option<String>,
}

impl Updates {
    pub fn new(auto: bool, restart_error: Option<String>) -> Self {
        Self {
            auto,
            open: restart_error.is_some(),
            state: State::Idle,
            next_check: None,
            restart_error,
        }
    }

    fn check(&mut self) {
        self.next_check = Some(Instant::now() + CHECK_INTERVAL);
        self.state = State::Checking(worker::spawn(update::check));
    }

    pub fn poll(&mut self, allow_auto: bool) -> bool {
        let next = match &self.state {
            State::Checking(job) => worker::poll(job).map(|result| match result {
                Ok(Some(release)) => State::Available(release),
                Ok(None) => State::Current,
                Err(error) => State::Error(format!("Could not check for updates. {error}")),
            }),
            State::Downloading(job) => worker::poll(job).map(|result| match result {
                Ok(staged) => State::Ready(staged),
                Err(error) => State::Error(format!("Could not download the update. {error}")),
            }),
            State::Restarting(job) => match worker::poll(job) {
                Some(Ok(())) => return true,
                Some(Err(error)) => Some(State::Error(error)),
                None => None,
            },
            _ => None,
        };
        if let Some(next) = next {
            self.state = next;
        }
        if allow_auto
            && self.auto
            && !self.busy()
            && !matches!(self.state, State::Ready(_))
            && self.next_check.is_none_or(|time| Instant::now() >= time)
        {
            self.check();
        }
        false
    }

    fn busy(&self) -> bool {
        matches!(
            self.state,
            State::Checking(_) | State::Downloading(_) | State::Restarting(_)
        )
    }

    pub fn has_update(&self) -> bool {
        matches!(self.state, State::Available(_) | State::Ready(_))
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview(&mut self) {
        self.open = true;
        self.state = State::Available(Release::preview());
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let modal_id = egui::Id::new("settings");
        let modal = egui::Modal::new(modal_id)
            .frame(
                panel(PANEL, 20)
                    .stroke(egui::Stroke::new(1.0_f32, LINE))
                    .corner_radius(8),
            )
            .show(ctx, |ui| {
                ui.set_width(400.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Settings").font(medium(22.0)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            self.open = false;
                        }
                    });
                });
                ui.add_space(12.0);
                rule(ui);
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Software updates").strong().size(15.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        caption(ui, &format!("Installed v{}", update::VERSION));
                    });
                });
                ui.add_space(8.0);
                if ui
                    .checkbox(&mut self.auto, "Automatically check for updates")
                    .changed()
                    && self.auto
                {
                    self.next_check = None;
                }
                ui.label(
                    RichText::new("Checks at startup and daily while VijiDeck is open.")
                        .color(MUTED),
                );
                ui.add_space(12.0);
                if let Some(error) = &self.restart_error {
                    ui.label(
                        RichText::new(format!("The update could not be installed. {error}"))
                            .color(WARNING),
                    );
                }
                let (status, color) = match &self.state {
                    State::Idle => (None, MUTED),
                    State::Checking(_) => (Some("Checking for updates…".into()), MUTED),
                    State::Current => (Some("VijiDeck is up to date.".into()), MUTED),
                    State::Available(release) => (
                        Some(format!("VijiDeck {} is available.", release.version)),
                        ACCENT,
                    ),
                    State::Downloading(_) => {
                        (Some("Downloading and verifying the update…".into()), MUTED)
                    }
                    State::Ready(staged) => (
                        Some(format!(
                            "VijiDeck {} is ready to install.",
                            staged.version()
                        )),
                        ACCENT,
                    ),
                    State::Restarting(_) => (Some("Preparing to restart…".into()), MUTED),
                    State::Error(error) => (Some(error.clone()), WARNING),
                };
                if let Some(status) = status {
                    ui.horizontal_wrapped(|ui| {
                        if self.busy() {
                            ui.spinner();
                        }
                        ui.label(RichText::new(status).color(color));
                    });
                    ui.add_space(8.0);
                }
                ui.horizontal(|ui| {
                    let preview = cfg!(feature = "ui-preview")
                        && std::env::var_os("VIJIDECK_SCREENSHOT_TO").is_some();
                    let checking_allowed = !self.busy() && !matches!(self.state, State::Ready(_));
                    if ui
                        .add_enabled(checking_allowed, egui::Button::new("Check for updates"))
                        .clicked()
                        && !preview
                    {
                        self.restart_error = None;
                        self.check();
                    }
                    if let State::Available(release) = &self.state
                        && ui
                            .button(RichText::new("Download update").color(ACCENT))
                            .clicked()
                        && !preview
                    {
                        let release = release.clone();
                        self.state =
                            State::Downloading(worker::spawn(move || update::download(release)));
                    }
                    if matches!(self.state, State::Ready(_))
                        && ui
                            .button(RichText::new("Restart to update").color(ACCENT))
                            .clicked()
                        && let State::Ready(staged) =
                            std::mem::replace(&mut self.state, State::Idle)
                    {
                        self.state = State::Restarting(worker::spawn(move || staged.restart()));
                    }
                });
                ui.add_space(6.0);
                ui.hyperlink_to("Release notes", update::RELEASES);
            });
        if modal.should_close() {
            self.open = false;
        }
    }
}
