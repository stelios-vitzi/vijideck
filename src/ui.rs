//! "Glacier" visual language: cold ink neutrals, ice-violet accent, glacier teal
//! as a second data tone, cold rose warnings. Flat surfaces separated by space
//! and hairlines rather than cards.
use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use std::sync::Arc;

/// Main canvas.
pub const BG: Color32 = Color32::from_rgb(19, 26, 38);
/// Navigation rail and recessed areas.
pub const PANEL: Color32 = Color32::from_rgb(15, 21, 31);
/// Raised surfaces such as dialogs and popups.
pub const RAISE: Color32 = Color32::from_rgb(30, 39, 53);
/// Subtle fill for inputs, chips, and hover states.
pub const SURFACE: Color32 = Color32::from_rgb(30, 39, 52);
pub const FILL_2: Color32 = Color32::from_rgb(41, 50, 64);
/// Hairline separators.
pub const LINE: Color32 = Color32::from_rgb(37, 46, 60);
/// Unfilled gauge and meter tracks.
pub const TRACK: Color32 = Color32::from_rgb(40, 49, 63);
pub const TEXT: Color32 = Color32::from_rgb(234, 239, 247);
pub const TEXT_2: Color32 = Color32::from_rgb(169, 178, 191);
pub const MUTED: Color32 = Color32::from_rgb(125, 135, 150);
pub const ACCENT: Color32 = Color32::from_rgb(158, 168, 255);
pub const WARNING: Color32 = Color32::from_rgb(243, 127, 176);
pub const DANGER: Color32 = Color32::from_rgb(238, 100, 118);
pub const SELECTED_FILL: Color32 = Color32::from_rgb(40, 47, 74);
pub const G_CPU: Color32 = Color32::from_rgb(169, 165, 255);
pub const G_MEM: Color32 = Color32::from_rgb(92, 216, 209);
pub const G_GPU: Color32 = Color32::from_rgb(98, 187, 245);

/// Light numerals used for large readouts.
pub fn light(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("light".into()))
}
pub fn mono_light(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("mono-light".into()))
}
pub fn medium(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("medium".into()))
}

fn fonts() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    let faces: [(&str, &'static [u8]); 5] = [
        ("geist", include_bytes!("../assets/fonts/Geist-Regular.ttf")),
        (
            "geist-light",
            include_bytes!("../assets/fonts/Geist-Light.ttf"),
        ),
        (
            "geist-medium",
            include_bytes!("../assets/fonts/Geist-Medium.ttf"),
        ),
        (
            "geist-mono",
            include_bytes!("../assets/fonts/GeistMono-Regular.ttf"),
        ),
        (
            "geist-mono-light",
            include_bytes!("../assets/fonts/GeistMono-Light.ttf"),
        ),
    ];
    for (name, data) in faces {
        fonts
            .font_data
            .insert(name.into(), Arc::new(egui::FontData::from_static(data)));
    }
    // Keep egui's bundled fonts as fallbacks for symbols Geist does not cover.
    let fallback = fonts.families[&egui::FontFamily::Proportional].clone();
    let mono_fallback = fonts.families[&egui::FontFamily::Monospace].clone();
    let family = |first: &str, rest: &[String]| {
        std::iter::once(first.to_owned())
            .chain(rest.iter().cloned())
            .collect::<Vec<_>>()
    };
    let families = [
        (egui::FontFamily::Proportional, family("geist", &fallback)),
        (
            egui::FontFamily::Monospace,
            family("geist-mono", &mono_fallback),
        ),
        (
            egui::FontFamily::Name("light".into()),
            family("geist-light", &fallback),
        ),
        (
            egui::FontFamily::Name("medium".into()),
            family("geist-medium", &fallback),
        ),
        (
            egui::FontFamily::Name("mono-light".into()),
            family("geist-mono-light", &mono_fallback),
        ),
    ];
    for (name, list) in families {
        fonts.families.insert(name, list);
    }
    fonts
}

pub fn theme(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    let v = &mut style.visuals;
    v.override_text_color = Some(TEXT);
    v.panel_fill = BG;
    v.window_fill = RAISE;
    v.extreme_bg_color = PANEL;
    v.faint_bg_color = Color32::from_rgb(24, 32, 45);
    v.code_bg_color = SURFACE;
    v.window_stroke = Stroke::NONE;
    v.window_corner_radius = 18.into();
    v.window_shadow = egui::Shadow {
        offset: [0, 20],
        blur: 60,
        spread: 0,
        color: Color32::from_black_alpha(140),
    };
    v.popup_shadow = egui::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(110),
    };
    v.menu_corner_radius = 10.into();
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = WARNING;
    v.error_fg_color = DANGER;
    v.selection.bg_fill = SELECTED_FILL;
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    v.widgets.noninteractive.bg_fill = BG;
    v.widgets.noninteractive.weak_bg_fill = BG;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, LINE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, MUTED);
    v.widgets.inactive.bg_fill = SURFACE;
    v.widgets.inactive.weak_bg_fill = SURFACE;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT_2);
    v.widgets.hovered.bg_fill = FILL_2;
    v.widgets.hovered.weak_bg_fill = FILL_2;
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.hovered.expansion = 0.0;
    v.widgets.active.bg_fill = SELECTED_FILL;
    v.widgets.active.weak_bg_fill = SELECTED_FILL;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT.gamma_multiply(0.6));
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, ACCENT);
    v.widgets.active.expansion = 0.0;
    v.widgets.open = v.widgets.active;
    for widget in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        widget.corner_radius = 8.into();
    }
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.interact_size.y = 32.0;
    style.spacing.scroll.bar_width = 5.0;
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(13.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, egui::FontId::proportional(11.0));
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, egui::FontId::monospace(12.0));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, light(34.0));
    ctx.set_style(style);
}

pub fn panel(fill: Color32, margin: i8) -> egui::Frame {
    egui::Frame::new().fill(fill).inner_margin(margin)
}

/// Unfilled frame for main content panels so the [`backdrop`] shows through.
pub fn canvas(margin: i8) -> egui::Frame {
    panel(Color32::TRANSPARENT, margin)
}

/// Paints the window backdrop: a soft diagonal gradient from a faintly violet
/// top-left to a deeper, teal-leaning bottom-right. Call before any panels.
pub fn backdrop(ctx: &egui::Context) {
    let rect = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::background());
    let mut mesh = egui::Mesh::default();
    let corners = [
        (rect.left_top(), Color32::from_rgb(27, 33, 50)),
        (rect.right_top(), Color32::from_rgb(21, 30, 43)),
        (rect.right_bottom(), Color32::from_rgb(14, 22, 31)),
        (rect.left_bottom(), Color32::from_rgb(17, 23, 35)),
    ];
    for (pos, color) in corners {
        mesh.colored_vertex(pos, color);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}

/// Frame shared by every text input: recessed panel fill with a hairline border.
pub fn input_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0_f32, LINE))
        .corner_radius(5)
        .inner_margin(egui::Margin::symmetric(12, INPUT_MARGIN_Y))
}

const INPUT_MARGIN_Y: i8 = 5;

/// Standard single-line text input. `width` is the outer width including the
/// frame; the height matches buttons and combo boxes (`interact_size.y`) so
/// inputs line up with the controls beside them.
pub fn text_input(
    ui: &mut egui::Ui,
    text: &mut String,
    hint: impl Into<egui::WidgetText>,
    width: f32,
) -> egui::Response {
    // Frame adds the vertical margin on both sides plus a 1 px stroke each side.
    let inner = (ui.spacing().interact_size.y - 2.0 * f32::from(INPUT_MARGIN_Y) - 2.0).max(16.0);
    input_frame()
        .show(ui, |ui| {
            ui.add_sized(
                [(width - 26.0).max(40.0), inner],
                egui::TextEdit::singleline(text)
                    .hint_text(hint)
                    .frame(false),
            )
        })
        .inner
}

pub fn caption(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(MUTED));
}

/// Small uppercase section label, as used above lists.
pub fn eyebrow(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .font(medium(12.0))
            .extra_letter_spacing(0.8)
            .color(MUTED),
    );
}

pub fn rule(ui: &mut egui::Ui) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, Stroke::new(1.0_f32, LINE));
}

pub fn dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), 5.0, color.gamma_multiply(0.15));
    ui.painter().circle_filled(rect.center(), 3.0, color);
}

pub fn badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    // Measure first and allocate the whole badge as one rect, so a wrapping
    // parent layout (`horizontal_wrapped`) can move it to the next line.
    let galley = egui::WidgetText::from(RichText::new(text).monospace().size(11.0).color(color))
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Monospace,
        );
    let margin = Vec2::new(7.0, 3.0);
    let (rect, _) = ui.allocate_exact_size(galley.size() + 2.0 * margin, egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let fill = if color == MUTED {
            FILL_2
        } else {
            color.gamma_multiply(0.12)
        };
        painter.rect_filled(rect, 5, fill);
        painter.galley(rect.min + margin, galley, color);
    }
}

pub fn field(ui: &mut egui::Ui, label: &str, value: &str) {
    caption(ui, label);
    ui.add(
        egui::Label::new(RichText::new(value).size(14.0))
            .wrap()
            .selectable(true),
    );
    ui.add_space(6.0);
}

pub fn metric(ui: &mut egui::Ui, label: &str, value: &str) {
    caption(ui, label);
    ui.label(RichText::new(value).font(light(28.0)).color(TEXT));
}

/// Segmented-control item: a pill that lifts when pressed.
pub fn tab(ui: &mut egui::Ui, selected: bool, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).size(13.0).color(if selected {
            TEXT
        } else {
            TEXT_2
        }))
        .fill(if selected { RAISE } else { SURFACE })
        .stroke(Stroke::NONE)
        .corner_radius(255),
    )
}

/// The VijiDeck mark at a given size.
pub fn reactor(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    mark(ui.painter(), rect, ACCENT);
}

/// The VijiDeck mark: a small gauge arc with a needle.
pub fn mark(p: &egui::Painter, rect: egui::Rect, color: Color32) {
    let s = rect.width() / 24.0;
    let center = rect.center() + egui::vec2(0.0, 2.0 * s);
    let stroke = Stroke::new((1.6 * s).max(1.4), color);
    let points: Vec<_> = (0..=24)
        .map(|i| {
            let a = std::f32::consts::PI * (1.0 + i as f32 / 24.0);
            center + Vec2::angled(a) * 8.0 * s
        })
        .collect();
    p.add(egui::Shape::line(points, stroke));
    p.line_segment([center, center + egui::vec2(4.5, -4.5) * s], stroke);
    p.circle_filled(center, 1.3 * s, color);
}

pub fn disk_glyph(p: &egui::Painter, rect: egui::Rect, color: Color32) {
    let r = rect.shrink(rect.width() * 0.12);
    p.rect_stroke(
        r,
        3.0,
        Stroke::new(1.3_f32, color),
        egui::StrokeKind::Inside,
    );
    let y = r.bottom() - r.height() * 0.25;
    p.line_segment(
        [egui::pos2(r.left(), y), egui::pos2(r.right(), y)],
        Stroke::new(1.2_f32, color),
    );
    p.circle_stroke(
        egui::pos2(r.center().x, r.top() + r.height() * 0.35),
        r.width() * 0.17,
        Stroke::new(1.1_f32, color),
    );
    p.circle_filled(egui::pos2(r.right() - 4.0, r.bottom() - 3.5), 1.2, color);
}

/// Text-only navigation item with a mono index, as in the Glacier rail.
/// Hairline navigation icons, drawn in a 16 pt box so they follow the theme
/// colour and stay crisp at any scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Overview,
    Computer,
    Ports,
    Processes,
    Services,
    Docker,
    Cleanup,
    Environment,
    Logs,
}

impl Glyph {
    pub fn paint(self, p: &egui::Painter, center: egui::Pos2, color: Color32) {
        use std::f32::consts::PI;
        let s = Stroke::new(1.3_f32, color);
        let at = |x: f32, y: f32| center + Vec2::new(x, y);
        let line = |points: &[(f32, f32)]| {
            p.add(egui::Shape::line(
                points.iter().map(|&(x, y)| at(x, y)).collect(),
                s,
            ));
        };
        let frame = |w: f32, h: f32, dy: f32, radius: f32| {
            p.rect_stroke(
                egui::Rect::from_center_size(at(0.0, dy), Vec2::new(w, h)),
                radius,
                s,
                egui::StrokeKind::Middle,
            );
        };
        match self {
            Self::Overview => {
                let arc = (0..=24)
                    .map(|i| center + Vec2::angled(PI * 0.75 + PI * 1.5 * i as f32 / 24.0) * 6.5)
                    .collect();
                p.add(egui::Shape::line(arc, s));
                p.line_segment([center, center + Vec2::angled(-PI * 0.3) * 4.2], s);
                p.circle_filled(center, 1.4, color);
            }
            Self::Computer => {
                frame(15.0, 10.0, -1.5, 1.5);
                line(&[(0.0, 3.5), (0.0, 6.0)]);
                line(&[(-3.5, 6.0), (3.5, 6.0)]);
            }
            Self::Ports => {
                let nodes = [(0.0, -5.5), (-5.2, 4.2), (5.2, 4.2)];
                for (x, y) in nodes {
                    let node = at(x, y);
                    p.line_segment([center + (node - center).normalized() * 2.3, node], s);
                    p.circle_filled(node, 1.9, color);
                }
                p.circle_stroke(center, 2.3, s);
            }
            Self::Processes => line(&[
                (-7.0, 0.5),
                (-3.8, 0.5),
                (-1.8, -5.5),
                (1.6, 6.0),
                (3.6, 0.5),
                (7.0, 0.5),
            ]),
            Self::Services => {
                p.circle_stroke(center, 4.8, s);
                p.circle_stroke(center, 1.8, s);
                for i in 0..8 {
                    let direction = Vec2::angled(PI / 4.0 * i as f32);
                    p.line_segment(
                        [center + direction * 4.8, center + direction * 7.0],
                        Stroke::new(2.0_f32, color),
                    );
                }
            }
            Self::Docker => {
                let r = 7.0;
                let (dx, dy) = (r * 0.866, r * 0.5);
                line(&[
                    (0.0, -r),
                    (dx, -dy),
                    (dx, dy),
                    (0.0, r),
                    (-dx, dy),
                    (-dx, -dy),
                    (0.0, -r),
                ]);
                line(&[(-dx, -dy), (0.0, 0.0), (dx, -dy)]);
                line(&[(0.0, 0.0), (0.0, r)]);
            }
            Self::Cleanup => {
                line(&[(-6.0, -4.5), (6.0, -4.5)]);
                line(&[(-2.0, -4.5), (-2.0, -6.5), (2.0, -6.5), (2.0, -4.5)]);
                line(&[(-4.5, -4.5), (-3.7, 6.5), (3.7, 6.5), (4.5, -4.5)]);
                line(&[(-1.3, -1.5), (-1.3, 3.5)]);
                line(&[(1.3, -1.5), (1.3, 3.5)]);
            }
            Self::Environment => {
                frame(15.0, 12.0, 0.0, 2.0);
                line(&[(-4.0, -2.0), (-1.5, 0.0), (-4.0, 2.0)]);
                line(&[(0.5, 2.5), (4.0, 2.5)]);
            }
            Self::Logs => {
                frame(11.0, 14.0, 0.0, 1.5);
                line(&[(-2.8, -3.0), (2.8, -3.0)]);
                line(&[(-2.8, 0.0), (2.8, 0.0)]);
                line(&[(-2.8, 3.0), (0.8, 3.0)]);
            }
        }
    }
}

pub fn nav_item(ui: &mut egui::Ui, glyph: Glyph, label: &str, selected: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), egui::Sense::click());
    let hovered = response.hovered() || response.has_focus();
    let p = ui.painter();
    if hovered && !selected {
        p.rect_filled(rect, 8, SURFACE);
    }
    glyph.paint(
        p,
        rect.left_center() + egui::vec2(19.0, 0.0),
        if selected {
            ACCENT
        } else if hovered {
            TEXT_2
        } else {
            MUTED.gamma_multiply(0.85)
        },
    );
    p.text(
        rect.left_center() + egui::vec2(40.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(14.0),
        if selected || hovered { TEXT } else { MUTED },
    );
    response
}

/// A 270° hairline gauge with an end-point bead and a centred light readout.
/// Values at or above 80% switch to the warning tone.
pub fn gauge_face(ui: &mut egui::Ui, size: f32, value: Option<f32>, color: Color32) {
    // The open bottom of a 270° arc needs no space of its own.
    let (rect, _) = ui.allocate_exact_size(Vec2::new(size, size * 0.86), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let color = gauge_color(value, color);
    let p = ui.painter();
    let center = rect.center_top() + egui::vec2(0.0, size / 2.0);
    let radius = size * 0.44;
    let width = (size / 100.0 * 2.0).clamp(1.5, 3.0);
    let start = std::f32::consts::PI * 0.75;
    let span = std::f32::consts::PI * 1.5;
    let arc = |from: f32, to: f32| -> Vec<egui::Pos2> {
        let steps = ((to - from).abs() * 40.0).ceil().max(2.0) as usize;
        (0..=steps)
            .map(|i| center + Vec2::angled(from + (to - from) * i as f32 / steps as f32) * radius)
            .collect()
    };
    p.add(egui::Shape::line(
        arc(start, start + span),
        Stroke::new(width, TRACK),
    ));
    if let Some(value) = value {
        let end = start + span * value.clamp(0.0, 100.0) / 100.0;
        if value > 0.5 {
            let points = arc(start, end);
            p.add(egui::Shape::line(
                points.clone(),
                Stroke::new(width * 4.0, color.gamma_multiply(0.06)),
            ));
            p.add(egui::Shape::line(
                points.clone(),
                Stroke::new(width * 2.2, color.gamma_multiply(0.12)),
            ));
            p.add(egui::Shape::line(points, Stroke::new(width, color)));
        }
        let bead = center + Vec2::angled(end) * radius;
        p.circle_filled(bead, size * 0.045, color.gamma_multiply(0.14));
        p.circle_filled(bead, size * 0.026, BG);
        p.circle_stroke(bead, size * 0.026, Stroke::new(width, color));
    }
    let readout = value.map_or_else(|| "—".to_owned(), |v| format!("{:.0}", v.clamp(0.0, 100.0)));
    let number = p.layout_no_wrap(readout, mono_light(size * 0.23), TEXT);
    let unit = p.layout_no_wrap("%".into(), mono_light(size * 0.09), MUTED);
    let unit_width = if value.is_some() {
        unit.size().x + 2.0
    } else {
        0.0
    };
    let number_size = number.size();
    let origin = center - egui::vec2((number_size.x + unit_width) / 2.0, number_size.y / 2.0);
    p.galley(origin, number, TEXT);
    if value.is_some() {
        let unit_top = origin.y + number_size.y * 0.78 - unit.size().y;
        p.galley(
            egui::pos2(origin.x + number_size.x + 2.0, unit_top),
            unit,
            MUTED,
        );
    }
}

pub fn gauge_color(value: Option<f32>, color: Color32) -> Color32 {
    if value.is_some_and(|v| v >= 80.0) {
        WARNING
    } else {
        color
    }
}

/// Thin usage bar with a soft glow on the filled part.
pub fn thin_bar(ui: &mut egui::Ui, fraction: f32, color: Color32) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 3.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 2, TRACK);
    let fill = egui::Rect::from_min_size(
        rect.min,
        egui::vec2(rect.width() * fraction.clamp(0.0, 1.0), rect.height()),
    );
    if fill.width() > 0.0 {
        p.rect_filled(fill.expand(2.0), 4, color.gamma_multiply(0.1));
        p.rect_filled(fill, 2, color);
    }
}

pub fn port_heading(ui: &mut egui::Ui, port: u16, protocol: &str, state: &str, compact: bool) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), if compact { 76.0 } else { 96.0 }),
        egui::Sense::hover(),
    );
    let painter = ui.painter();
    painter.text(
        rect.left_top() + egui::vec2(0.0, 8.0),
        egui::Align2::LEFT_TOP,
        "SELECTED PORT",
        medium(11.0),
        MUTED,
    );
    painter.text(
        rect.left_top() + egui::vec2(-2.0, 28.0),
        egui::Align2::LEFT_TOP,
        port.to_string(),
        mono_light(if compact { 38.0 } else { 50.0 }),
        ACCENT,
    );
    painter.text(
        rect.right_bottom() - egui::vec2(0.0, 14.0),
        egui::Align2::RIGHT_BOTTOM,
        format!("{protocol} · {state}"),
        egui::FontId::monospace(12.0),
        MUTED,
    );
}
