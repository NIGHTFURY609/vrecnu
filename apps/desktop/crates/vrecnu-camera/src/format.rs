//! Choosing which of a device's formats to open, as pure logic.
//!
//! Split out from the backend deliberately: format negotiation is where camera capture
//! actually goes wrong, and it is the one part that can be tested without a webcam attached.
//! The backend below it is then thin enough to reason about by reading.

use vrecnu_types::Size;

/// One capture format a device offers, reduced to what the choice depends on.
///
/// Deliberately not the backend's own type. Keeping the decision expressible without naming
/// `nokhwa` is what lets it be unit-tested, and what would let the backend be replaced without
/// rewriting the rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceFormat {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Frames per second the device advertises for this format.
    pub fps: u32,
}

impl DeviceFormat {
    const fn area(self) -> u64 {
        (self.width as u64) * (self.height as u64)
    }

    const fn covers(self, min: Size) -> bool {
        self.width >= min.width && self.height >= min.height
    }
}

/// Picks the index of the format to open: the **smallest** one that still covers `min`.
///
/// Smallest, not largest, and that is the whole point. The camera image is centre-cropped to a
/// circle a couple of hundred pixels across (see [`crate::square_crop`]); opening a 4K sensor
/// to fill it costs USB bandwidth, a much larger per-frame decode, and CPU during recording —
/// the one moment AGENTS.md §1 has no headroom to spare — and every one of those extra pixels
/// is thrown away by the crop before anybody sees it.
///
/// Ties on size are broken by the higher frame rate, because at equal cost a smoother bubble is
/// strictly better.
///
/// When nothing covers `min` the **largest** available format is chosen rather than failing.
/// A slightly soft bubble upscaled from a 320x180 device is obviously better than refusing to
/// show the camera at all, and the caller has no more useful action to take on an error here.
/// Returns `None` only for an empty list, which means the device offered nothing we can decode
/// — that *is* worth an error, and [`crate::CameraError::FormatNegotiationFailed`] is it.
#[must_use]
pub fn select_format(available: &[DeviceFormat], min: Size) -> Option<usize> {
    let covering = available
        .iter()
        .enumerate()
        .filter(|(_, format)| format.covers(min))
        .min_by_key(|(_, format)| (format.area(), std::cmp::Reverse(format.fps)))
        .map(|(index, _)| index);

    covering.or_else(|| {
        available
            .iter()
            .enumerate()
            .max_by_key(|(_, format)| (format.area(), format.fps))
            .map(|(index, _)| index)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(width: u32, height: u32, fps: u32) -> DeviceFormat {
        DeviceFormat { width, height, fps }
    }

    fn min(side: u32) -> Size {
        Size {
            width: side,
            height: side,
        }
    }

    #[test]
    fn the_smallest_covering_format_wins() {
        // A 240 px bubble does not need a 4K sensor, and opening one costs bandwidth, decode
        // time and CPU for pixels the circular crop discards.
        let formats = [
            format(3840, 2160, 30),
            format(640, 480, 30),
            format(1920, 1080, 30),
        ];
        assert_eq!(select_format(&formats, min(240)), Some(1));
    }

    #[test]
    fn a_format_that_is_too_small_in_one_axis_does_not_count_as_covering() {
        // 320x180 is wider than a 240 px bubble but not tall enough, and the crop is square.
        let formats = [format(320, 180, 30), format(640, 480, 30)];
        assert_eq!(select_format(&formats, min(240)), Some(1));
    }

    #[test]
    fn equal_sizes_are_broken_by_the_higher_frame_rate() {
        let formats = [format(640, 480, 15), format(640, 480, 30)];
        assert_eq!(select_format(&formats, min(240)), Some(1));
    }

    #[test]
    fn nothing_covering_falls_back_to_the_largest_rather_than_failing() {
        // A soft bubble beats no bubble, and there is no useful action for the caller to take
        // on an error here.
        let formats = [format(160, 120, 30), format(320, 180, 30)];
        assert_eq!(select_format(&formats, min(240)), Some(1));
    }

    #[test]
    fn an_empty_list_is_the_one_genuine_negotiation_failure() {
        assert_eq!(select_format(&[], min(240)), None);
    }

    #[test]
    fn an_exact_match_is_preferred_over_anything_larger() {
        let formats = [format(1280, 720, 30), format(240, 240, 30)];
        assert_eq!(select_format(&formats, min(240)), Some(1));
    }
}
