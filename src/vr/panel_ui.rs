mod paint;
use super::menu::Command;
use crate::theme::{CANVAS, INK, LINE, MUTE, NAV, PAPER, TEXT};
use eframe::egui::Color32;
pub(super) use paint::{Icon, Renderer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) w: i32,
    pub(super) h: i32,
}
impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }
    fn inset(self, n: i32) -> Self {
        Self::new(self.x + n, self.y + n, self.w - 2 * n, self.h - 2 * n)
    }
    fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x as f32
            && y >= self.y as f32
            && x < (self.x + self.w) as f32
            && y < (self.y + self.h) as f32
    }
    fn contains_rounded(self, x: f32, y: f32, r: i32) -> bool {
        if !self.contains(x, y) {
            return false;
        }
        let r = r.min(self.w / 2).min(self.h / 2) as f32;
        let cx = x.clamp(self.x as f32 + r, (self.x + self.w) as f32 - r);
        let cy = y.clamp(self.y as f32 + r, (self.y + self.h) as f32 - r);
        (x - cx).powi(2) + (y - cy).powi(2) <= r * r
    }
}
#[derive(Debug, PartialEq)]
pub(super) enum Part {
    Surface {
        rect: Rect,
        radius: i32,
        fill: Color32,
        stroke: Option<Color32>,
        command: Option<Command>,
    },
    Label(Rect, String, i32, bool, bool, Color32),
    Icon(Rect, Icon, Color32),
}
#[derive(Debug, PartialEq)]
pub struct Layout {
    pub(super) parts: Vec<Part>,
    width: i32,
    height: i32,
}
pub(super) fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect::new(x, y, w, h)
}
impl Default for Layout {
    fn default() -> Self {
        Self::new(super::wrist::WIDTH, super::wrist::HEIGHT)
    }
}
impl Layout {
    pub(super) fn new(width: i32, height: i32) -> Self {
        Self {
            parts: Vec::new(),
            width,
            height,
        }
    }
    pub(super) fn append_at(&mut self, other: Self, dx: i32, dy: i32) {
        for mut part in other.parts {
            let rect = match &mut part {
                Part::Surface { rect, .. } | Part::Label(rect, ..) | Part::Icon(rect, ..) => rect,
            };
            rect.x += dx;
            rect.y += dy;
            self.parts.push(part);
        }
    }

    pub fn hit(&self, u: f32, v: f32) -> Option<&Command> {
        self.target(u, v).map(|(_, command)| command)
    }
    pub(super) fn target(&self, u: f32, v: f32) -> Option<(usize, &Command)> {
        self.target_pixel(u * self.width as f32, (1. - v) * self.height as f32)
    }
    pub(super) fn target_pixel(&self, x: f32, y: f32) -> Option<(usize, &Command)> {
        self.parts
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, p)| match p {
                Part::Surface {
                    rect,
                    radius,
                    command: Some(cmd),
                    ..
                } if rect.contains_rounded(x, y, *radius) => Some((index, cmd)),
                _ => None,
            })
    }
    pub(super) fn surface(
        &mut self,
        rect: Rect,
        radius: i32,
        fill: Color32,
        stroke: Option<Color32>,
        command: Option<Command>,
    ) {
        self.parts.push(Part::Surface {
            rect,
            radius,
            fill,
            stroke,
            command,
        });
    }
    pub(super) fn fill(&mut self, b: Rect, radius: i32, c: Color32) {
        self.surface(b, radius, c, None, None);
    }
    pub(super) fn button(&mut self, b: Rect, fill: Color32, stroke: Option<Color32>, cmd: Command) {
        self.surface(b, 11, fill, stroke, Some(cmd));
    }
    pub(super) fn label(&mut self, b: Rect, s: &str, px: i32, bold: bool, c: Color32) {
        self.parts
            .push(Part::Label(b, s.into(), px, bold, false, c));
    }
    pub(super) fn center(&mut self, b: Rect, s: &str, px: i32, bold: bool, c: Color32) {
        self.parts.push(Part::Label(b, s.into(), px, bold, true, c));
    }
    pub(super) fn icon(&mut self, x: i32, y: i32, size: i32, id: Icon, c: Color32) {
        self.parts.push(Part::Icon(r(x, y, size, size), id, c));
    }
    pub(super) fn toggle(&mut self, x: i32, y: i32, on: bool, accent: Color32) {
        self.fill(r(x, y, 54, 30), 15, if on { accent } else { NAV });
        self.fill(
            r(x + if on { 28 } else { 4 }, y + 4, 22, 22),
            11,
            if on { INK } else { MUTE },
        );
    }
    pub(super) fn icon_button(&mut self, b: Rect, id: Icon, cmd: Option<Command>) {
        let enabled = cmd.is_some();
        self.surface(b, 10, if enabled { PAPER } else { CANVAS }, Some(LINE), cmd);
        self.icon(
            b.x + (b.w - 26) / 2,
            b.y + (b.h - 26) / 2,
            26,
            id,
            if enabled { TEXT } else { LINE },
        );
    }
}
