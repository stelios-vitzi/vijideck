use crate::ui::*;
use eframe::egui::{self, Color32, RichText, Stroke};
use std::{
    collections::VecDeque,
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};
use sysinfo::{Components, Disks, Networks, System};

#[derive(Default)]
struct Sample {
    cpu: f32,
    gpu: Option<f32>,
    threads: Vec<f32>,
    processor: String,
    cores: Option<usize>,
    frequency: u64,
    memory: u64,
    total_memory: u64,
    swap: u64,
    total_swap: u64,
    disks: Vec<Volume>,
    networks: Vec<(String, String, f64, f64)>,
    temperatures: Vec<(String, f32)>,
    uptime: u64,
}
struct Volume {
    name: String,
    mount: String,
    kind: String,
    filesystem: String,
    total: u64,
    free: u64,
}
struct Hardware {
    sections: Vec<(String, String)>,
    gpu_card: Option<(String, String)>,
    memory_summary: Option<String>,
    note: Option<String>,
}

impl Hardware {
    fn graphics_details(&self) -> Option<&str> {
        self.sections.iter().find_map(|(title, value)| {
            let value = value.trim();
            (title == "Graphics"
                && !value.is_empty()
                && !value.eq_ignore_ascii_case("Unavailable")
                && !value.eq_ignore_ascii_case("Not reported"))
            .then_some(value)
        })
    }
}

fn show_gpu(usage: Option<f32>, hardware: Option<&Hardware>) -> bool {
    // Wait for inventory before deciding there is no GPU information. An idle
    // GPU (0%) or known adapter without counters should still have a GPU gauge.
    hardware.is_none()
        || usage.is_some()
        || hardware.is_some_and(|h| h.gpu_card.is_some() || h.graphics_details().is_some())
}

pub struct Overview {
    rx: Receiver<Sample>,
    hardware_rx: Receiver<Hardware>,
    sample: Option<Sample>,
    hardware: Option<Hardware>,
    history: VecDeque<f32>,
    updated: Instant,
    host: String,
    os: String,
}

impl Overview {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = mpsc::sync_channel(1);
        let (hardware_tx, hardware_rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("hardware-inventory".into())
            .spawn(move || {
                hardware_tx.send(hardware_inventory()).ok();
            })
            .expect("start hardware inventory");
        std::thread::Builder::new()
            .name("system-monitor".into())
            .spawn(move || {
                let mut system = System::new();
                let mut gpu = crate::gpu::Monitor::new();
                let mut disks = Disks::new_with_refreshed_list();
                let mut networks = Networks::new_with_refreshed_list();
                let mut components = Components::new_with_refreshed_list();
                system.refresh_cpu_all();
                let cores = System::physical_core_count();
                let mut last = Instant::now();
                std::thread::sleep(Duration::from_millis(300));
                loop {
                    system.refresh_cpu_all();
                    system.refresh_memory();
                    disks.refresh(true);
                    networks.refresh(true);
                    components.refresh(true);
                    let elapsed = last.elapsed().as_secs_f64().max(0.001);
                    last = Instant::now();
                    let mut network_rows: Vec<_> = networks
                        .iter()
                        .map(|(name, data)| {
                            (
                                name.clone(),
                                format!(
                                    "{}  {}",
                                    data.mac_address(),
                                    data.ip_networks()
                                        .iter()
                                        .map(|ip| ip.to_string())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ),
                                data.received() as f64 / elapsed,
                                data.transmitted() as f64 / elapsed,
                            )
                        })
                        .collect();
                    network_rows.sort_by(|a, b| a.0.cmp(&b.0));
                    let sample = Sample {
                        cpu: system.global_cpu_usage(),
                        gpu: gpu.sample(),
                        threads: system.cpus().iter().map(|cpu| cpu.cpu_usage()).collect(),
                        processor: system
                            .cpus()
                            .first()
                            .map(|cpu| cpu.brand().trim().to_owned())
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| "Processor name unavailable".into()),
                        cores,
                        frequency: system.cpus().first().map_or(0, |cpu| cpu.frequency()),
                        memory: system.used_memory(),
                        total_memory: system.total_memory(),
                        swap: system.used_swap(),
                        total_swap: system.total_swap(),
                        disks: crate::volumes::visible(&disks)
                            .into_iter()
                            .map(|disk| Volume {
                                name: disk.name().to_string_lossy().into_owned(),
                                mount: disk.mount_point().display().to_string(),
                                kind: format!("{:?}", disk.kind()),
                                filesystem: disk.file_system().to_string_lossy().into_owned(),
                                total: disk.total_space(),
                                free: disk.available_space(),
                            })
                            .collect(),
                        networks: network_rows,
                        temperatures: components
                            .iter()
                            .filter_map(|c| c.temperature().map(|t| (c.label().to_owned(), t)))
                            .collect(),
                        uptime: System::uptime(),
                    };
                    match tx.try_send(sample) {
                        Err(mpsc::TrySendError::Disconnected(_)) => break,
                        Err(mpsc::TrySendError::Full(_)) | Ok(()) => {}
                    }
                    ctx.request_repaint();
                    std::thread::sleep(Duration::from_secs(1));
                }
            })
            .expect("start system monitor");
        Self {
            rx,
            hardware_rx,
            sample: None,
            hardware: None,
            history: VecDeque::new(),
            updated: Instant::now(),
            host: System::host_name().unwrap_or_else(|| "This computer".into()),
            os: System::long_os_version()
                .or_else(System::name)
                .unwrap_or_else(|| std::env::consts::OS.into()),
        }
    }

    pub fn poll(&mut self) {
        while let Ok(sample) = self.rx.try_recv() {
            self.history.push_back(sample.cpu);
            if self.history.len() > 60 {
                self.history.pop_front();
            }
            self.sample = Some(sample);
            self.updated = Instant::now();
        }
        if let Ok(hardware) = self.hardware_rx.try_recv() {
            self.hardware = Some(hardware);
        }
    }

    pub fn status(&self) -> &str {
        if self.sample.is_none() {
            "Reading system hardware"
        } else if self.updated.elapsed() > Duration::from_secs(5) {
            "Telemetry delayed"
        } else {
            "Live · 1 second samples"
        }
    }

    pub fn widget(&self, ctx: &egui::Context, can_hide: bool) -> Option<crate::background::Action> {
        use crate::background::Action;
        #[cfg(not(windows))]
        let _ = can_hide;
        let mut action = None;
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(6, 11, 20, 235))
                    .stroke(Stroke::new(1.0_f32, LINE))
                    .corner_radius(16)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ctx, |ui| {
                // The global 8px item spacing makes the content taller than the
                // fixed 160px window, which clips the frame's bottom outline.
                ui.spacing_mut().item_spacing.y = 4.0;
                let drag_area = ui.interact(
                    ui.max_rect(),
                    ui.id().with("widget_drag_area"),
                    egui::Sense::click_and_drag(),
                );
                if drag_area.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new("VIJIDECK").size(12.0).strong().color(TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        if Self::widget_button(ui, "Close app", 2).clicked() {
                            action = Some(Action::Quit);
                        }
                        if Self::widget_button(ui, "Go to app", 1).clicked() {
                            action = Some(Action::Open);
                        }
                        #[cfg(windows)]
                        if can_hide && Self::minimize_button(ui).clicked() {
                            action = Some(Action::Hide);
                        }
                    });
                });
                ui.add_space(2.0);
                let values = self
                    .sample
                    .as_ref()
                    .map(|sample| {
                        [
                            Some(sample.cpu),
                            (sample.total_memory > 0)
                                .then(|| sample.memory as f32 / sample.total_memory as f32 * 100.0),
                            sample.gpu,
                        ]
                    })
                    .unwrap_or([None; 3]);
                ui.columns(3, |columns| {
                    for ((ui, (label, color)), value) in columns
                        .iter_mut()
                        .zip([("CPU", G_CPU), ("RAM", G_MEM), ("GPU", G_GPU)])
                        .zip(values)
                    {
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new(label).size(10.0).color(MUTED));
                            gauge_face(ui, 64.0, value, color);
                        });
                    }
                });
                ui.label(
                    RichText::new(if self.sample.is_none() {
                        "Waiting for telemetry"
                    } else if self.updated.elapsed() > Duration::from_secs(5) {
                        "Telemetry delayed"
                    } else {
                        "Live · 1s"
                    })
                    .size(9.0)
                    .color(MUTED),
                );
            });
        action
    }

    #[cfg(windows)]
    fn minimize_button(ui: &mut egui::Ui) -> egui::Response {
        Self::widget_button(ui, "Minimize to tray", 0)
    }

    fn widget_button(ui: &mut egui::Ui, tooltip: &str, icon: u8) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(23.0, 21.0), egui::Sense::click());
        let visuals = ui.style().interact(&response);
        ui.painter().rect(
            rect,
            5,
            visuals.bg_fill,
            Stroke::new(
                1.0_f32,
                if response.hovered() {
                    ACCENT.gamma_multiply(0.65)
                } else {
                    LINE
                },
            ),
            egui::StrokeKind::Inside,
        );
        let center = rect.center();
        let stroke = Stroke::new(1.6_f32, if response.hovered() { ACCENT } else { TEXT });
        match icon {
            0 => {
                ui.painter().line_segment(
                    [
                        center + egui::vec2(-4.0, 2.0),
                        center + egui::vec2(4.0, 2.0),
                    ],
                    stroke,
                );
            }
            1 => {
                ui.painter().rect_stroke(
                    egui::Rect::from_center_size(center, egui::vec2(11.0, 9.0)),
                    1,
                    stroke,
                    egui::StrokeKind::Inside,
                );
                ui.painter().line_segment(
                    [
                        center + egui::vec2(-5.0, -1.0),
                        center + egui::vec2(5.0, -1.0),
                    ],
                    stroke,
                );
            }
            _ => {
                ui.painter().line_segment(
                    [
                        center + egui::vec2(-4.0, -4.0),
                        center + egui::vec2(4.0, 4.0),
                    ],
                    stroke,
                );
                ui.painter().line_segment(
                    [
                        center + egui::vec2(4.0, -4.0),
                        center + egui::vec2(-4.0, 4.0),
                    ],
                    stroke,
                );
            }
        }
        response.on_hover_text(tooltip)
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_ready(&self) -> bool {
        self.sample.is_some() && self.hardware.is_some() && self.history.len() >= 2
    }

    /// Draws the overview. Returns `true` when the user asks to free up space.
    pub fn show(&self, ctx: &egui::Context) -> bool {
        let mut open_cleanup = false;
        let width = ctx.content_rect().width();
        let side = (width * 0.04).clamp(16.0, 64.0) as i8;
        let frame = egui::Frame::new().fill(BG).inner_margin(egui::Margin {
            left: side,
            right: side,
            top: 8,
            bottom: 0,
        });
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            let scroll = egui::ScrollArea::vertical()
                .id_salt("overview_scroll")
                .auto_shrink([false, false]);
            #[cfg(feature = "ui-preview")]
            let scroll = if std::env::var("VIJIDECK_PREVIEW_VIEW").as_deref()
                == Ok("overview-hardware")
            {
                scroll.vertical_scroll_offset(1040.0)
            } else {
                scroll
            };
            scroll.show(ui, |ui| {
                ui.add_space(40.0);
                let Some(s) = &self.sample else {
                    ui.vertical_centered(|ui| {
                        ui.add_space(120.0);
                        ui.spinner();
                        caption(
                            ui,
                            "Reading processor, memory, storage, and network activity…",
                        );
                    });
                    return;
                };
                self.gauges(ui, s);
                ui.add_space(40.0);
                insight(ui, s);
                ui.add_space(72.0);
                let wide = ui.available_width() >= 900.0;
                if wide {
                    let gap = (ui.available_width() * 0.06).clamp(32.0, 96.0);
                    let left = (ui.available_width() - gap) * 1.45 / 2.45;
                    let right = ui.available_width() - gap - left;
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.allocate_ui_with_layout(
                            egui::vec2(left, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(left);
                                self.machine(ui, s);
                            },
                        );
                        ui.add_space(gap);
                        ui.allocate_ui_with_layout(
                            egui::vec2(right, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(right);
                                open_cleanup |= drives(ui, s);
                            },
                        );
                    });
                } else {
                    self.machine(ui, s);
                    ui.add_space(56.0);
                    open_cleanup |= drives(ui, s);
                }
                ui.add_space(72.0);
                self.processor(ui, s);
                ui.add_space(64.0);
                self.inventory(ui);
                ui.add_space(64.0);
                networks(ui, s);
                ui.add_space(24.0);
                caption(
                    ui,
                    "Unavailable sensors and device details depend on operating system and driver support.",
                );
                ui.add_space(40.0);
            });
        });
        open_cleanup
    }

    fn gauges(&self, ui: &mut egui::Ui, s: &Sample) {
        let card = self.hardware.as_ref().and_then(|h| h.gpu_card.as_ref());
        let mut items = vec![
            (
                "CPU",
                Some(s.cpu),
                G_CPU,
                format!(
                    "{} cores · {}",
                    s.cores.unwrap_or(s.threads.len()),
                    if s.frequency == 0 {
                        "frequency n/a".to_owned()
                    } else {
                        format!("{:.1} GHz", s.frequency as f64 / 1000.0)
                    }
                ),
                format!("{} · {} logical processors", s.processor, s.threads.len()),
            ),
            (
                "Memory",
                percent(s.memory, s.total_memory),
                G_MEM,
                format!("{} / {}", gib(s.memory), bytes(s.total_memory)),
                format!("{} in use of {}", bytes(s.memory), bytes(s.total_memory)),
            ),
        ];
        if show_gpu(s.gpu, self.hardware.as_ref()) {
            let model = card.map_or_else(
                || {
                    if self.hardware.is_none() {
                        "Reading GPU model…".to_owned()
                    } else {
                        "GPU model unavailable".to_owned()
                    }
                },
                |(model, _)| model.clone(),
            );
            let detail = card
                .map(|(_, capacity)| capacity.as_str())
                .filter(|capacity| !capacity.contains("unavailable"))
                .map_or_else(|| "Utilization".to_owned(), |c| format!("{c} VRAM"));
            items.push(("GPU", s.gpu, G_GPU, detail, model));
        }
        let available = ui.available_width();
        let count = items.len() as f32;
        let gap = (available * 0.04).clamp(12.0, 64.0);
        let column = ((available - gap * (count - 1.0)) / count).max(80.0);
        let size = column.min(280.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (title, value, color, detail, hover) in items {
                ui.allocate_ui_with_layout(
                    egui::vec2(column, size + 60.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        ui.set_width(column);
                        ui.spacing_mut().item_spacing.y = 4.0;
                        gauge_face(ui, size, value, color);
                        gauge_title(ui, title, gauge_color(value, color));
                        ui.add(
                            egui::Label::new(
                                RichText::new(detail).monospace().size(13.0).color(MUTED),
                            )
                            .truncate(),
                        )
                        .on_hover_text(hover);
                    },
                );
            }
        });
    }

    fn machine(&self, ui: &mut egui::Ui, s: &Sample) {
        eyebrow(ui, "This machine");
        ui.add_space(6.0);
        let hardware = self.hardware.as_ref();
        let section = |name: &str| {
            hardware.and_then(|h| {
                h.sections
                    .iter()
                    .find(|(title, _)| title == name)
                    .map(|(_, value)| value.as_str())
                    .filter(|value| {
                        !value.is_empty() && *value != "Unavailable" && *value != "Not reported"
                    })
            })
        };
        let first = |value: &str| {
            value
                .split(" / ")
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned()
        };
        let graphics = hardware
            .and_then(|h| h.gpu_card.as_ref())
            .map(|(model, capacity)| (model.clone(), format!(" · {capacity}")))
            .or_else(|| section("Graphics").map(|g| (first(g), String::new())));
        let mut rows: Vec<(&str, String, String)> = vec![
            ("Name", self.host.clone(), String::new()),
            (
                "System",
                self.os.clone(),
                format!(" · {}", System::cpu_arch()),
            ),
            (
                "Processor",
                s.processor.clone(),
                format!(
                    " · {} cores · {} threads",
                    s.cores.map_or_else(|| "unknown".into(), |n| n.to_string()),
                    s.threads.len()
                ),
            ),
        ];
        if let Some((model, tail)) = graphics {
            rows.push(("Graphics", model, tail));
        }
        rows.push((
            "Memory",
            bytes(s.total_memory),
            hardware
                .and_then(|h| h.memory_summary.as_deref())
                .map(|summary| {
                    summary
                        .split(" / ")
                        .filter(|part| !part.contains("installed") && !part.contains("unavailable"))
                        .fold(String::new(), |acc, part| acc + " · " + part)
                })
                .unwrap_or_default(),
        ));
        if let Some(board) = section("Motherboard") {
            rows.push(("Board", first(board), String::new()));
        }
        if let Some((name, ip)) = primary_network(s) {
            rows.push(("Network", name, format!(" · {ip}")));
        }
        rows.push(("Uptime", uptime(s.uptime), String::new()));
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, (label, value, tail)) in rows.into_iter().enumerate() {
                spec_row(ui, index == 0, label, |ui| {
                    let mut job = egui::text::LayoutJob::default();
                    job.append(
                        &value,
                        0.0,
                        egui::TextFormat::simple(egui::FontId::proportional(14.0), TEXT),
                    );
                    job.append(
                        &tail,
                        0.0,
                        egui::TextFormat::simple(egui::FontId::proportional(14.0), MUTED),
                    );
                    ui.add(egui::Label::new(job).wrap().selectable(true));
                });
            }
        });
    }

    fn processor(&self, ui: &mut egui::Ui, s: &Sample) {
        eyebrow(ui, "Processor activity");
        ui.add_space(14.0);
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 72.0), egui::Sense::hover());
        let p = ui.painter();
        for fraction in [0.0, 0.5, 1.0] {
            let y = rect.bottom() - rect.height() * fraction;
            p.hline(rect.x_range(), y, Stroke::new(1.0_f32, LINE));
        }
        let points: Vec<_> = self
            .history
            .iter()
            .enumerate()
            .map(|(i, cpu)| {
                egui::pos2(
                    rect.right() - (self.history.len() - 1 - i) as f32 / 59.0 * rect.width(),
                    rect.bottom() - cpu.clamp(0.0, 100.0) / 100.0 * rect.height(),
                )
            })
            .collect();
        if points.len() > 1 {
            p.add(egui::Shape::line(
                points.clone(),
                Stroke::new(5.0_f32, G_CPU.gamma_multiply(0.1)),
            ));
            p.add(egui::Shape::line(points, Stroke::new(1.5_f32, G_CPU)));
        }
        ui.add_space(6.0);
        caption(ui, "CPU history · last 60 seconds · 0–100%");
        ui.add_space(14.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            for (i, usage) in s.threads.iter().enumerate() {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(10.0, 28.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 2.0, TRACK);
                let fill = egui::Rect::from_min_max(
                    egui::pos2(
                        rect.left(),
                        rect.bottom() - rect.height() * usage.clamp(0.0, 100.0) / 100.0,
                    ),
                    rect.right_bottom(),
                );
                ui.painter()
                    .rect_filled(fill, 2.0, gauge_color(Some(*usage), G_CPU));
                response.on_hover_text(format!("Logical processor {}: {:.1}%", i + 1, usage));
            }
        });
        if !s.temperatures.is_empty() || s.total_swap > 0 {
            ui.add_space(14.0);
            ui.horizontal_wrapped(|ui| {
                badge(
                    ui,
                    &format!("Swap {} / {}", bytes(s.swap), bytes(s.total_swap)),
                    MUTED,
                );
                for (label, temperature) in &s.temperatures {
                    badge(ui, &format!("{label}  {temperature:.0} °C"), MUTED);
                }
            });
        }
    }

    fn inventory(&self, ui: &mut egui::Ui) {
        eyebrow(ui, "Hardware inventory");
        ui.add_space(6.0);
        let Some(hardware) = &self.hardware else {
            ui.horizontal(|ui| {
                ui.spinner();
                caption(
                    ui,
                    "Reading graphics, motherboard, memory modules, and connected devices…",
                );
            });
            return;
        };
        if hardware.sections.is_empty() {
            caption(ui, "Additional device details unavailable.");
        }
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, (title, value)) in hardware.sections.iter().enumerate() {
                spec_row(ui, index == 0, title, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    if title == "Memory modules"
                        && let Some(summary) = &hardware.memory_summary
                    {
                        ui.label(RichText::new(summary.replace(" / ", " · ")).color(TEXT_2));
                        ui.add_space(4.0);
                    }
                    for item in value.split("; ").filter(|item| !item.trim().is_empty()) {
                        let mut parts = item
                            .split(" / ")
                            .map(str::trim)
                            .filter(|part| !part.is_empty());
                        let Some(primary) = parts.next() else {
                            continue;
                        };
                        let mut job = egui::text::LayoutJob::default();
                        job.append(
                            primary,
                            0.0,
                            egui::TextFormat::simple(egui::FontId::proportional(14.0), TEXT),
                        );
                        for detail in parts {
                            job.append(
                                &format!(" · {detail}"),
                                0.0,
                                egui::TextFormat::simple(egui::FontId::proportional(14.0), MUTED),
                            );
                        }
                        ui.add(egui::Label::new(job).wrap().selectable(true));
                    }
                });
            }
        });
        if let Some(note) = &hardware.note {
            ui.add_space(12.0);
            caption(ui, note);
        }
    }
}

/// Gauge label with a small coloured dot, centred in its column.
fn gauge_title(ui: &mut egui::Ui, title: &str, color: Color32) {
    let galley = ui
        .painter()
        .layout_no_wrap(title.to_owned(), medium(15.0), TEXT);
    let size = galley.size() + egui::vec2(13.0, 0.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().circle_filled(rect.left_center(), 2.5, color);
    ui.painter()
        .galley(rect.left_top() + egui::vec2(13.0, 0.0), galley, TEXT);
}

fn spec_row(ui: &mut egui::Ui, first: bool, label: &str, contents: impl FnOnce(&mut egui::Ui)) {
    if !first {
        rule(ui);
    }
    ui.add_space(11.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(112.0, 19.0), egui::Sense::hover());
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.0),
            MUTED,
        );
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            contents(ui);
        });
    });
    ui.add_space(11.0);
}

/// The status sentence under the gauges. Disk pressure lives here rather than
/// in a gauge of its own.
fn insight(ui: &mut egui::Ui, s: &Sample) {
    let disks: Vec<_> = s
        .disks
        .iter()
        .filter_map(|d| {
            percent(d.total.saturating_sub(d.free), d.total).map(|p| (drive_label(&d.mount), p))
        })
        .collect();
    let segments = insight_segments(Some(s.cpu), percent(s.memory, s.total_memory), &disks);
    let mut job = egui::text::LayoutJob::default();
    for (text, emphasis) in &segments {
        job.append(
            text,
            0.0,
            egui::TextFormat::simple(light(23.0), if *emphasis { WARNING } else { TEXT }),
        );
    }
    job.halign = egui::Align::Center;
    job.wrap.max_width = ui.available_width().min(680.0);
    ui.vertical_centered(|ui| {
        ui.add(egui::Label::new(job).wrap());
        let free: u64 = s.disks.iter().map(|d| d.free).sum();
        let subline = if s.disks.is_empty() {
            "No mounted volumes reported.".to_owned()
        } else {
            format!(
                "{} free across {} {}.",
                bytes(free),
                s.disks.len(),
                if s.disks.len() == 1 {
                    "drive"
                } else {
                    "drives"
                }
            )
        };
        ui.add_space(2.0);
        ui.label(RichText::new(subline).size(14.0).color(MUTED));
    });
}

/// Builds the insight sentence as `(text, emphasised)` segments.
fn insight_segments(
    cpu: Option<f32>,
    memory: Option<f32>,
    disks: &[(String, f32)],
) -> Vec<(String, bool)> {
    const HIGH: f32 = 80.0;
    let mut hot: Vec<(String, f32)> = [("CPU", cpu), ("Memory", memory)]
        .into_iter()
        .filter_map(|(name, value)| value.filter(|v| *v >= HIGH).map(|v| (name.to_owned(), v)))
        .collect();
    let compute_steady = hot.is_empty();
    if let Some((name, value)) = disks
        .iter()
        .filter(|(_, value)| *value >= HIGH)
        .max_by(|a, b| a.1.total_cmp(&b.1))
    {
        hot.push((format!("Disk {name}"), *value));
    }
    let mut segments = Vec::new();
    if hot.is_empty() {
        segments.push(("Everything is steady.".to_owned(), false));
        return segments;
    }
    if compute_steady {
        segments.push(("Everything is steady. ".to_owned(), false));
    }
    let count = hot.len();
    for (index, (name, value)) in hot.into_iter().enumerate() {
        segments.push((format!("{name} is "), false));
        segments.push(("running high".to_owned(), true));
        let gap = if index + 1 < count { " " } else { "" };
        segments.push((format!(" at {value:.0}%.{gap}"), false));
    }
    segments
}

fn drive_label(mount: &str) -> String {
    let trimmed = mount.trim_end_matches(['\\', '/']);
    if trimmed.is_empty() {
        mount.to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Drive list. Returns `true` when "Free up space" is clicked.
fn drives(ui: &mut egui::Ui, s: &Sample) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        eyebrow(ui, "Drives");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let link = ui.add(
                egui::Label::new(
                    RichText::new("Free up space →")
                        .font(medium(13.0))
                        .color(ACCENT),
                )
                .sense(egui::Sense::click()),
            );
            if link.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            clicked = link.on_hover_text("Open disk cleanup").clicked();
        });
    });
    ui.add_space(12.0);
    if s.disks.is_empty() {
        caption(ui, "No mounted volumes reported.");
    }
    for (index, disk) in s.disks.iter().enumerate() {
        if index > 0 {
            ui.add_space(24.0);
        }
        let used = disk.total.saturating_sub(disk.free);
        let fraction = percent(used, disk.total).unwrap_or(0.0);
        let color = gauge_color(Some(fraction), ACCENT);
        ui.horizontal(|ui| {
            let mut job = egui::text::LayoutJob::default();
            job.append(
                &drive_label(&disk.mount),
                0.0,
                egui::TextFormat::simple(egui::FontId::monospace(14.0), TEXT),
            );
            if !disk.name.is_empty() {
                job.append(
                    &disk.name,
                    8.0,
                    egui::TextFormat::simple(egui::FontId::proportional(14.0), TEXT_2),
                );
            }
            ui.add(egui::Label::new(job).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{} / {}", gib(used), bytes(disk.total)))
                        .monospace()
                        .size(13.0)
                        .color(TEXT_2),
                );
            });
        });
        ui.add_space(2.0);
        thin_bar(ui, fraction / 100.0, color);
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(format!("{} · {}", disk_kind(&disk.kind), disk.filesystem))
                        .size(13.0)
                        .color(MUTED),
                )
                .truncate(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{} free", bytes(disk.free)))
                        .monospace()
                        .size(13.0)
                        .color(MUTED),
                );
            });
        });
    }
    clicked
}

fn disk_kind(kind: &str) -> &str {
    match kind {
        "SSD" => "SSD",
        "HDD" => "HDD",
        _ => "Drive",
    }
}

fn networks(ui: &mut egui::Ui, s: &Sample) {
    eyebrow(ui, "Network interfaces");
    ui.add_space(2.0);
    caption(
        ui,
        "Receive / send rates include virtual and loopback interfaces; they are not internet speed tests.",
    );
    ui.add_space(6.0);
    if s.networks.is_empty() {
        caption(ui, "No network interfaces reported.");
    }
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (index, (name, address, received, sent)) in s.networks.iter().enumerate() {
            if index > 0 {
                rule(ui);
            }
            ui.add_space(10.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width((ui.available_width() - 220.0).max(160.0));
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add(egui::Label::new(RichText::new(name).size(14.0)).truncate());
                    ui.add(
                        egui::Label::new(
                            RichText::new(address).monospace().size(12.0).color(MUTED),
                        )
                        .wrap()
                        .selectable(true),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    ui.label(
                        RichText::new(format!("↑ {}/s", bytes(*sent as u64)))
                            .monospace()
                            .size(13.0)
                            .color(TEXT_2),
                    );
                    ui.label(
                        RichText::new(format!("↓ {}/s", bytes(*received as u64)))
                            .monospace()
                            .size(13.0)
                            .color(TEXT_2),
                    );
                });
            });
            ui.add_space(10.0);
        }
    });
}

/// The busiest interface that has a routable IPv4 address.
fn primary_network(s: &Sample) -> Option<(String, String)> {
    s.networks
        .iter()
        .filter_map(|(name, address, received, sent)| {
            let ip = address.split([' ', ',']).find(|part| {
                part.contains('.') && !part.starts_with("127.") && !part.starts_with("169.254.")
            })?;
            let ip = ip.split('/').next().unwrap_or(ip).to_owned();
            Some((name.clone(), ip, received + sent))
        })
        .max_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(name, ip, _)| (name, ip))
}

fn uptime(seconds: u64) -> String {
    format!(
        "{}d {}h {}m",
        seconds / 86_400,
        seconds / 3_600 % 24,
        seconds / 60 % 60
    )
}

fn percent(used: u64, total: u64) -> Option<f32> {
    (total > 0).then(|| (used as f64 / total as f64 * 100.0).clamp(0.0, 100.0) as f32)
}
/// Plain GiB number without unit, for "used / total" pairs.
fn gib(value: u64) -> String {
    format!("{:.1}", value as f64 / (1u64 << 30) as f64)
}
fn bytes(value: u64) -> String {
    if value >= 1 << 40 {
        format!("{:.1} TiB", value as f64 / (1u64 << 40) as f64)
    } else if value >= 1 << 30 {
        format!("{:.1} GiB", value as f64 / (1u64 << 30) as f64)
    } else if value >= 1 << 20 {
        format!("{:.1} MiB", value as f64 / (1u64 << 20) as f64)
    } else if value >= 1 << 10 {
        format!("{:.1} KiB", value as f64 / 1024.0)
    } else {
        format!("{value} B")
    }
}

#[cfg(windows)]
fn hardware_inventory() -> Hardware {
    use std::process::Command;
    // A fixed, read-only CIM query. No user content is interpolated into shell code.
    let script = r#"
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$ErrorActionPreference = 'Stop'
function Report($title, $class, $format) {
    try {
        $rows = @(Get-CimInstance -ClassName $class | ForEach-Object $format | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
        $value = if ($rows.Count) { $rows -join '; ' } else { 'Not reported' }
        $value = $value -replace '[\r\n\t]', ' '
        Write-Output ($title + "`t" + $value)
    } catch { Write-Output ($title + "`tUnavailable") }
}
Report 'Computer' 'Win32_ComputerSystem' { "$($_.Manufacturer) $($_.Model)" }
Report 'Motherboard' 'Win32_BaseBoard' { "$($_.Manufacturer) $($_.Product) / $($_.Version)" }
Report 'Graphics' 'Win32_VideoController' { "$($_.Name) / driver $($_.DriverVersion)" }
try {
    $drivers = @(Get-ChildItem -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}' -ErrorAction SilentlyContinue | ForEach-Object { Get-ItemProperty -LiteralPath $_.PSPath -ErrorAction SilentlyContinue })
    $cards = @(Get-CimInstance -ClassName Win32_VideoController | Where-Object { $_.ConfigManagerErrorCode -eq 0 } | ForEach-Object {
        $name = $_.Name
        # AdapterRAM is only 32 bits; use the driver's 64-bit capacity instead.
        $memory = ($drivers | Where-Object { $_.DriverDesc -eq $name } | ForEach-Object { $_.'HardwareInformation.qwMemorySize' } | Measure-Object -Maximum).Maximum
        [PSCustomObject]@{ Name = $name; Memory = $memory }
    })
    $card = $cards | Sort-Object Memory -Descending | Select-Object -First 1
    if ($card) {
        $model = $card.Name -replace '[\r\n\t]', ' '
        $capacity = if ($card.Memory -gt 0) { "{0:0} GiB" -f ($card.Memory / 1GB) } else { 'Capacity unavailable' }
        Write-Output ("GPU card`t" + $model + "`t" + $capacity)
    }
} catch {}
Report 'Memory modules' 'Win32_PhysicalMemory' { "$($_.DeviceLocator): $([math]::Round($_.Capacity / 1GB, 1)) GiB / $($_.ConfiguredClockSpeed) MT/s / $($_.Manufacturer) $($_.PartNumber.Trim())" }
try {
    $modules = @(Get-CimInstance Win32_PhysicalMemory)
    if ($modules.Count) {
        $capacity = ($modules | Measure-Object Capacity -Sum).Sum
        $speeds = @($modules | ForEach-Object { $_.ConfiguredClockSpeed } | Where-Object { $_ -gt 0 } | Sort-Object -Unique)
        $types = @($modules | ForEach-Object { switch ([int]$_.SMBIOSMemoryType) { 24 { 'DDR3' } 26 { 'DDR4' } 34 { 'DDR5' } } } | Sort-Object -Unique)
        $details = @()
        if ($capacity -gt 0) { $details += ("{0:0.#} GiB installed" -f ($capacity / 1GB)) }
        if ($types.Count) { $details += ($types -join ', ') }
        if ($speeds.Count) { $details += (($speeds -join ', ') + ' MT/s configured') } else { $details += 'Speed unavailable' }
        $details += 'CL unavailable'
        Write-Output ("Memory summary`t" + ($details -join ' / '))
    }
} catch {}
Report 'Physical drives' 'Win32_DiskDrive' {
    $disk = $_
    if (-not $disk.Size -or $disk.Size -le 0 -or [string]::IsNullOrWhiteSpace($disk.Model)) { return }
    $letters = @($disk | Get-CimAssociatedInstance -Association Win32_DiskDriveToDiskPartition -ErrorAction SilentlyContinue | ForEach-Object {
        $_ | Get-CimAssociatedInstance -Association Win32_LogicalDiskToPartition -ErrorAction SilentlyContinue
    } | ForEach-Object {
        if ($_.DeviceID -ieq $env:SystemDrive) { "$($_.DeviceID) (System)" } else { $_.DeviceID }
    } | Sort-Object -Unique)
    $volumes = if ($letters.Count) { 'Volumes: ' + ($letters -join ', ') } else { 'No drive letters reported' }
    "$($disk.Model) / $volumes / $([math]::Round($disk.Size / 1GB, 1)) GiB / $($disk.InterfaceType)"
}
Report 'Firmware' 'Win32_BIOS' { "$($_.Manufacturer) / $($_.SMBIOSBIOSVersion)" }
Report 'Displays' 'Win32_DesktopMonitor' { "$($_.Name) / $($_.MonitorManufacturer)" }
Report 'Audio' 'Win32_SoundDevice' { $_.Name }
Report 'Network adapters' 'Win32_NetworkAdapter' { if ($_.PhysicalAdapter) { $_.Name } }
Report 'Battery' 'Win32_Battery' { "$($_.Name) / $($_.EstimatedChargeRemaining)% remaining" }
Report 'Keyboards' 'Win32_Keyboard' { $_.Name }
Report 'Pointing devices' 'Win32_PointingDevice' { $_.Name }
"#;
    let result = (|| -> Result<String, String> {
        let mut command = Command::new("powershell.exe");
        command.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ]);
        let output = crate::command::capture(
            command,
            crate::command::Limits {
                timeout: Duration::from_secs(25),
                output_bytes: 4 * 1024 * 1024,
            },
        )
        .map_err(|error| format!("Hardware inventory could not be read: {error}"))?;
        let status = output.status;
        if !status.success() {
            return Err("Windows hardware inventory could not be read.".into());
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    })();
    match result {
        Ok(output) => Hardware {
            memory_summary: output.lines().find_map(|line| line.strip_prefix("Memory summary\t").map(str::to_owned)),
            gpu_card: output.lines().find_map(|line| {
                let value = line.strip_prefix("GPU card\t")?;
                let (model, capacity) = value.split_once('\t')?;
                Some((model.to_owned(), capacity.to_owned()))
            }),
            sections: output
                .lines()
                .filter(|line| !line.starts_with("GPU card\t") && !line.starts_with("Memory summary\t"))
                .filter_map(|line| {
                    line.split_once('\t')
                        .map(|(k, v)| (k.to_owned(), v.trim().to_owned()))
                })
                .collect(),
            note: Some(
                "GPU utilization uses supported Windows performance counters. VRAM and fan speeds are not collected."
                    .into(),
            ),
        },
        Err(note) => Hardware {
            memory_summary: None,
            gpu_card: None,
            sections: Vec::new(),
            note: Some(note),
        },
    }
}

#[cfg(not(windows))]
fn hardware_inventory() -> Hardware {
    Hardware { memory_summary: None, gpu_card: crate::gpu::card(), sections: vec![("Architecture".into(), System::cpu_arch()), ("Kernel".into(), System::kernel_version().unwrap_or_else(|| "Unavailable".into()))], note: Some("Extended graphics, motherboard, firmware, and memory module inventory is currently available on Windows. Processor, memory, mounted storage, network, and supported temperature sensors are shown above.".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capacity_percent_handles_missing_and_inconsistent_totals() {
        assert_eq!(percent(10, 0), None);
        assert_eq!(percent(25, 100), Some(25.0));
        assert_eq!(percent(110, 100), Some(100.0));
        assert_eq!(percent(u64::MAX, u64::MAX), Some(100.0));
    }

    #[test]
    fn gpu_gauge_falls_back_only_when_all_gpu_information_is_missing() {
        assert!(show_gpu(None, None), "inventory is still loading");
        for value in ["", "  ", "Unavailable", "Not reported"] {
            let hardware = Hardware {
                memory_summary: None,
                gpu_card: None,
                sections: vec![("Graphics".into(), value.into())],
                note: None,
            };
            assert!(!show_gpu(None, Some(&hardware)));
            assert!(
                show_gpu(Some(0.0), Some(&hardware)),
                "idle is a valid reading"
            );
        }
        let hardware = Hardware {
            memory_summary: None,
            gpu_card: None,
            sections: vec![("Graphics".into(), "Example GPU / driver 1.0".into())],
            note: None,
        };
        assert!(
            show_gpu(None, Some(&hardware)),
            "known GPU without utilization"
        );
        assert_eq!(
            hardware.graphics_details(),
            Some("Example GPU / driver 1.0")
        );
        let failed = Hardware {
            memory_summary: None,
            gpu_card: None,
            sections: Vec::new(),
            note: Some("Inventory failed".into()),
        };
        assert!(!show_gpu(None, Some(&failed)));
        assert!(
            show_gpu(Some(12.0), Some(&failed)),
            "counters work without inventory"
        );
    }

    fn sentence(segments: &[(String, bool)]) -> String {
        segments.iter().map(|(text, _)| text.as_str()).collect()
    }

    #[test]
    fn insight_reports_steady_system() {
        let segments = insight_segments(Some(23.0), Some(57.0), &[("C:".into(), 41.0)]);
        assert_eq!(sentence(&segments), "Everything is steady.");
        assert!(segments.iter().all(|(_, emphasis)| !emphasis));
    }

    #[test]
    fn insight_names_the_fullest_high_disk() {
        let disks = [
            ("C:".into(), 84.0),
            ("D:".into(), 91.0),
            ("E:".into(), 20.0),
        ];
        let segments = insight_segments(Some(10.0), Some(40.0), &disks);
        assert_eq!(
            sentence(&segments),
            "Everything is steady. Disk D: is running high at 91%."
        );
        assert_eq!(segments.iter().filter(|(_, emphasis)| *emphasis).count(), 1);
    }

    #[test]
    fn insight_lists_busy_compute_without_claiming_steadiness() {
        let segments = insight_segments(Some(95.0), Some(82.4), &[]);
        assert_eq!(
            sentence(&segments),
            "CPU is running high at 95%. Memory is running high at 82%."
        );
    }

    #[test]
    fn drive_labels_drop_trailing_separators() {
        assert_eq!(drive_label("C:\\"), "C:");
        assert_eq!(drive_label("/"), "/");
        assert_eq!(drive_label("/mnt/data/"), "/mnt/data");
    }
}
