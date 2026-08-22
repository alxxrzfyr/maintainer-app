//! Sparkline drawing widget.
//!
//! Renders a bounded ring buffer of samples as a simple area chart.
//! Only redraws when new data arrives. Exposes the current value as
//! accessible text for screen readers.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use gtk4::DrawingArea;
use gtk4::prelude::*;

/// A sparkline widget that renders samples as an area chart.
pub struct Sparkline {
    drawing_area: DrawingArea,
    data: Rc<RefCell<SparklineData>>,
}

struct SparklineData {
    samples: VecDeque<f64>,
    min_value: f64,
    max_value: f64,
    accessible_text: String,
}

impl Sparkline {
    /// Create a new sparkline widget with the given value range.
    pub fn new(min_value: f64, max_value: f64) -> Self {
        let drawing_area = DrawingArea::new();
        drawing_area.set_content_height(56);
        drawing_area.set_content_width(200);
        drawing_area.add_css_class("maintainer_sparkline");

        let data = Rc::new(RefCell::new(SparklineData {
            samples: VecDeque::new(),
            min_value,
            max_value,
            accessible_text: "No data".into(),
        }));

        let data_for_draw = data.clone();
        drawing_area.set_draw_func(move |_area, cr, width, height| {
            let d = data_for_draw.borrow();
            Self::draw(cr, width as f64, height as f64, &d);
        });

        drawing_area.update_property(&[gtk4::accessible::Property::Label("Sparkline chart")]);

        Self { drawing_area, data }
    }

    /// Push new samples from a ring buffer, triggering a redraw.
    pub fn update_from_ring(&self, samples: &VecDeque<f64>, accessible_text: &str) {
        {
            let mut d = self.data.borrow_mut();
            d.samples = samples.clone();
            d.accessible_text = accessible_text.to_string();
        }
        self.drawing_area
            .update_property(&[gtk4::accessible::Property::Label(
                &self.data.borrow().accessible_text,
            )]);
        self.drawing_area.queue_draw();
    }

    /// Get the underlying GTK widget.
    pub fn widget(&self) -> &DrawingArea {
        &self.drawing_area
    }

    fn draw(cr: &gtk4::cairo::Context, width: f64, height: f64, data: &SparklineData) {
        let samples = &data.samples;
        if samples.is_empty() {
            return;
        }

        let range = data.max_value - data.min_value;
        if range <= 0.0 {
            return;
        }

        let n = samples.len();
        let step = if n > 1 { width / (n - 1) as f64 } else { width };

        let normalize = |v: f64| -> f64 {
            let clamped = v.clamp(data.min_value, data.max_value);
            height - ((clamped - data.min_value) / range) * height
        };

        cr.move_to(0.0, height);
        for (i, &val) in samples.iter().enumerate() {
            let x = i as f64 * step;
            let y = normalize(val);
            cr.line_to(x, y);
        }
        cr.line_to((n - 1) as f64 * step, height);
        cr.close_path();

        cr.set_source_rgba(0.14, 0.37, 0.84, 0.15);
        let _ = cr.fill_preserve();
        cr.new_path();

        for (i, &val) in samples.iter().enumerate() {
            let x = i as f64 * step;
            let y = normalize(val);
            if i == 0 {
                cr.move_to(x, y);
            } else {
                cr.line_to(x, y);
            }
        }
        cr.set_source_rgba(0.14, 0.37, 0.84, 0.7);
        cr.set_line_width(2.0);
        let _ = cr.stroke();
    }
}
