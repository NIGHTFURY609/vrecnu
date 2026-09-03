//! Software rendering for the bubble surface.
//!
//! Deliberately hand-rolled rather than pulled from a 2D library. The whole drawing is a
//! shadow, a circle, a blit and a ring; a rasteriser dependency would cost more binary than
//! the code it replaced, and AGENTS.md §3 says to prefer a small utility over a framework.
//!
//! Two consumers, two alpha conventions, one rasterisation. `UpdateLayeredWindow` requires
//! **premultiplied** BGRA — straight alpha renders a dark fringe around the circle that looks
//! like a rendering bug. FFmpeg's `-pixel_format bgra` input requires **straight** alpha, and
//! feeding it premultiplied data burns that same dark fringe into the recording, where —
//! unlike on screen — it cannot be undone afterwards. So [`AlphaMode`] is an explicit argument
//! at every entry point rather than a convention someone has to remember.
//!
//! The circle's edge is supersampled, which is not cheap: at the default 240 px bubble that is
//! ~77 000 pixels x 2 coverage tests x 16 samples, and it would run twice per camera frame
//! (once for the window, once for the overlay pipe) during exactly the window AGENTS.md §1 has
//! no CPU headroom to spare. But the expensive part depends only on the *geometry*, which does
//! not change while a bubble is open. [`BubbleMask`] precomputes it once; compositing a frame
//! through the mask is then a per-pixel blend and nothing more.

use crate::geometry::{alpha_premultiply, ring_coverage, BubbleLayout};

/// Colour of the ring drawn around the bubble, as straight BGRA.
pub const RING_COLOR: [u8; 4] = [0xF8, 0xF8, 0xF8, 0xFF];
/// How thick the ring is, as a fraction of the circle's radius.
const RING_FRACTION: f32 = 0.035;
/// Peak opacity of the drop shadow.
const SHADOW_ALPHA: f32 = 0.35;
/// Supersampling rate for the circle edge. 4x4 is enough to hide stair-stepping at 240 px.
const EDGE_SAMPLES: u32 = 4;

/// Which alpha convention a composited surface carries.
///
/// Not a default and not inferred: the two consumers of this module need opposite answers, and
/// picking the wrong one is invisible in code review and obvious on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaMode {
    /// Colour channels premultiplied by alpha — what `UpdateLayeredWindow` requires.
    Premultiplied,
    /// Colour channels independent of alpha — what FFmpeg's `bgra` rawvideo input requires.
    Straight,
}

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

/// What one surface pixel is made of, once the geometry is resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MaskKind {
    /// Inside the camera area. `mix` is how much ring colour feathers over it, 0-255.
    Camera { mix: u8 },
    /// On the ring itself.
    Ring,
    /// Outside the circle: the drop shadow, or nothing.
    Shadow,
}

/// One resolved surface pixel. Four bytes, so the default bubble's mask is ~309 KB — paid once
/// per open bubble rather than sixty times a second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MaskPixel {
    alpha: u8,
    kind: MaskKind,
}

/// The bubble's geometry, rasterised once and reused for every frame.
///
/// Build one when the bubble opens and keep it for as long as the layout is unchanged. See the
/// module doc comment for why this exists rather than rasterising per frame.
#[derive(Debug, Clone)]
pub struct BubbleMask {
    width: u32,
    height: u32,
    /// Where the camera square's top-left corner lands in surface coordinates.
    camera_origin: u32,
    pixels: Vec<MaskPixel>,
}

impl BubbleMask {
    /// Rasterises `layout`.
    #[must_use]
    pub fn new(layout: &BubbleLayout) -> Self {
        let width = layout.window.width;
        let height = layout.window.height;

        let centre = width as f32 / 2.0;
        let radius = layout.circle_diameter as f32 / 2.0;
        let ring_width = (radius * RING_FRACTION).max(1.0);
        let inner_radius = radius - ring_width;
        let shadow_radius = radius + layout.shadow_margin as f32 * 0.75;

        let mut pixels = Vec::with_capacity((width as usize) * (height as usize));
        for y in 0..height {
            for x in 0..width {
                let coverage = ring_coverage(x as f32, y as f32, centre, radius, EDGE_SAMPLES);
                let inner = ring_coverage(x as f32, y as f32, centre, inner_radius, EDGE_SAMPLES);

                let pixel = if inner > 0.0 {
                    MaskPixel {
                        alpha: to_u8(coverage),
                        kind: MaskKind::Camera {
                            mix: to_u8(1.0 - inner),
                        },
                    }
                } else if coverage > 0.0 {
                    MaskPixel {
                        alpha: to_u8(coverage),
                        kind: MaskKind::Ring,
                    }
                } else {
                    // Outside the circle: a soft shadow that fades to nothing at the window edge.
                    let distance =
                        ((x as f32 - centre).powi(2) + (y as f32 - centre).powi(2)).sqrt();
                    let falloff =
                        ((shadow_radius - distance) / (shadow_radius - radius)).clamp(0.0, 1.0);
                    MaskPixel {
                        alpha: to_u8(falloff * SHADOW_ALPHA),
                        kind: MaskKind::Shadow,
                    }
                };
                pixels.push(pixel);
            }
        }

        Self {
            width,
            height,
            camera_origin: layout.shadow_margin,
            pixels,
        }
    }

    /// Bytes in one composited surface.
    #[must_use]
    pub fn surface_len(&self) -> usize {
        (self.width as usize) * (self.height as usize) * 4
    }

    /// The camera square side this mask expects — pass it to [`to_square`].
    #[must_use]
    pub const fn camera_side(&self) -> u32 {
        self.width - self.camera_origin * 2
    }

    /// Composites `frame` through the mask into a fresh buffer.
    #[must_use]
    pub fn composite(&self, frame: &SquareFrame, alpha: AlphaMode) -> Vec<u8> {
        let mut out = vec![0u8; self.surface_len()];
        self.composite_into(frame, alpha, &mut out);
        out
    }

    /// Composites `frame` through the mask into an existing buffer.
    ///
    /// Takes the destination rather than returning one so a per-frame writer can keep a single
    /// surface for the life of a recording — the same "allocate once, never grow" rule the
    /// capture frame pool follows. A short `out` is filled as far as it goes rather than
    /// panicking: `panic = "abort"` means a panic on the camera thread ends the recording.
    pub fn composite_into(&self, frame: &SquareFrame, alpha: AlphaMode, out: &mut [u8]) {
        let origin = self.camera_origin;
        let last = frame.side.saturating_sub(1);

        for y in 0..self.height {
            for x in 0..self.width {
                let index = ((y * self.width + x) * 4) as usize;
                let Some(slot) = out.get_mut(index..index + 4) else {
                    return;
                };
                let mask = self.pixels[(y * self.width + x) as usize];

                let (b, g, r) = match mask.kind {
                    MaskKind::Camera { mix } => {
                        let px = frame.pixel(
                            x.saturating_sub(origin).min(last),
                            y.saturating_sub(origin).min(last),
                        );
                        (
                            blend(px[0], RING_COLOR[0], mix),
                            blend(px[1], RING_COLOR[1], mix),
                            blend(px[2], RING_COLOR[2], mix),
                        )
                    }
                    MaskKind::Ring => (RING_COLOR[0], RING_COLOR[1], RING_COLOR[2]),
                    MaskKind::Shadow => (0, 0, 0),
                };

                match alpha {
                    AlphaMode::Premultiplied => {
                        slot[0] = alpha_premultiply(b, mask.alpha);
                        slot[1] = alpha_premultiply(g, mask.alpha);
                        slot[2] = alpha_premultiply(r, mask.alpha);
                    }
                    AlphaMode::Straight => {
                        slot[0] = b;
                        slot[1] = g;
                        slot[2] = r;
                    }
                }
                slot[3] = mask.alpha;
            }
        }
    }
}

fn to_u8(value: f32) -> u8 {
    (value * 255.0).clamp(0.0, 255.0) as u8
}

/// Renders one complete bubble surface.
///
/// Convenience over [`BubbleMask`] for a one-off render (a test, a still preview). A caller
/// drawing every frame should build the mask once and call [`BubbleMask::composite_into`]
/// instead — see the module doc comment.
#[must_use]
pub fn render_bubble(layout: &BubbleLayout, frame: &SquareFrame) -> Vec<u8> {
    BubbleMask::new(layout).composite(frame, AlphaMode::Premultiplied)
}

/// Converts a straight-alpha BGRA surface into a premultiplied one.
///
/// The camera pipeline composites **once**, in straight alpha, because that is what FFmpeg's
/// `bgra` input needs; the bubble window needs the same picture premultiplied. Deriving the
/// second from the first is exact and costs one multiply per channel, where compositing twice
/// would cost a second full pass through [`BubbleMask`] thirty times a second.
///
/// A short `out` is filled as far as it goes rather than panicking, for the same reason
/// [`BubbleMask::composite_into`] does: `panic = "abort"` means a panic on the camera thread
/// takes the recording with it.
pub fn premultiply_into(straight: &[u8], out: &mut [u8]) {
    for (src, dst) in straight.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
        let alpha = src[3];
        dst[0] = alpha_premultiply(src[0], alpha);
        dst[1] = alpha_premultiply(src[1], alpha);
        dst[2] = alpha_premultiply(src[2], alpha);
        dst[3] = alpha;
    }
}

fn blend(from: u8, to: u8, amount: u8) -> u8 {
    let amount = u16::from(amount);
    ((u16::from(from) * (255 - amount) + u16::from(to) * amount) / 255) as u8
}

/// Centre-crops and nearest-neighbour scales a camera frame to a square.
///
/// Nearest-neighbour is a deliberate choice: at bubble sizes the source is being *downscaled*
/// from a much larger sensor, the result is masked to a circle, and it is redrawn 30 times a
/// second. A better filter would cost CPU during recording, which is the one moment
/// AGENTS.md §1 has no headroom to spare.
#[must_use]
pub fn to_square(src: &[u8], src_width: u32, src_height: u32, side: u32) -> SquareFrame {
    let mut frame = SquareFrame {
        side,
        bgra: vec![0u8; (side as usize) * (side as usize) * 4],
    };
    to_square_into(src, src_width, src_height, &mut frame);
    frame
}

/// [`to_square`] into an existing [`SquareFrame`], so a per-frame caller allocates nothing.
///
/// `dst.side` is honoured as-is; the buffer is resized only when it does not already match,
/// which on the steady-state path it always does.
pub fn to_square_into(src: &[u8], src_width: u32, src_height: u32, dst: &mut SquareFrame) {
    let side = dst.side;
    let needed = (side as usize) * (side as usize) * 4;
    if dst.bgra.len() != needed {
        dst.bgra.resize(needed, 0);
    }

    let (crop_x, crop_y, crop_side) = crate::square_crop(src_width, src_height);

    for y in 0..side {
        let src_y = crop_y + (y * crop_side) / side.max(1);
        for x in 0..side {
            let src_x = crop_x + (x * crop_side) / side.max(1);
            let src_index = ((src_y * src_width + src_x) * 4) as usize;
            let dst_index = ((y * side + x) * 4) as usize;
            if let Some(pixel) = src.get(src_index..src_index + 4) {
                dst.bgra[dst_index..dst_index + 4].copy_from_slice(pixel);
            }
        }
    }
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
    fn straight_alpha_keeps_the_edge_brighter_than_premultiplied_does() {
        // The whole reason AlphaMode exists. FFmpeg's `bgra` input is straight alpha; handing
        // it premultiplied data burns a dark fringe around the bubble into the recording,
        // where — unlike on screen — it cannot be undone afterwards.
        let layout = layout();
        let mask = BubbleMask::new(&layout);
        let frame = SquareFrame::solid(64, [255, 255, 255, 255]);
        let straight = mask.composite(&frame, AlphaMode::Straight);
        let premultiplied = mask.composite(&frame, AlphaMode::Premultiplied);

        // `s[0] > 0` is load-bearing, not belt-and-braces: the drop shadow is also partially
        // transparent, and it is pure black, so premultiplying it changes nothing. Scanning
        // for partial alpha alone finds a shadow pixel first and proves nothing.
        let edge = straight
            .chunks_exact(4)
            .zip(premultiplied.chunks_exact(4))
            .find(|(s, _)| s[3] > 40 && s[3] < 215 && s[0] > 0)
            .expect("a partially covered lit edge pixel exists");
        assert!(
            edge.0[0] > edge.1[0],
            "straight {} should be brighter than premultiplied {}",
            edge.0[0],
            edge.1[0]
        );
        assert_eq!(edge.0[3], edge.1[3], "alpha must be identical either way");
    }

    #[test]
    fn both_alpha_modes_agree_wherever_the_pixel_is_fully_opaque() {
        // Premultiplying by 255 is the identity, so the interior must be byte-identical. A
        // difference here would mean the two paths had drifted into drawing different pictures.
        let layout = layout();
        let mask = BubbleMask::new(&layout);
        let frame = SquareFrame::solid(64, [200, 30, 30, 255]);
        let straight = mask.composite(&frame, AlphaMode::Straight);
        let premultiplied = mask.composite(&frame, AlphaMode::Premultiplied);
        for (s, p) in straight.chunks_exact(4).zip(premultiplied.chunks_exact(4)) {
            if s[3] == 255 {
                assert_eq!(s, p);
            }
        }
    }

    #[test]
    fn premultiplying_a_straight_surface_reproduces_the_premultiplied_one() {
        // This is the load-bearing claim behind compositing only once: if the derived surface
        // differed from a directly composited one, the bubble on screen and the bubble burned
        // into the recording would be drawing different pictures.
        let layout = layout();
        let mask = BubbleMask::new(&layout);
        let frame = SquareFrame::solid(64, [200, 30, 30, 255]);
        let straight = mask.composite(&frame, AlphaMode::Straight);
        let mut derived = vec![0u8; straight.len()];
        premultiply_into(&straight, &mut derived);
        assert_eq!(derived, mask.composite(&frame, AlphaMode::Premultiplied));
    }

    #[test]
    fn premultiplying_into_a_short_buffer_stops_rather_than_panicking() {
        let mut out = vec![0u8; 4];
        premultiply_into(&[10, 20, 30, 128, 40, 50, 60, 255], &mut out);
        assert_eq!(out, vec![5, 10, 15, 128]);
    }

    #[test]
    fn the_mask_reproduces_what_the_one_shot_renderer_draws() {
        // BubbleMask is an optimisation, not a redesign: if it ever draws something different
        // from `render_bubble`, the bubble on screen and the bubble in the recording diverge.
        let layout = layout();
        let frame = SquareFrame::solid(64, [12, 200, 90, 255]);
        assert_eq!(
            BubbleMask::new(&layout).composite(&frame, AlphaMode::Premultiplied),
            render_bubble(&layout, &frame)
        );
    }

    #[test]
    fn the_mask_reports_the_camera_side_the_layout_asks_for() {
        // The overlay writer sizes its `to_square` call from this. If it disagreed with the
        // layout, the camera image would be scaled wrong inside a correctly drawn circle.
        let layout = layout();
        assert_eq!(
            BubbleMask::new(&layout).camera_side(),
            layout.circle_diameter
        );
    }

    #[test]
    fn compositing_into_a_short_buffer_stops_rather_than_panicking() {
        let layout = layout();
        let mask = BubbleMask::new(&layout);
        let mut out = vec![0u8; 16];
        mask.composite_into(
            &SquareFrame::solid(64, [1, 2, 3, 4]),
            AlphaMode::Straight,
            &mut out,
        );
        assert_eq!(out.len(), 16);
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
    fn to_square_into_reuses_the_buffer_it_is_given() {
        // The per-frame path must not allocate; if it did, the camera thread would be doing
        // thirty heap allocations a second for the life of a recording.
        let src: Vec<u8> = (0..(8 * 8 * 4)).map(|i| (i % 256) as u8).collect();
        let mut frame = SquareFrame::solid(4, [0, 0, 0, 0]);
        let before = frame.bgra.as_ptr();
        to_square_into(&src, 8, 8, &mut frame);
        assert_eq!(frame.bgra.as_ptr(), before, "buffer was reallocated");
        assert_eq!(frame.bgra, to_square(&src, 8, 8, 4).bgra);
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
