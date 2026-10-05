//! Lightweight live line chart drawn with cairo on a GtkDrawingArea.

use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// Series palette (distinguishable in light and dark themes).
const COLORS: [(f64, f64, f64); 6] = [(0.21, 0.52, 0.89), (0.90, 0.38, 0.0), (0.18, 0.76, 0.49), (0.75, 0.38, 0.80), (0.86, 0.20, 0.25), (0.6, 0.6, 0.6)];

struct Series {
    name: String,
    data: VecDeque<f64>,
}

struct Inner {
    series: Vec<Series>,
    capacity: usize,
    /// Fixed y max (e.g. 100 for percentages); auto-scaled when `None`.
    max: Option<f64>,
    unit: String,
}

#[derive(Clone)]
pub struct Chart {
    pub area: gtk::DrawingArea,
    inner: Rc<RefCell<Inner>>,
}

impl Chart {
    pub fn new(names: &[&str], max: Option<f64>, unit: &str, height: i32) -> Chart {
        let area = gtk::DrawingArea::new();
        area.set_content_height(height);
        area.set_hexpand(true);
        area.add_css_class("chart");
        let inner = Rc::new(RefCell::new(Inner {
            series: names.iter().map(|n| Series { name: n.to_string(), data: VecDeque::new() }).collect(),
            capacity: 120,
            max,
            unit: unit.to_string(),
        }));
        let i2 = inner.clone();
        area.set_draw_func(move |a, cr, w, h| {
            let fg = a.color();
            let (fr, fgc, fb) = (fg.red() as f64, fg.green() as f64, fg.blue() as f64);
            let inn = i2.borrow();
            let (w, h) = (w as f64, h as f64);
            let top = 16.0;
            let plot_h = h - top - 2.0;
            let maxv = inn.max.unwrap_or_else(|| {
                let m = inn.series.iter().flat_map(|s| s.data.iter().copied()).fold(0.0, f64::max);
                if m <= 0.0 { 1.0 } else { m * 1.15 }
            });
            // Grid
            cr.set_source_rgba(fr, fgc, fb, 0.12);
            cr.set_line_width(1.0);
            for k in 0..=4 {
                let y = top + plot_h * k as f64 / 4.0;
                cr.move_to(0.0, y.round() + 0.5);
                cr.line_to(w, y.round() + 0.5);
            }
            let _ = cr.stroke();
            // Lines
            for (si, s) in inn.series.iter().enumerate() {
                if s.data.len() < 2 {
                    continue;
                }
                let (r, g, b) = COLORS[si % COLORS.len()];
                cr.set_source_rgb(r, g, b);
                cr.set_line_width(1.8);
                let step = w / (inn.capacity.max(2) - 1) as f64;
                let off = inn.capacity - s.data.len();
                for (i, v) in s.data.iter().enumerate() {
                    let x = (off + i) as f64 * step;
                    let y = top + plot_h - (v / maxv).clamp(0.0, 1.0) * plot_h;
                    if i == 0 {
                        cr.move_to(x, y);
                    } else {
                        cr.line_to(x, y);
                    }
                }
                let _ = cr.stroke();
            }
            // Legend with latest values (text, not color only).
            cr.select_font_face("Sans", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
            cr.set_font_size(10.0);
            let mut x = 2.0;
            for (si, s) in inn.series.iter().enumerate() {
                let (r, g, b) = COLORS[si % COLORS.len()];
                cr.set_source_rgb(r, g, b);
                cr.rectangle(x, 4.0, 8.0, 8.0);
                let _ = cr.fill();
                cr.set_source_rgba(fr, fgc, fb, 0.85);
                let label = match s.data.back() {
                    Some(v) => format!("{} {:.1}{}", s.name, v, inn.unit),
                    None => format!("{} —", s.name),
                };
                cr.move_to(x + 11.0, 12.0);
                let _ = cr.show_text(&label);
                x += 11.0 + cr.text_extents(&label).map(|e| e.x_advance()).unwrap_or(60.0) + 12.0;
            }
            let scale = format!("max {:.0}{}", maxv, inn.unit);
            let tw = cr.text_extents(&scale).map(|e| e.width()).unwrap_or(40.0);
            cr.set_source_rgba(fr, fgc, fb, 0.5);
            cr.move_to(w - tw - 4.0, 12.0);
            let _ = cr.show_text(&scale);
        });
        Chart { area, inner }
    }

    /// Append one value per series (`None` leaves a series unchanged).
    pub fn push(&self, values: &[Option<f64>]) {
        {
            let mut inn = self.inner.borrow_mut();
            let cap = inn.capacity;
            for (s, v) in inn.series.iter_mut().zip(values) {
                if let Some(v) = v {
                    s.data.push_back(*v);
                    while s.data.len() > cap {
                        s.data.pop_front();
                    }
                }
            }
        }
        if self.area.is_mapped() {
            self.area.queue_draw();
        }
    }

    pub fn reset(&self) {
        for s in self.inner.borrow_mut().series.iter_mut() {
            s.data.clear();
        }
        self.area.queue_draw();
    }

    pub fn set_accessible_summary(&self, text: &str) {
        self.area.update_property(&[gtk::accessible::Property::Label(text)]);
    }
}
