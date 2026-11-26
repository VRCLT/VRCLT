use super::{Command, Layout, Part, Rect};
use crate::theme::{CJK, PRETENDARD, PRETENDARD_BOLD, SKY, TEXT};
use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use eframe::egui::Color32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Brand,
    Voice,
    Text,
    Talk,
    Settings,
    Chevron,
    Back,
    Next,
    Mic,
    Placement,
    Captions,
    Globe,
    Restart,
    Play,
    Minus,
    Plus,
    Reset,
    Check,
    Refresh,
    Wrist,
}
pub struct Renderer {
    regular: FontRef<'static>,
    bold: FontRef<'static>,
    fallback: FontRef<'static>,
}
impl Default for Renderer {
    fn default() -> Self {
        Self {
            regular: FontRef::try_from_slice(PRETENDARD).unwrap(),
            bold: FontRef::try_from_slice(PRETENDARD_BOLD).unwrap(),
            fallback: FontRef::try_from_slice(CJK).unwrap(),
        }
    }
}
struct Canvas {
    pixels: Vec<u8>,
    width: i32,
    height: i32,
}
impl Canvas {
    fn dot(&mut self, x: i32, y: i32, c: Color32, alpha: f32) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height || alpha <= 0. {
            return;
        }
        let i = ((self.width * y + x) * 4) as usize;
        let pixel = &mut self.pixels[i..i + 4];
        let color = c.to_array();
        if alpha >= 0.999 {
            pixel.copy_from_slice(&color);
            return;
        }
        let a = alpha.clamp(0., 1.);
        let old = (pixel[3] as f32 / 255.) * (1. - a);
        let total = a + old;
        for j in 0..3 {
            pixel[j] = ((color[j] as f32 * a + pixel[j] as f32 * old) / total).round() as u8;
        }
        pixel[3] = (255. * total).round() as u8;
    }
    fn round(&mut self, b: Rect, r: i32, c: Color32) {
        let radius = r.min(b.w / 2).min(b.h / 2) as f32;
        // Fill complete scanline spans in one pass; only the two edge pixels need blending.
        for y in b.y.max(0)..(b.y + b.h).min(self.height) {
            let dy = (radius - ((y - b.y).min(b.y + b.h - 1 - y) as f32 + 0.5)).max(0.);
            let inset = radius - (radius * radius - dy * dy).max(0.).sqrt();
            let left = b.x as f32 + inset;
            let right = (b.x + b.w) as f32 - inset;
            let start = left.ceil() as i32;
            let end = right.floor() as i32;
            self.dot(start - 1, y, c, start as f32 - left);
            self.dot(end, y, c, right - end as f32);
            let a = start.clamp(0, self.width);
            let z = end.clamp(0, self.width);
            if z > a {
                let bytes = &mut self.pixels
                    [((y * self.width + a) * 4) as usize..((y * self.width + z) * 4) as usize];
                let color = c.to_array();
                for pixel in bytes.chunks_exact_mut(4) {
                    pixel.copy_from_slice(&color);
                }
            }
        }
    }
    fn line(&mut self, a: (f32, f32), b: (f32, f32), w: f32, c: Color32) {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).max(0.001);
        let r = w / 2. + 0.5;
        for y in (a.1.min(b.1) - r).floor() as i32..=(a.1.max(b.1) + r).ceil() as i32 {
            for x in (a.0.min(b.0) - r).floor() as i32..=(a.0.max(b.0) + r).ceil() as i32 {
                let (u, v) = (x as f32 + 0.5 - a.0, y as f32 + 0.5 - a.1);
                let t = ((u * dx + v * dy) / len).clamp(0., 1.);
                self.dot(
                    x,
                    y,
                    c,
                    r - ((u - t * dx).powi(2) + (v - t * dy).powi(2)).sqrt(),
                );
            }
        }
    }
    fn icon(&mut self, icon: Icon, b: Rect, c: Color32) {
        let s = b.w as f32 / 24.;
        let pos = |x: f32, y: f32| (b.x as f32 + s * x, b.y as f32 + s * y);
        let path = |canvas: &mut Self, pts: &[(f32, f32)]| {
            for p in pts.windows(2) {
                canvas.line(pos(p[0].0, p[0].1), pos(p[1].0, p[1].1), 2. * s, c);
            }
        };
        let bounds = |x: i32, y: i32, w: i32, h: i32| {
            Rect::new(
                b.x + (x as f32 * s) as i32,
                b.y + (y as f32 * s) as i32,
                (w as f32 * s).round() as i32,
                (h as f32 * s).round() as i32,
            )
        };
        let ring = |canvas: &mut Self, x: f32, y: f32, rx: f32, ry: f32| {
            let mut prev = pos(x + rx, y);
            for i in 1..=24 {
                let a = i as f32 * std::f32::consts::TAU / 24.;
                let next = pos(x + rx * a.cos(), y + ry * a.sin());
                canvas.line(prev, next, 1.8 * s, c);
                prev = next;
            }
        };
        match icon {
            Icon::Brand | Icon::Talk => {
                let (a, d) = if icon == Icon::Brand {
                    (TEXT, SKY)
                } else {
                    (c, c)
                };
                self.round(bounds(1, 2, 14, 10), (3. * s) as i32, a);
                self.line(pos(5., 11.), pos(4., 14.), 2.5 * s, a);
                self.round(bounds(9, 10, 14, 10), (3. * s) as i32, d);
                self.line(pos(18., 19.), pos(20., 22.), 2.5 * s, d);
            }
            Icon::Next | Icon::Chevron => path(self, &[(9., 5.), (16., 12.), (9., 19.)]),
            Icon::Back => path(self, &[(15., 5.), (8., 12.), (15., 19.)]),
            Icon::Play => path(self, &[(7., 4.), (20., 12.), (7., 20.), (7., 4.)]),
            Icon::Voice => {
                path(
                    self,
                    &[
                        (2., 10.),
                        (6., 10.),
                        (11., 5.),
                        (11., 19.),
                        (6., 14.),
                        (2., 14.),
                        (2., 10.),
                    ],
                );
                path(self, &[(15., 8.), (17., 12.), (15., 16.)]);
                path(self, &[(18., 5.), (21., 12.), (18., 19.)]);
            }
            Icon::Text => {
                path(
                    self,
                    &[
                        (3., 5.),
                        (21., 5.),
                        (21., 16.),
                        (11., 16.),
                        (6., 20.),
                        (6., 16.),
                        (3., 16.),
                        (3., 5.),
                    ],
                );
                path(self, &[(7., 10.), (17., 10.)]);
            }
            Icon::Settings => {
                for (y, x) in [(5., 8.), (12., 16.), (19., 10.)] {
                    path(self, &[(3., y), (x - 3., y)]);
                    path(self, &[(x + 3., y), (21., y)]);
                    ring(self, x, y, 2.5, 2.5);
                }
            }
            Icon::Mic => {
                ring(self, 12., 9., 3.5, 6.5);
                path(
                    self,
                    &[
                        (5., 11.),
                        (6., 15.),
                        (9., 18.),
                        (15., 18.),
                        (18., 15.),
                        (19., 11.),
                    ],
                );
                path(self, &[(12., 18.), (12., 22.)]);
            }
            Icon::Placement => {
                path(self, &[(3., 9.), (3., 3.), (9., 3.)]);
                path(self, &[(15., 3.), (21., 3.), (21., 9.)]);
                path(self, &[(21., 15.), (21., 21.), (15., 21.)]);
                path(self, &[(9., 21.), (3., 21.), (3., 15.)]);
            }
            Icon::Captions => {
                path(
                    self,
                    &[(2., 5.), (22., 5.), (22., 19.), (2., 19.), (2., 5.)],
                );
                path(self, &[(5., 14.), (11., 14.)]);
                path(self, &[(14., 14.), (19., 14.)]);
            }
            Icon::Globe => {
                ring(self, 12., 12., 9., 9.);
                ring(self, 12., 12., 4., 9.);
                path(self, &[(3., 12.), (21., 12.)]);
            }
            Icon::Restart | Icon::Refresh | Icon::Reset => {
                path(
                    self,
                    &[
                        (5., 8.),
                        (8., 5.),
                        (13., 4.),
                        (18., 6.),
                        (20., 11.),
                        (19., 16.),
                        (15., 19.),
                        (9., 19.),
                        (6., 17.),
                    ],
                );
                path(self, &[(4., 3.), (4., 9.), (10., 9.)]);
            }
            Icon::Plus | Icon::Minus => {
                path(self, &[(5., 12.), (19., 12.)]);
                if icon == Icon::Plus {
                    path(self, &[(12., 5.), (12., 19.)]);
                }
            }
            Icon::Check => path(self, &[(4., 12.), (9., 17.), (20., 6.)]),
            Icon::Wrist => {
                path(
                    self,
                    &[
                        (8., 2.),
                        (16., 2.),
                        (17., 7.),
                        (19., 8.),
                        (19., 16.),
                        (17., 17.),
                        (16., 22.),
                        (8., 22.),
                        (7., 17.),
                        (5., 16.),
                        (5., 8.),
                        (7., 7.),
                        (8., 2.),
                    ],
                );
                path(self, &[(9., 12.), (15., 12.)]);
            }
        }
    }
}
impl Renderer {
    fn font(&self, ch: char, bold: bool) -> &FontRef<'static> {
        let f = if bold { &self.bold } else { &self.regular };
        if f.glyph_id(ch).0 != 0 {
            f
        } else {
            &self.fallback
        }
    }
    fn advance(&self, ch: char, size: f32, bold: bool) -> f32 {
        let f = self.font(ch, bold);
        f.as_scaled(size).h_advance(f.glyph_id(ch))
    }
    fn text(
        &self,
        canvas: &mut Canvas,
        b: Rect,
        text: &str,
        size: f32,
        bold: bool,
        centered: bool,
        color: Color32,
    ) {
        let units: f32 = text.chars().map(|c| self.advance(c, 1., bold)).sum();
        let size = size.min(b.w as f32 / units.max(0.01)).max(size * 0.68);
        let mut chars: Vec<(char, f32)> = Vec::new();
        let mut width = 0.;
        for c in text.chars() {
            let w = self.advance(c, size, bold);
            if width + w > b.w as f32 + 0.01 {
                let ell = self.advance('…', size, bold);
                while width + ell > b.w as f32 && !chars.is_empty() {
                    width -= chars.pop().unwrap().1;
                }
                chars.push(('…', ell));
                width += ell;
                break;
            }
            chars.push((c, w));
            width += w;
        }
        let f = if bold { &self.bold } else { &self.regular };
        let metrics = f.as_scaled(size);
        let mut x = b.x as f32
            + if centered {
                (b.w as f32 - width) / 2.
            } else {
                0.
            };
        let y = b.y as f32 + (b.h as f32 - metrics.height()) / 2. + metrics.ascent();
        for (c, advance) in chars {
            let font = self.font(c, bold);
            let glyph = font
                .glyph_id(c)
                .with_scale_and_position(PxScale::from(size), point(x, y));
            if let Some(outline) = font.outline_glyph(glyph) {
                let bounds = outline.px_bounds();
                outline.draw(|dx, dy, a| {
                    let x = bounds.min.x as i32 + dx as i32;
                    let y = bounds.min.y as i32 + dy as i32;
                    if b.contains(x as f32, y as f32) {
                        canvas.dot(x, y, color, a);
                    }
                });
            }
            x += advance;
        }
    }
    pub fn render(
        &self,
        layout: &Layout,
        hover: Option<&Command>,
        pressed: Option<&Command>,
    ) -> Vec<u8> {
        let index = |command: &Command| {
            layout
                .parts
                .iter()
                .position(|p| matches!(p, Part::Surface{command: Some(c), ..} if c == command))
        };
        self.render_targets(layout, hover.and_then(index), pressed.and_then(index))
    }
    pub fn render_targets(
        &self,
        layout: &Layout,
        hover: Option<usize>,
        pressed: Option<usize>,
    ) -> Vec<u8> {
        let mut canvas = Canvas {
            pixels: vec![0; (layout.width * layout.height * 4) as usize],
            width: layout.width,
            height: layout.height,
        };
        for (index, p) in layout.parts.iter().enumerate() {
            match p {
                Part::Surface {
                    rect,
                    radius,
                    fill,
                    stroke,
                    ..
                } => {
                    let hot = Some(index) == hover;
                    let down = hot && Some(index) == pressed;
                    let border = if hot {
                        Some(if down { TEXT } else { SKY })
                    } else {
                        *stroke
                    };
                    let inner = if down {
                        Color32::from_rgb(
                            fill.r().saturating_add(16),
                            fill.g().saturating_add(18),
                            fill.b().saturating_add(20),
                        )
                    } else {
                        *fill
                    };
                    if let Some(color) = border {
                        let n = if hot { 3 } else { 1 };
                        canvas.round(*rect, *radius, color);
                        canvas.round(rect.inset(n), (*radius - n).max(0), inner);
                    } else {
                        canvas.round(*rect, *radius, inner);
                    }
                }
                Part::Label(rect, s, size, weight, center, c) => {
                    self.text(&mut canvas, *rect, s, *size as f32, *weight, *center, *c)
                }
                Part::Icon(rect, id, c) => canvas.icon(*id, *rect, *c),
            }
        }
        canvas.pixels
    }
}
