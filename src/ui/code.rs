//! Painting of code lines: syntax colours + word-level diff emphasis.

use std::ops::Range;

use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, FontId, Painter, Rect, Stroke, pos2};

use crate::highlight::{Hl, Span};
use crate::text::TAB_WIDTH;
use crate::theme::Palette;

/// Font metrics for the code views.
#[derive(Clone)]
pub struct CodeStyle {
    pub font: FontId,
    pub small: FontId,
    pub row_h: f32,
    pub char_w: f32,
    pub pal: &'static Palette,
}

impl CodeStyle {
    pub fn new(ctx: &egui::Context, font_size: f32) -> Self {
        let font = FontId::monospace(font_size);
        let small = FontId::monospace((font_size - 1.5).max(8.0));
        let (row, char_w) = ctx.fonts_mut(|f| (f.row_height(&font), f.glyph_width(&font, 'M')));
        Self {
            font,
            small,
            row_h: (row * 1.4).round(),
            char_w,
            pal: crate::theme::palette(ctx),
        }
    }

    pub fn gutter_width(&self, max_line: usize) -> f32 {
        let digits = max_line.max(1).to_string().len().max(2);
        digits as f32 * self.char_w + 18.0
    }
}

fn push(job: &mut LayoutJob, text: &str, format: TextFormat) {
    if text.contains('\t') {
        job.append(&text.replace('\t', &" ".repeat(TAB_WIDTH)), 0.0, format);
    } else {
        job.append(text, 0.0, format);
    }
}

/// Build a single-line layout job from syntax spans and emphasis ranges.
pub fn line_job(
    text: &str,
    spans: &[Span],
    emph: &[Range<usize>],
    emph_bg: Color32,
    st: &CodeStyle,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let plain = st.pal.syntax(Hl::Plain);
    if spans.is_empty() && emph.is_empty() {
        push(&mut job, text, TextFormat::simple(st.font.clone(), plain));
        return job;
    }

    let mut cuts: Vec<usize> = Vec::with_capacity(2 + 2 * (spans.len() + emph.len()));
    cuts.push(0);
    cuts.push(text.len());
    for (r, _) in spans {
        cuts.extend([r.start, r.end]);
    }
    for r in emph {
        cuts.extend([r.start, r.end]);
    }
    cuts.retain(|&c| c <= text.len() && text.is_char_boundary(c));
    cuts.sort_unstable();
    cuts.dedup();

    let (mut si, mut ei) = (0, 0);
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        while si < spans.len() && spans[si].0.end <= a {
            si += 1;
        }
        while ei < emph.len() && emph[ei].end <= a {
            ei += 1;
        }
        let color = spans
            .get(si)
            .filter(|(r, _)| r.start <= a)
            .map_or(plain, |(_, hl)| st.pal.syntax(*hl));
        let emphasised = emph.get(ei).is_some_and(|r| r.start <= a);
        let mut format = TextFormat::simple(st.font.clone(), color);
        if emphasised {
            format.background = emph_bg;
        }
        if spans.get(si).is_some_and(|(r, hl)| r.start <= a && *hl == Hl::Comment) {
            format.italics = true;
        }
        push(&mut job, &text[a..b], format);
    }
    job
}

/// Paint a laid-out line into `rect`, clipped, scrolled horizontally by `h_off`.
pub fn paint_job(painter: &Painter, rect: Rect, job: LayoutJob, h_off: f32, st: &CodeStyle) {
    let galley = painter.layout_job(job);
    let y = rect.top() + ((st.row_h - galley.size().y) * 0.5).round();
    painter
        .with_clip_rect(rect.intersect(painter.clip_rect()))
        .galley(pos2(rect.left() - h_off, y), galley, st.pal.text);
}

pub fn paint_line_number(painter: &Painter, rect: Rect, n: Option<usize>, color: Color32, st: &CodeStyle) {
    if let Some(n) = n {
        painter.text(
            pos2(rect.right() - 8.0, rect.center().y),
            Align2::RIGHT_CENTER,
            n.to_string(),
            st.small.clone(),
            color,
        );
    }
}

/// Diagonal hatching used for "no line here" filler cells.
pub fn paint_filler(painter: &Painter, rect: Rect, st: &CodeStyle) {
    painter.rect_filled(rect, 0.0, st.pal.filler);
    let p = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let color = if st.pal.dark {
        Color32::from_white_alpha(6)
    } else {
        Color32::from_black_alpha(10)
    };
    let step = 7.0;
    // Anchor the pattern to absolute coordinates so it does not "swim" while scrolling.
    // Stripes satisfy x + y ≡ 0 (mod step).
    let start = ((rect.left() + rect.bottom()) / step).floor() * step - rect.bottom();
    let mut x = start - rect.height();
    while x < rect.right() {
        p.line_segment(
            [pos2(x, rect.bottom()), pos2(x + rect.height(), rect.top())],
            Stroke::new(1.0, color),
        );
        x += step;
    }
}
