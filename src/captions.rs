use crate::{
    labels::Lang,
    runtime::{Caption, Lane},
    theme,
};
use eframe::egui::{
    self, Color32, FontFamily, FontId, TextFormat,
    text::{Fonts, LayoutJob},
};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Row {
    pub label: String,
    pub color: Color32,
    original: String,
    pub body: String,
    pub partial: bool,
}
fn speaker_color(id: &str) -> Color32 {
    let palette = [
        [125, 190, 246],
        [238, 184, 110],
        [140, 212, 172],
        [204, 164, 232],
        [239, 152, 174],
        [129, 211, 218],
    ];
    let index = id
        .parse::<i64>()
        .map(|n| n.saturating_sub(1).max(0) as usize)
        .unwrap_or_else(|_| {
            id.chars()
                .enumerate()
                .map(|(i, ch)| (i + 1) * ch as usize)
                .sum()
        });
    let [r, g, b] = palette[index % palette.len()];
    Color32::from_rgb(r, g, b)
}
impl Row {
    pub fn new(c: &Caption, lang: Lang, source: bool, partial: bool, outbound: bool) -> Self {
        let me = lang.t("나", "Me", "自分", "我");
        let label = if c.speaker.is_empty() {
            if outbound { me.into() } else { String::new() }
        } else {
            let who = format!(
                "{} {}",
                lang.t("화자", "Speaker", "話者", "说话人"),
                c.speaker
            );
            if outbound {
                format!("{me} ({who})")
            } else {
                who
            }
        };
        let body = if c.text.trim().is_empty() {
            c.source.trim().into()
        } else {
            c.text.trim().into()
        };
        Self {
            label,
            original: if source && !c.text.trim().is_empty() {
                c.source.trim().into()
            } else {
                String::new()
            },
            color: if outbound && c.speaker.is_empty() {
                theme::SKY
            } else {
                speaker_color(&c.speaker)
            },
            body,
            partial,
        }
    }
    fn prefix_format(&self, size: f32) -> TextFormat {
        TextFormat {
            font_id: FontId::new(size, FontFamily::Name("Heading".into())),
            color: self.color,
            italics: self.partial,
            ..Default::default()
        }
    }
    fn body_format(&self, size: f32) -> TextFormat {
        TextFormat {
            font_id: FontId::new(size, FontFamily::Name("Heading".into())),
            italics: self.partial,
            color: if self.partial {
                theme::MUTE
            } else {
                theme::TEXT
            },
            ..Default::default()
        }
    }
    pub fn inline(&self, size: f32) -> LayoutJob {
        let mut job = LayoutJob::default();
        if !self.label.is_empty() {
            job.append(&self.label, 0., self.prefix_format(size));
        }
        self.append_text(&mut job, size, if self.label.is_empty() { 0. } else { 8. });
        job
    }
    fn append_text(&self, job: &mut LayoutJob, size: f32, indent: f32) {
        if self.original.is_empty() {
            job.append(&self.body, indent, self.body_format(size));
        } else {
            job.append(
                &format!("{}\n", self.original),
                indent,
                TextFormat {
                    font_id: FontId::proportional(size * 0.87),
                    color: theme::MUTE,
                    ..Default::default()
                },
            );
            job.append(&self.body, 0., self.body_format(size));
        }
    }
}

pub fn recent(lane: &Lane, lang: Lang, count: usize, seconds: f64, source: bool) -> Vec<Row> {
    let transcript = lane.captions.lock();
    let pending = transcript
        .overlay_draft
        .as_ref()
        .unwrap_or(&transcript.pending);
    let rows = transcript
        .history
        .iter()
        .map(|r| (r.body_changed, &r.caption, false, r.expects_translation))
        .chain(
            pending
                .iter()
                .map(|r| (r.body_changed, &r.caption, true, r.expects_translation)),
        );
    let mut visible: Vec<_> = rows
        .filter(|(at, c, _, expects_translation)| {
            at.elapsed().as_secs_f64() < seconds
                && (!c.text.trim().is_empty()
                    || (!c.source.trim().is_empty() && (source || !expects_translation)))
        })
        .collect();
    // The HMD clips from the top. Reserve the bottom line for the most recent
    // visible words even if they translate a speaker who started earlier.
    visible.sort_by_key(|r| r.0);
    let limit = count + usize::from(visible.iter().any(|r| r.2));
    visible
        .iter()
        .skip(visible.len().saturating_sub(limit))
        .map(|(_, c, live, _)| Row::new(c, lang, source, *live, false))
        .collect()
}

// Use the same bundled fonts, wrapping and speaker palette on both native overlays.
pub struct TextRenderer {
    fonts: Fonts,
    subpixel: bool,
}
impl Default for TextRenderer {
    fn default() -> Self {
        Self {
            fonts: Fonts::new(1., 2048, theme::fonts()),
            subpixel: false,
        }
    }
}
impl TextRenderer {
    pub fn for_headset() -> Self {
        Self {
            subpixel: true,
            ..Self::default()
        }
    }
    pub fn draw(
        &self,
        rows: &[Row],
        width: usize,
        height: usize,
        size: f32,
        pixels: &mut [Color32],
    ) {
        self.fonts.begin_pass(1., 2048);
        let pad = (size * 0.55).round().max(3.);
        let mut laid_out = Vec::new();
        for row in rows {
            let mut prefix = LayoutJob::default();
            prefix.append(&row.label, 0., row.prefix_format(size * 0.85));
            prefix.wrap.max_width = (width as f32 * 0.32).max(1.);
            prefix.wrap.max_rows = 1;
            let label = self.fonts.layout_job(prefix);
            let gutter = if row.label.is_empty() {
                0.
            } else {
                label.size().x + size * 0.4
            };
            let mut job = LayoutJob::default();
            row.append_text(&mut job, size, 0.);
            job.wrap.max_width = (width as f32 - pad * 2. - gutter).max(1.);
            laid_out.push((label, self.fonts.layout_job(job), gutter));
        }
        let mut visible = Vec::new();
        let mut used = 0.;
        let spacing = (size * 0.12).round();
        'rows: for (index, (_, body, _)) in laid_out.iter().enumerate().rev() {
            for (n, line) in body.rows.iter().enumerate().rev() {
                let h = line.rect.height();
                if !visible.is_empty() && used + h > height as f32 - pad {
                    break 'rows;
                }
                visible.push((index, n));
                used += h;
            }
            used += spacing;
        }
        let atlas = self.fonts.texture_atlas();
        let guard = atlas.lock();
        let texture = guard.image();
        let mut y = ((height as f32 - used + spacing) * 0.5).max(0.);
        let mut previous = None;
        for (index, n) in visible.into_iter().rev() {
            let (label, body, gutter) = &laid_out[index];
            if previous != Some(index) {
                if previous.is_some() {
                    y += spacing;
                }
                if let Some(line) = label.rows.first() {
                    let offset = y + (body.rows[n].rect.height() - line.rect.height()) * 0.5;
                    blit(
                        line,
                        label,
                        texture,
                        pad,
                        offset,
                        width,
                        height,
                        pixels,
                        self.subpixel,
                    );
                }
            }
            blit(
                &body.rows[n],
                body,
                texture,
                pad + gutter,
                y,
                width,
                height,
                pixels,
                self.subpixel,
            );
            y += body.rows[n].rect.height();
            previous = Some(index);
        }
    }
}
fn blit(
    row: &egui::epaint::text::Row,
    galley: &egui::Galley,
    atlas: &egui::FontImage,
    x: f32,
    y: f32,
    width: usize,
    height: usize,
    dst: &mut [Color32],
    subpixel: bool,
) {
    for g in &row.glyphs {
        let uv = g.uv_rect;
        let origin = g.pos + uv.offset;
        let pen_x = x + origin.x;
        let y0 = (y + origin.y - row.rect.top()).round() as i32;
        let format = &galley.job.sections[g.section_index as usize].format;
        let color = format.color;
        for v in uv.min[1]..uv.max[1] {
            let py = y0 + i32::from(v - uv.min[1]);
            if !(0..height as i32).contains(&py) {
                continue;
            }
            let skew = if format.italics {
                f32::from(uv.max[1] - v) * 0.25
            } else {
                0.
            };
            let (left, fraction) = if subpixel {
                let start = pen_x + skew;
                let floor = start.floor();
                (floor as i32, start - floor)
            } else {
                ((pen_x.round() + skew.round()) as i32, 0.)
            };
            let begin = v as usize * atlas.width() + usize::from(uv.min[0]);
            let mask = &atlas.pixels[begin..begin + usize::from(uv.max[0] - uv.min[0])];
            let mut previous = 0.;
            // Interpolate coverage before compositing so fractional glyph positions
            // and slanted live text keep smooth edges without dark seams.
            for (column, coverage) in mask
                .iter()
                .copied()
                .chain((fraction > 0.).then_some(0.))
                .enumerate()
            {
                let alpha = coverage + (previous - coverage) * fraction;
                previous = coverage;
                let px = left + column as i32;
                if !(0..width as i32).contains(&px) {
                    continue;
                }
                if alpha > 0. {
                    let i = py as usize * width + px as usize;
                    dst[i] = dst[i].blend(color.gamma_multiply(alpha));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_shows_above_translation_and_source_only_appears_once() {
        let mut c = Caption {
            source: "hello".into(),
            text: "안녕".into(),
            ..Default::default()
        };
        let job = Row::new(&c, Lang::Ko, true, false, false).inline(24.);
        assert_eq!(job.text, "hello\n안녕");
        assert!(job.sections[0].format.font_id.size < job.sections[1].format.font_id.size);
        assert_eq!(
            Row::new(&c, Lang::Ko, false, false, false).inline(24.).text,
            "안녕"
        );
        c.text.clear();
        assert_eq!(
            Row::new(&c, Lang::Ko, true, false, false).inline(24.).text,
            "hello"
        );
    }
    #[test]
    fn scrolls_recent_turns_with_stable_colors_and_live_tail() {
        let lane = Lane::new(true, 16000, true, Default::default(), None);
        for (who, words) in [
            ("2", "old"),
            ("1", "첫 번째"),
            ("2", "두 번째"),
            ("1", "세 번째"),
        ] {
            lane.fragment(false, words, who);
            lane.finish();
        }
        lane.fragment(false, "말하는 중", "2");
        let rows = recent(&lane, Lang::Ko, 3, 7., false);
        assert_eq!(
            rows.iter()
                .map(|r| (&*r.label, &*r.body, r.partial))
                .collect::<Vec<_>>(),
            [
                ("화자 1", "첫 번째", false),
                ("화자 2", "두 번째", false),
                ("화자 1", "세 번째", false),
                ("화자 2", "말하는 중", true)
            ]
        );
        assert_eq!(rows[0].color, Color32::from_rgb(125, 190, 246));
        assert_eq!(rows[1].color, Color32::from_rgb(238, 184, 110));
        let mut pixels = vec![Color32::TRANSPARENT; 520 * 160];
        TextRenderer::default().draw(&rows, 520, 160, 23., &mut pixels);
        for r in &rows[..2] {
            assert!(pixels.contains(&r.color));
        }
        let plain = Row::new(
            &Caption {
                text: "번역문".into(),
                ..Default::default()
            },
            Lang::Ko,
            false,
            false,
            false,
        );
        let job = plain.inline(17.);
        assert_eq!(job.text, "번역문");
        assert_eq!(job.sections.len(), 1);
        assert_eq!(job.sections[0].leading_space, 0.);
    }
}
