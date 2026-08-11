use vrecnu_types::{BubbleSpec, Rect};

/// Resolved pixel geometry for one bubble frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BubbleLayout {
    /// Window rectangle in screen coordinates.
    pub window: Rect,
    /// Diameter of the visible circle, inside the shadow margin.
    pub circle_diameter: u32,
    /// Margin reserved around the circle for the drop shadow.
    pub shadow_margin: u32,
}

impl BubbleLayout {
    /// Fraction of the diameter reserved for the shadow on each side.
    const SHADOW_FRACTION: f32 = 0.08;

    /// Computes layout for a spec.
    ///
    /// The window is larger than the circle because `UpdateLayeredWindow` composites the
    /// shadow as part of the same surface — there is no separate shadow layer to grow into.
    #[must_use]
    pub fn from_spec(spec: BubbleSpec) -> Self {
        let shadow_margin = ((spec.diameter as f32) * Self::SHADOW_FRACTION).round() as u32;
        let side = spec.diameter + shadow_margin * 2;
        Self {
            window: Rect {
                x: spec.position.0 - shadow_margin as i32,
                y: spec.position.1 - shadow_margin as i32,
                width: side,
                height: side,
            },
            circle_diameter: spec.diameter,
            shadow_margin,
        }
    }

    /// Bytes for one premultiplied-BGRA surface of this size.
    #[must_use]
    pub const fn surface_bytes(&self) -> usize {
        (self.window.width as usize) * (self.window.height as usize) * 4
    }
}

/// Coverage of a pixel by the bubble's circle, 0.0 outside to 1.0 inside.
///
/// Sampled rather than thresholded so the circle's edge and ring are anti-aliased. A hard
/// threshold produces visibly stepped edges at 240 px, which is exactly the size the bubble
/// is normally drawn at.
#[must_use]
pub fn ring_coverage(x: f32, y: f32, centre: f32, radius: f32, samples: u32) -> f32 {
    if samples == 0 {
        return 0.0;
    }
    let mut hits = 0u32;
    let step = 1.0 / samples as f32;
    for sy in 0..samples {
        for sx in 0..samples {
            let px = x + (sx as f32 + 0.5) * step;
            let py = y + (sy as f32 + 0.5) * step;
            let dx = px - centre;
            let dy = py - centre;
            if dx * dx + dy * dy <= radius * radius {
                hits += 1;
            }
        }
    }
    hits as f32 / (samples * samples) as f32
}

/// Premultiplies one colour channel by its alpha.
///
/// `UpdateLayeredWindow` requires premultiplied BGRA. Handing it straight alpha draws a dark
/// halo around the circle, which reads as a rendering bug rather than a format error — so the
/// conversion has to happen before present, every time.
#[must_use]
pub const fn alpha_premultiply(channel: u8, alpha: u8) -> u8 {
    ((channel as u16 * alpha as u16) / 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_is_larger_than_the_circle_to_hold_the_shadow() {
        let layout = BubbleLayout::from_spec(BubbleSpec {
            diameter: 240,
            position: (100, 100),
            click_through: true,
        });
        assert_eq!(layout.shadow_margin, 19);
        assert_eq!(layout.window.width, 240 + 38);
        assert_eq!(layout.circle_diameter, 240);
    }

    #[test]
    fn the_window_is_offset_so_the_circle_lands_where_asked() {
        // The user positions the visible circle, not the invisible shadow box.
        let layout = BubbleLayout::from_spec(BubbleSpec {
            diameter: 240,
            position: (100, 100),
            click_through: true,
        });
        assert_eq!(layout.window.x, 100 - 19);
        assert_eq!(layout.window.y, 100 - 19);
    }

    #[test]
    fn a_default_bubble_surface_is_small_enough_to_present_per_frame() {
        let layout = BubbleLayout::from_spec(BubbleSpec::default());
        // ~309 KB per present at 30 fps is a negligible GDI blit.
        assert!(
            layout.surface_bytes() < 400_000,
            "{}",
            layout.surface_bytes()
        );
    }

    #[test]
    fn the_circle_centre_is_fully_covered() {
        assert!((ring_coverage(50.0, 50.0, 50.5, 20.0, 4) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_pixel_far_outside_has_no_coverage() {
        assert_eq!(ring_coverage(0.0, 0.0, 50.0, 20.0, 4), 0.0);
    }

    #[test]
    fn an_edge_pixel_is_partially_covered_so_the_edge_is_smooth() {
        // The reason for supersampling: a hard threshold gives visible stair-stepping.
        //
        // The pixel has to genuinely straddle the boundary. On the horizontal axis it does
        // not: with centre 50 and radius 20 the edge lands exactly on x = 70, a pixel
        // boundary, so [69,70) is wholly inside and [70,71) wholly outside. The 45-degree
        // point at 50 + 20/sqrt(2) ~= 64.14 falls mid-pixel, so (64, 64) is a real edge case.
        let coverage = ring_coverage(64.0, 64.0, 50.0, 20.0, 8);
        assert!(coverage > 0.0 && coverage < 1.0, "coverage was {coverage}");
    }

    #[test]
    fn coverage_falls_off_monotonically_across_the_boundary() {
        // Pins the direction of the gradient, so an inverted comparison in `ring_coverage`
        // could not pass by accident.
        let inside = ring_coverage(60.0, 60.0, 50.0, 20.0, 8);
        let edge = ring_coverage(64.0, 64.0, 50.0, 20.0, 8);
        let outside = ring_coverage(68.0, 68.0, 50.0, 20.0, 8);
        assert!(inside > edge, "inside {inside} should exceed edge {edge}");
        assert!(
            edge > outside,
            "edge {edge} should exceed outside {outside}"
        );
    }

    #[test]
    fn premultiplying_by_full_alpha_is_a_no_op() {
        assert_eq!(alpha_premultiply(200, 255), 200);
    }

    #[test]
    fn premultiplying_by_zero_alpha_clears_the_channel() {
        assert_eq!(alpha_premultiply(200, 0), 0);
    }

    #[test]
    fn premultiplying_by_half_alpha_halves_the_channel() {
        assert_eq!(alpha_premultiply(200, 128), 100);
    }
}
