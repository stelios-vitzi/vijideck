mod ports_view;
mod tool;

use crate::{
    ports::{self, Snapshot, SocketKey, StopRequest},
    ui::*,
};
use eframe::egui::{self, RichText, Stroke};
use std::{
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant},
};
use tool::Tool;

enum Command {
    Scan,
    Stop(StopRequest),
}
enum Event {
    Snapshot(Snapshot),
    Stopped(Result<String, String>),
}

pub struct VijiDeck {
    background: crate::background::Background,
    storage: Option<crate::storage::Storage>,
    saved_preferences: crate::storage::Preferences,
    active_tool: Tool,
    logs_only: bool,
    processes: crate::processes::Processes,
    overview: crate::overview::Overview,
    cleanup: Option<crate::cleanup_ui::Cleanup>,
    logs: crate::logs_ui::Logs,
    environment: crate::environment::Environment,
    docker: crate::docker::Docker,
    services: crate::services_ui::Services,
    tx: Sender<Command>,
    rx: Receiver<Event>,
    snapshot: Snapshot,
    selected: Option<SocketKey>,
    query: String,
    protocol: usize,
    listeners_only: bool,
    auto: bool,
    busy: bool,
    last_scan: Instant,
    scanned: bool,
    notice: Option<(String, bool)>,
    confirmation: Option<StopRequest>,
    descending: bool,
    sort: usize,
    inspector_tab: usize,
    host: (String, String),
    #[cfg(feature = "ui-preview")]
    preview_started: Instant,
    #[cfg(feature = "ui-preview")]
    preview_requested: bool,
}

impl VijiDeck {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme(&cc.egui_ctx);
        let (snapshot, selected) = initial_state();
        let scanned = !snapshot.ports.is_empty();
        let (tx, commands) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        let repaint = cc.egui_ctx.clone();
        std::thread::Builder::new()
            .name("port-worker".into())
            .spawn(move || {
                let mut system = sysinfo::System::new();
                while let Ok(command) = commands.recv() {
                    let event = match command {
                        Command::Scan => Event::Snapshot(ports::scan(&mut system)),
                        Command::Stop(request) => Event::Stopped(ports::stop(&request)),
                    };
                    if events.send(event).is_err() {
                        break;
                    }
                    repaint.request_repaint();
                }
            })
            .expect("start port worker");
        let mut storage = if cfg!(feature = "ui-preview")
            && std::env::var_os("VIJIDECK_SCREENSHOT_TO").is_some()
        {
            None
        } else {
            match crate::storage::Storage::open() {
                Ok(store) => Some(store),
                Err(error) => {
                    eprintln!("Could not open preferences: {error}");
                    None
                }
            }
        };
        let preferences = match storage.as_ref().map(|store| store.load_preferences()) {
            Some(Ok(prefs)) => prefs,
            Some(Err(error)) => {
                eprintln!("Could not load preferences: {error}");
                storage = None;
                Default::default()
            }
            None => Default::default(),
        };
        let mut app = Self {
            storage,
            saved_preferences: preferences,
            background: crate::background::Background::new(cc),
            active_tool: Tool::Overview,
            logs_only: false,
            processes: crate::processes::Processes::new(cc.egui_ctx.clone()),
            overview: crate::overview::Overview::new(cc.egui_ctx.clone()),
            cleanup: None,
            logs: crate::logs_ui::Logs::new(),
            environment: crate::environment::Environment::new(),
            docker: crate::docker::Docker::new(),
            services: crate::services_ui::Services::new(),
            tx,
            rx,
            snapshot,
            selected,
            query: String::new(),
            protocol: preferences.protocol,
            listeners_only: preferences.listeners_only,
            auto: preferences.auto_refresh,
            busy: false,
            last_scan: Instant::now(),
            scanned,
            notice: None,
            confirmation: None,
            descending: preferences.descending,
            sort: preferences.sort,
            inspector_tab: 0,
            host: (
                sysinfo::System::host_name().unwrap_or_else(|| "This computer".into()),
                sysinfo::System::long_os_version()
                    .or_else(sysinfo::System::name)
                    .unwrap_or_else(|| std::env::consts::OS.into()),
            ),
            #[cfg(feature = "ui-preview")]
            preview_started: Instant::now(),
            #[cfg(feature = "ui-preview")]
            preview_requested: false,
        };
        app.configure_preview();
        #[cfg(feature = "ui-preview")]
        if std::env::var("VIJIDECK_PREVIEW_VIEW").as_deref() == Ok("widget") {
            app.background
                .apply(crate::background::Action::Widget, &cc.egui_ctx);
        }
        if let Some(request) = crate::logs_access::startup_request() {
            app.active_tool = Tool::Logs;
            app.logs_only = true;
            app.logs.configure_startup(request);
        }
        app
    }

    fn configure_preview(&mut self) {
        #[cfg(feature = "ui-preview")]
        if std::env::var_os("VIJIDECK_SCREENSHOT_TO").is_some() {
            match std::env::var("VIJIDECK_PREVIEW_VIEW").as_deref() {
                Ok("overview" | "widget") => self.active_tool = Tool::Overview,
                Ok("computer") => self.active_tool = Tool::Computer,
                Ok("processes") => self.active_tool = Tool::Processes,
                Ok("environment") => self.active_tool = Tool::Environment,
                Ok("docker") => self.active_tool = Tool::Docker,
                Ok("services") => self.active_tool = Tool::Services,
                Ok(view @ ("services-list" | "services-confirm" | "services-empty")) => {
                    self.active_tool = Tool::Services;
                    self.services
                        .preview(view == "services-confirm", view == "services-empty");
                }
                Ok("logs") => self.active_tool = Tool::Logs,
                Ok("logs-list") => {
                    self.active_tool = Tool::Logs;
                    self.logs.preview();
                }
                Ok("process") => self.inspector_tab = 1,
                Ok("sockets") => self.inspector_tab = 2,
                Ok("empty") => self.selected = None,
                Ok(
                    "cleanup" | "cleanup-confirm" | "cleanup-disks" | "cleanup-scanning"
                    | "cleanup-filters",
                ) => {
                    self.active_tool = Tool::Cleanup;
                    self.cleanup
                        .get_or_insert_with(crate::cleanup_ui::Cleanup::new)
                        .preview();
                }
                Ok("confirm") => {
                    if let Some(key) = &self.selected
                        && let Some(p) = key.pid.and_then(|id| self.snapshot.processes.get(&id))
                    {
                        self.confirmation = Some(StopRequest {
                            socket: key.clone(),
                            start: p.start,
                            name: p.name.clone(),
                            force: true,
                        });
                    }
                }
                _ => {}
            }
            if !matches!(
                std::env::var("VIJIDECK_PREVIEW_VIEW").as_deref(),
                Ok("overview" | "widget")
            ) && self.active_tool == Tool::Overview
            {
                self.active_tool = Tool::Ports;
            }
        }
    }

    fn persist_preferences(&mut self) {
        if self.logs_only {
            return;
        }
        let prefs = crate::storage::Preferences {
            auto_refresh: self.auto,
            listeners_only: self.listeners_only,
            protocol: self.protocol,
            sort: self.sort,
            descending: self.descending,
        };
        if prefs != self.saved_preferences
            && let Some(store) = &mut self.storage
        {
            match store.save_preferences(prefs) {
                Ok(()) => self.saved_preferences = prefs,
                Err(error) => {
                    eprintln!("Could not save preferences: {error}");
                    self.storage = None;
                }
            }
        }
    }

    fn refresh(&mut self) {
        if !self.busy {
            self.busy = self.tx.send(Command::Scan).is_ok();
        }
    }

    #[cfg(feature = "ui-preview")]
    fn capture_preview(&mut self, ctx: &egui::Context) {
        let Some(path) = std::env::var_os("VIJIDECK_SCREENSHOT_TO") else {
            return;
        };
        // Let layout, modal sizing, worker results, and hover state settle.
        let ready = match self.active_tool {
            Tool::Environment => self.environment.preview_ready(),
            Tool::Processes => self.processes.preview_ready(),
            Tool::Overview => self.overview.preview_ready(),
            Tool::Computer => self.overview.computer_preview_ready(),
            Tool::Logs => self.logs.preview_ready(),
            Tool::Services => self.services.preview_ready(),
            _ => true,
        } || self.preview_started.elapsed() > Duration::from_secs(35);
        if !self.preview_requested
            && ready
            && self.preview_started.elapsed() > Duration::from_millis(700)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.preview_requested = true;
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let rgba: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    std::path::Path::new(&path),
                    &rgba,
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ColorType::Rgba8,
                )
                .expect("save UI preview");
                self.background.quitting = true;
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
            }
        }
    }

    fn status_text(&self) -> &str {
        match self.active_tool {
            Tool::Docker => self.docker.status(),
            Tool::Services => self.services.status(),
            Tool::Environment => self.environment.status(),
            Tool::Cleanup => self
                .cleanup
                .as_ref()
                .map_or("Disk cleanup", |cleanup| cleanup.status()),
            Tool::Overview => self.overview.status(),
            Tool::Computer => self.overview.computer_status(),
            Tool::Processes => "Refresh every 2 seconds",
            Tool::Logs => self.logs.status(),
            Tool::Ports if !self.scanned => "Waiting for first scan",
            Tool::Ports if self.auto => "Live · refresh every 2 seconds",
            Tool::Ports => "Auto refresh paused",
        }
    }

    /// Icon navigation rail with host details at its foot.
    fn rail(&mut self, ctx: &egui::Context) {
        let width = if ctx.content_rect().width() < 1100.0 {
            196.0
        } else {
            224.0
        };
        egui::SidePanel::left("tool_rail")
            .exact_width(width)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL.gamma_multiply(0.85))
                    .inner_margin(egui::Margin {
                        left: 18,
                        right: 14,
                        top: 30,
                        bottom: 20,
                    }),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    reactor(ui, 18.0);
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new("VijiDeck")
                            .font(medium(15.0))
                            .extra_letter_spacing(-0.3),
                    );
                });
                ui.add_space(if ui.available_height() < 520.0 {
                    18.0
                } else {
                    36.0
                });
                ui.spacing_mut().item_spacing.y = 2.0;
                if self.logs_only {
                    let _ = nav_item(ui, Tool::Logs.glyph(), "Logs", true);
                } else {
                    for tool in Tool::ALL {
                        if nav_item(ui, tool.glyph(), tool.label(), self.active_tool == tool)
                            .on_hover_text(tool.title())
                            .clicked()
                        {
                            self.active_tool = tool;
                        }
                    }
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        caption(ui, concat!("v", env!("CARGO_PKG_VERSION")));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let widget = ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Widget").size(13.0).color(TEXT_2),
                                    )
                                    .fill(SURFACE)
                                    .stroke(Stroke::NONE)
                                    .corner_radius(255),
                                )
                                .on_hover_text("Keep monitoring in a floating widget");
                            if widget.clicked() {
                                self.background
                                    .apply(crate::background::Action::Widget, ctx);
                            }
                        });
                    });
                    ui.add_space(14.0);
                    #[cfg(windows)]
                    if let Some(error) = &self.background.error {
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            ui.label(RichText::new("Tray unavailable").size(12.0).color(WARNING))
                                .on_hover_text(error);
                        });
                    }
                    let warn =
                        self.active_tool == Tool::Ports && !self.snapshot.warnings.is_empty();
                    ui.horizontal(|ui| {
                        ui.add_space(6.0);
                        dot(ui, if warn { WARNING } else { TEAL });
                        ui.add(
                            egui::Label::new(
                                RichText::new(self.status_text()).size(12.0).color(MUTED),
                            )
                            .truncate(),
                        )
                        .on_hover_text(self.status_text());
                    });
                    let uptime = sysinfo::System::uptime();
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!(
                                    "{} · up {}d {}h",
                                    self.host.1,
                                    uptime / 86_400,
                                    uptime / 3_600 % 24
                                ))
                                .size(12.0)
                                .color(MUTED),
                            )
                            .truncate(),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new(&self.host.0).font(medium(13.0)).color(TEXT_2),
                            )
                            .truncate(),
                        );
                    });
                });
            });
    }
}

impl eframe::App for VijiDeck {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        egui::Color32::TRANSPARENT.to_normalized_gamma_f32()
    }

    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.persist_preferences();
    }

    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.persist_preferences();
        // Screenshot previews must be allowed to exit without becoming a widget.
        #[cfg(feature = "ui-preview")]
        if self.preview_requested && ctx.input(|i| i.viewport().close_requested()) {
            return;
        }
        self.background.poll(ctx);
        self.background.sync_window_shape();
        self.background.sync_window_size(ctx);
        if self.active_tool != Tool::Docker || self.background.compact {
            self.docker.close_logs();
        }
        self.docker.poll();
        self.services.poll();
        if let Some(cleanup) = &mut self.cleanup {
            cleanup.poll();
        }
        self.overview.poll();
        while let Some(result) = crate::worker::receive(&self.rx) {
            let event = match result {
                Ok(event) => event,
                Err(error) => {
                    self.busy = false;
                    self.notice = Some((error, true));
                    break;
                }
            };
            self.busy = false;
            match event {
                Event::Snapshot(snapshot) => {
                    self.snapshot = snapshot;
                    self.scanned = true;
                    self.last_scan = Instant::now();
                }
                Event::Stopped(result) => {
                    self.notice = Some(match result {
                        Ok(text) => (text, false),
                        Err(text) => (text, true),
                    });
                    self.refresh();
                }
            }
        }
        if self.active_tool == Tool::Ports
            && (!self.scanned || (self.auto && self.last_scan.elapsed() >= Duration::from_secs(2)))
            && self.confirmation.is_none()
        {
            self.refresh();
        }
        ctx.request_repaint_after(Duration::from_millis(250));
        if self.background.widget_visible {
            let can_hide = self.background.can_hide();
            let action = self.overview.widget(ctx, can_hide);
            #[cfg(feature = "ui-preview")]
            self.capture_preview(ctx);
            if let Some(action) = action {
                self.background.apply(action, ctx);
            }
        }
        if self.background.compact {
            return;
        }
        if self.active_tool == Tool::Ports
            && self.confirmation.is_none()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::R))
        {
            self.refresh();
        }
        crate::ui::backdrop(ctx);
        self.rail(ctx);
        match self.active_tool {
            Tool::Docker => self.docker.show(ctx),
            Tool::Services => self.services.show(ctx),
            Tool::Environment => self.environment.show(ctx),
            Tool::Processes => self.processes.show(ctx),
            Tool::Logs => self.logs.show(ctx),
            Tool::Computer => self.overview.show_computer(ctx),
            Tool::Overview => {
                if self.overview.show(ctx) {
                    self.active_tool = Tool::Cleanup;
                }
            }
            Tool::Cleanup => {
                let cleanup = self.cleanup.get_or_insert_with(|| {
                    let mut cleanup = crate::cleanup_ui::Cleanup::new();
                    cleanup.restore_and_sync(ctx);
                    cleanup
                });
                cleanup.show(ctx);
            }
            Tool::Ports => self.show_ports(ctx),
        }
        #[cfg(feature = "ui-preview")]
        self.capture_preview(ctx);
    }
}

fn initial_state() -> (Snapshot, Option<SocketKey>) {
    #[cfg(feature = "ui-preview")]
    if std::env::var_os("VIJIDECK_SCREENSHOT_TO").is_some() {
        let snapshot = ports::scan(&mut sysinfo::System::new());
        let selected = snapshot
            .ports
            .iter()
            .find(|port| {
                port.state == "Listen"
                    && port.key.pid.is_some_and(|pid| {
                        pid > 4 && snapshot.processes.get(&pid).is_some_and(|p| p.start > 0)
                    })
            })
            .map(|port| port.key.clone());
        return (snapshot, selected);
    }
    (Snapshot::default(), None)
}
