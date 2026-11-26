use eframe::egui::{self, *};
pub const INK: Color32 = Color32::from_rgb(19, 23, 32);
pub const NAV: Color32 = Color32::from_rgb(46, 55, 73);
pub const MUTE: Color32 = Color32::from_rgb(164, 177, 193);
pub const CANVAS: Color32 = Color32::from_rgb(24, 29, 40);
pub const PAPER: Color32 = Color32::from_rgb(34, 41, 55);
pub const TEXT: Color32 = Color32::from_rgb(237, 242, 247);
pub const LINE: Color32 = Color32::from_rgb(62, 73, 92);
pub const SKY: Color32 = Color32::from_rgb(152, 204, 248);
pub const BLUE: Color32 = Color32::from_rgb(152, 204, 248);
pub const MINT: Color32 = Color32::from_rgb(140, 209, 196);
pub const SOFT: Color32 = Color32::from_rgb(44, 66, 87);
pub const ALERT: Color32 = Color32::from_rgb(255, 181, 141);
pub static CJK: &[u8] = include_bytes!("../assets/fonts/NotoSansCJKkr-Regular.otf");
pub static PRETENDARD: &[u8] = include_bytes!("../assets/fonts/PretendardJP-Regular.otf");
pub static PRETENDARD_BOLD: &[u8] = include_bytes!("../assets/fonts/PretendardJP-Bold.otf");
pub fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        ("Pretendard", PRETENDARD),
        ("Bold", PRETENDARD_BOLD),
        ("CJK", CJK),
    ] {
        fonts
            .font_data
            .insert(name.into(), FontData::from_static(bytes).into());
    }
    let family = fonts.families.get_mut(&FontFamily::Proportional).unwrap();
    family.insert(0, "Pretendard".into());
    family.push("CJK".into());
    let mut bold = family.clone();
    bold.insert(0, "Bold".into());
    fonts
        .families
        .insert(FontFamily::Name("Heading".into()), bold);
    fonts
}
pub fn setup(ctx: &Context) {
    ctx.set_fonts(fonts());
    ctx.set_theme(Theme::Dark);
    let mut v = Visuals::dark();
    v.panel_fill = CANVAS;
    v.window_fill = PAPER;
    v.extreme_bg_color = INK;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = BLUE;
    v.widgets.inactive.bg_fill = PAPER;
    v.widgets.inactive.weak_bg_fill = PAPER;
    v.widgets.inactive.bg_stroke = Stroke::new(1_f32, LINE);
    v.widgets.hovered.bg_fill = SOFT;
    v.widgets.hovered.weak_bg_fill = SOFT;
    v.widgets.hovered.bg_stroke = Stroke::new(1_f32, BLUE);
    v.selection.bg_fill = SOFT;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1_f32, LINE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1_f32, TEXT);
    ctx.all_styles_mut(|s| {
        s.visuals = v.clone();
        s.spacing.item_spacing = vec2(9., 9.);
        s.spacing.button_padding = vec2(12., 7.);
        s.spacing.interact_size.y = 30.;
        s.text_styles
            .insert(TextStyle::Body, FontId::proportional(16.));
        s.text_styles
            .insert(TextStyle::Button, FontId::proportional(15.));
        s.visuals.window_corner_radius = 8.into();
    });
}
pub fn bold(s: &str, size: f32) -> RichText {
    RichText::new(s).font(FontId::new(size, FontFamily::Name("Heading".into())))
}
pub fn hint(ui: &mut Ui, s: &str) {
    ui.label(RichText::new(s).size(15.).color(MUTE));
}
pub fn card_frame() -> Frame {
    Frame::new()
        .fill(PAPER)
        .stroke(Stroke::new(1_f32, LINE))
        .inner_margin(18)
        .corner_radius(10)
}
pub fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    card_frame().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        add(ui);
    });
}
pub fn primary(ui: &mut Ui, text: &str) -> Response {
    ui.add(
        Button::new(bold(text, 16.).color(INK))
            .fill(BLUE)
            .stroke(Stroke::NONE)
            .min_size(vec2(130., 38.))
            .corner_radius(7),
    )
}
pub fn mark(p: &Painter, pos: Pos2, size: f32) {
    for (x, y, c, right) in [
        (0., 0., Color32::from_rgb(245, 243, 237), false),
        (0.41, 0.38, SKY, true),
    ] {
        let o = pos + vec2(x * size, y * size);
        let r = Rect::from_min_size(o, vec2(size * 0.60, size * 0.44));
        if right {
            p.rect_filled(r.expand(1.8), 4, INK);
        }
        p.rect_filled(r, 4, c);
        let a = if right { r.right() - 3. } else { r.left() + 3. };
        let sign = if right { -1. } else { 1. };
        p.add(Shape::convex_polygon(
            vec![
                pos2(a, r.bottom() - 2.),
                pos2(a, r.bottom() + size * 0.12),
                pos2(a + sign * size * 0.16, r.bottom() - 1.),
            ],
            c,
            Stroke::NONE,
        ));
    }
}
pub fn choice(
    ui: &mut Ui,
    id: &str,
    title: &str,
    description: &str,
    tag: &str,
    selected: bool,
) -> bool {
    let w = ui.available_width();
    let h = if description.is_empty() { 48. } else { 66. };
    let (r, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let p = ui.painter();
    p.rect(
        r,
        8,
        if selected { SOFT } else { PAPER },
        Stroke::new(
            if selected { 1.7_f32 } else { 1_f32 },
            if selected { BLUE } else { LINE },
        ),
        StrokeKind::Inside,
    );
    let center = pos2(r.left() + 23., r.center().y);
    p.circle_stroke(
        center,
        7.,
        Stroke::new(1.7_f32, if selected { BLUE } else { MUTE }),
    );
    if selected {
        p.circle_filled(center, 3.6, BLUE);
    }
    let y = if description.is_empty() {
        r.center().y
    } else {
        r.top() + 21.
    };
    let rect = p.text(
        pos2(r.left() + 44., y),
        Align2::LEFT_CENTER,
        title,
        FontId::new(18., FontFamily::Name("Heading".into())),
        TEXT,
    );
    if !tag.is_empty() {
        p.text(
            pos2(rect.right() + 12., y),
            Align2::LEFT_CENTER,
            tag,
            FontId::proportional(13.),
            BLUE,
        );
    }
    if !description.is_empty() {
        p.text(
            pos2(r.left() + 44., r.top() + 45.),
            Align2::LEFT_CENTER,
            description,
            FontId::proportional(14.),
            MUTE,
        );
    }
    resp.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, true, selected, id));
    if resp.has_focus() {
        p.rect_stroke(
            r.shrink(3.),
            6,
            Stroke::new(1_f32, BLUE),
            StrokeKind::Inside,
        );
    }
    resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
}
pub fn engines(ui: &mut Ui, selected: &str, l: crate::labels::Lang) -> Option<&'static str> {
    let mut picked = None;
    ui.columns(4, |cols| {
        for (col, (id, name, model, tag)) in cols.iter_mut().zip([
            (
                "soniox",
                "Soniox",
                "Speech-to-Text + TTS",
                l.t("가장 추천", "Top pick", "一番おすすめ", "首选推荐"),
            ),
            (
                "gemini",
                "Gemini",
                "3.5 Live Translate",
                l.t("무료 추천", "Free tier", "無料枠", "免费推荐"),
            ),
            ("qwen", "Qwen", "LiveTranslate", ""),
            ("openai", "GPT", "OpenAI Realtime", ""),
        ]) {
            let active = selected == id;
            let (r, response) =
                col.allocate_exact_size(vec2(col.available_width(), 76.), Sense::click());
            let p = col.painter();
            let border = if active || response.has_focus() {
                SKY
            } else if response.hovered() {
                MUTE
            } else {
                LINE
            };
            p.rect(
                r,
                7,
                if active { SOFT } else { PAPER },
                Stroke::new(if active { 1.6_f32 } else { 1_f32 }, border),
                StrokeKind::Inside,
            );
            let x = r.left() + 14.;
            let y = r.top() + 13.;
            p.text(
                pos2(x, y),
                Align2::LEFT_TOP,
                name,
                FontId::new(24., FontFamily::Name("Heading".into())),
                TEXT,
            );
            if !tag.is_empty() {
                p.text(
                    pos2(r.right() - 11., y + 2.),
                    Align2::RIGHT_TOP,
                    tag,
                    FontId::proportional(12.),
                    if id == "soniox" { MINT } else { SKY },
                );
            }
            p.text(
                pos2(x, r.bottom() - 12.),
                Align2::LEFT_BOTTOM,
                model,
                FontId::proportional(14.),
                MUTE,
            );
            response
                .widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, true, active, name));
            if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                picked = Some(id);
            }
        }
    });
    picked
}
pub fn meter(ui: &mut Ui, level: u32, inbound: bool) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.), Sense::hover());
    let p = ui.painter();
    let width = r.width() - 59.;
    let db = 20. * ((level.max(1) as f32) / 32768.).log10();
    let lit = (((db + 60.) / 60.).clamp(0., 1.) * 38.).ceil() as usize;
    for i in 0..38 {
        let w = width / 38.;
        let x = r.left() + w * i as f32;
        p.rect_filled(
            Rect::from_min_size(pos2(x, r.top() + 2.), vec2((w - 2.).max(1.), 9.)),
            1,
            if i >= lit {
                LINE
            } else if i > 34 {
                ALERT
            } else if inbound {
                MINT
            } else {
                SKY
            },
        );
    }
    for (part, s) in [(0., "−60"), (0.4, "−36"), (0.8, "−12"), (1., "0")] {
        p.text(
            pos2(r.left() + part * (width - 10.), r.top() + 15.),
            Align2::LEFT_TOP,
            s,
            FontId::monospace(10.),
            MUTE,
        );
    }
    let value = if level == 0 {
        "−∞ dB".to_owned()
    } else {
        format!("{db:.0} dB")
    };
    p.text(
        pos2(r.right(), r.top()),
        Align2::RIGHT_TOP,
        value,
        FontId::monospace(12.),
        if inbound { MINT } else { SKY },
    );
}
pub fn device(
    ui: &mut Ui,
    id: &str,
    label: &str,
    value: &mut String,
    names: &[String],
    empty: &str,
) {
    ui.label(bold(label, 15.));
    egui::ComboBox::from_id_salt(id)
        .width(ui.available_width().min(540.))
        .selected_text(if value.is_empty() {
            empty
        } else {
            value.as_str()
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(value, String::new(), empty);
            for n in names {
                ui.selectable_value(value, n.clone(), n);
            }
        });
}
pub fn resize(ctx: &Context) {
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let Some(p) = ctx.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    let r = ctx.screen_rect();
    let n = p.y - r.top() < 5.;
    let s = r.bottom() - p.y < 7.;
    let w = p.x - r.left() < 5.;
    let e = r.right() - p.x < 5.;
    use ResizeDirection::*;
    let (dir, icon) = match (n, s, w, e) {
        (true, _, true, _) | (_, true, _, true) => (
            if n { NorthWest } else { SouthEast },
            CursorIcon::ResizeNwSe,
        ),
        (true, _, _, true) | (_, true, true, _) => (
            if n { NorthEast } else { SouthWest },
            CursorIcon::ResizeNeSw,
        ),
        (_, _, true, _) | (_, _, _, true) => {
            (if w { West } else { East }, CursorIcon::ResizeHorizontal)
        }
        (true, _, _, _) | (_, true, _, _) => {
            (if n { North } else { South }, CursorIcon::ResizeVertical)
        }
        _ => return,
    };
    ctx.set_cursor_icon(icon);
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
    }
}
