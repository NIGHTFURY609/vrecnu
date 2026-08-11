//! Software rendering for the bubble surface.
//!
//! Deliberately hand-rolled rather than pulled from a 2D library. The whole drawing is a
//! shadow, a circle, a blit and a ring; a rasteriser dependency would cost more binary than
//! the code it replaced, and AGENTS.md §3 says to prefer a small utility over a framework.
//!
//! Output is **premultiplied BGRA**, which is what `UpdateLayeredWindow` requires. Straight
//! alpha renders a dark fringe around the circle that looks like a rendering bug.

use crate::geometry::{alpha_premultiply, ring_coverage, BubbleLayout};

/// Colour of the ring drawn around the bubble, as straight BGRA.
pub const RING_COLOR: [u8; 4] = [0xF8, 0xF8, 0xF8, 0xFF];
/// How thick the ring is, as a fraction of the circle's radius.
const RING_FRACTION: f32 = 0.035;
/// Peak opacity of the drop shadow.
const SHADOW_ALPHA: f32 = 0.35;
/// Supersampling rate for the circle edge. 4x4 is enough to hide stair-stepping at 240 px.
const EDGE_SAMPLES: u32 = 4;

/// A camera frame scaled and centre-cropped to the bubble's square.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SquareFrame {
    /// Side length in pixels.
    pub side: u32,
    /// Tightly packed BGRA8, `side * side * 4` bytes.
    pub bgra: Vec<u8>,
}

impl SquareFrame {
    /// A solid-colour square, used before the first camera frame arrives.
    #[must_use]
    pub fn solid(side: u32, bgra: [u8; 4]) -> Self {
        let mut buffer = Vec::with_capacity((side * side * 4) as usize);
        for _ in 0..(side * side) {
            buffer.extend_from_slice(&bgra);
        }
        Self { side, bgra: buffer }
    }

    /// Reads one pixel, or opaque black when out of bounds.
    #[must_use]
    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.side || y >= self.side {
            return [0, 0, 0, 255];
        }
        let index = ((y * self.side + x) * 4) as usize;
        match self.bgra.get(index..index + 4) {
            Some(slice) => [slice[0], slice[1], slice[2], slice[3]],
            None => [0, 0, 0, 255],
        }
    }
}

/// Renders one complete bubble surface.
///
/// Returns premultiplied BGRA sized to `layout.window`, ready to hand to
/// `UpdateLayeredWindow` with no further conversion.
#[must_use]
pub fn render_bubble(layout: &BubbleLayout, frame: &SquareFrame) -> Vec<u8> {
    let width = layout.window.width;
    let height = layout.window.height;
    let mut out = vec![0u8; (width as usize) * (height as usize) * 4];

    let centre = width as f32 / 2.0;
    let radius = layout.circle_diameter as f32 / 2.0;
    let ring_width = (radius * RING_FRACTION).max(1.0);
    let inner_radius = radius - ring_width;
    let shadow_radius = radius + layout.shadow_margin as f32 * 0.75;

    for y in 0..height {
        for x in 0..width {
            let index = ((y * width + x) * 4) as usize;

            let coverage = ring_coverage(x as f32, y as f32, centre, radius, EDGE_SAMPLES);
            let inner = ring_coverage(x as f32, y as f32, centre, inner_radius, EDGE_SAMPLES);

            let (b, g, r, a) = if inner > 0.0 {
                // Inside the camera area. The ring feathers over the outer edge of it.
                let sample_x = ((x as f32 - (centre - radius)) as i64).max(0) as u32;
                let sample_y = ((y as f32 - (centre - radius)) as i64).max(0) as u32;
                let px = frame.pixel(
                    sample_x.min(frame.side.saturating_sub(1)),
                    sample_y.min(frame.side.saturating_sub(1)),
                );
                let ring_mix = 1.0 - inner;
                (
                    blend(px[0], RING_COLOR[0], ring_mix),
                    blend(px[1], RING_COLOR[1], ring_mix),
                    blend(px[2], RING_COLOR[2], ring_mix),
                    coverage,
                )
            } else if coverage > 0.0 {
                (RING_COLOR[0], RING_COLOR[1], RING_COLOR[2], coverage)
            } else {
                // Outside the circle: a soft shadow that fades to nothing at the window edge.
                let distance = ((x as f32 - centre).powi(2) + (y as f32 - centre).powi(2)).sqrt();
                let falloff =
                    ((shadow_radius - distance) / (shadow_radius - radius)).clamp(0.0, 1.0);
                (0, 0, 0, falloff * SHADOW_ALPHA)
            };

            let alpha = (a * 255.0).clamp(0.0, 255.0) as u8;
            // Premultiply here, once, rather than trusting every call site to remember.
            out[index] = alpha_premultiply(b, alpha);
            out[index + 1] = alpha_premultiply(g, alpha);
            out[index + 2] = alpha_premultiply(r, alpha);
            out[index + 3] = alpha;
        }
    }

    out
}

fn blend(from: u8, to: u8, amount: f32) -> u8 {
    let amount = amount.clamp(0.0, 1.0);
    (f32::from(from) * (1.0 - amount) + f32::from(to) * amount).round() as u8
}

/// Centre-crops and nearest-neighbour scales a camera frame to a square.
///
/// Nearest-neighbour is a deliberate choice: at bubble sizes the source is being *downscaled*
/// from a much larger sensor, the result is masked to a circle, and it is redrawn 30 times a
/// second. A better filter would cost CPU during recording, which is the one moment
/// AGENTS.md §1 has no headroom to spare.
#[must_use]
pub fn to_square(src: &[u8], src_width: u32, src_height: u32, side: u32) -> SquareFrame {
    let (crop_x, crop_y, crop_side) = crate::square_crop(src_width, src_height);
    let mut bgra = vec![0u8; (side as usize) * (side as usize) * 4];

    for y in 0..side {
        let src_y = crop_y + (y * crop_side) / side.max(1);
        for x in 0..side {
            let src_x = crop_x + (x * crop_side) / side.max(1);
            let src_index = ((src_y * src_width + src_x) * 4) as usize;
            let dst_index = ((y * side + x) * 4) as usize;
            if let Some(pixel) = src.get(src_index..src_index + 4) {
                bgra[dst_index..dst_index + 4].copy_from_slice(pixel);
            }
        }
    }

    SquareFrame { side, bgra }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vrecnu_types::BubbleSpec;

    fn layout() -> BubbleLayout {
        BubbleLayout::from_spec(BubbleSpec {
            diameter: 64,
            position: (0, 0),
            click_through: true,
        })
    }

    #[test]
    fn the_surface_is_exactly_the_window_size() {
        let layout = layout();
        let surface = render_bubble(&layout, &SquareFrame::solid(64, [10, 20, 30, 255]));
        assert_eq!(surface.len(), layout.surface_bytes());
    }

    #[test]
    fn the_window_corner_is_fully_transparent() {
        // The corner is outside both the circle and the shadow falloff. If it were opaque the
        // bubble would render as a square box, which is the classic layered-window bug.
        let layout = layout();
        let surface = render_bubble(&layout, &SquareFrame::solid(64, [10, 20, 30, 255]));
        assert_eq!(surface[3], 0, "top-left corner alpha must be 0");
    }

    #[test]
    fn the_centre_is_fully_opaque() {
        let layout = layout();
        let surface = render_bubble(&layout, &SquareFrame::solid(64, [10, 20, 30, 255]));
        let width = layout.window.width;
        let centre = ((width / 2) * width + width / 2) as usize * 4;
        assert_eq!(surface[centre + 3], 255, "centre alpha must be 255");
    }

    #[test]
    fn output_is_premultiplied() {
        // Every channel must be <= alpha. A channel brighter than its alpha is straight-alpha
        // data, which UpdateLayeredWindow renders with a dark halo.
        let layout = layout();
        let surface = render_bubble(&layout, &SquareFrame::solid(64, [255, 255, 255, 255]));
        for chunk in surface.chunks_exact(4) {
            let alpha = chunk[3];
            for channel in &chunk[..3] {
                assert!(
                    *channel <= alpha,
                    "channel {channel} exceeds alpha {alpha}: not premultiplied"
                );
            }
        }
    }

    #[test]
    fn the_camera_image_reaches_the_centre_of_the_bubble() {
        let layout = layout();
        // Distinct blue so it cannot be confused with the ring (near-white) or shadow (black).
        let surface = render_bubble(&layout, &SquareFrame::solid(64, [200, 30, 30, 255]));
        let width = layout.window.width;
        let centre = ((width / 2) * width + width / 2) as usize * 4;
        assert_eq!(surface[centre], 200, "blue channel");
        assert_eq!(surface[centre + 1], 30, "green channel");
    }

    #[test]
    fn to_square_centre_crops_a_landscape_frame() {
        // 4x2 source, two columns cropped away, scaled to 2x2.
        let src: Vec<u8> = (0..(4 * 2 * 4)).map(|i| (i % 256) as u8).collect();
        let square = to_square(&src, 4, 2, 2);
        assert_eq!(square.side, 2);
        assert_eq!(square.bgra.len(), 2 * 2 * 4);
    }

    #[test]
    fn to_square_handles_a_truncated_source_without_panicking() {
        // A short buffer from a mid-frame device disconnect must not take the app down.
        let src = vec![0u8; 8];
        let square = to_square(&src, 64, 64, 16);
        assert_eq!(square.bgra.len(), 16 * 16 * 4);
    }

    #[test]
    fn solid_frames_are_the_requested_size() {
        let frame = SquareFrame::solid(8, [1, 2, 3, 4]);
        assert_eq!(frame.bgra.len(), 8 * 8 * 4);
        assert_eq!(frame.pixel(0, 0), [1, 2, 3, 4]);
    }

    #[test]
    fn reading_outside_a_frame_returns_black_rather_than_panicking() {
        let frame = SquareFrame::solid(4, [9, 9, 9, 255]);
        assert_eq!(frame.pixel(99, 99), [0, 0, 0, 255]);
    }
}
