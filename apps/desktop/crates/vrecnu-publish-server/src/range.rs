/// A resolved byte range, inclusive at both ends as HTTP defines it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangeSpec {
    /// First byte to send.
    pub start: u64,
    /// Last byte to send, inclusive.
    pub end: u64,
}

impl RangeSpec {
    /// Number of bytes the range covers.
    #[must_use]
    pub const fn len(self) -> u64 {
        self.end - self.start + 1
    }

    /// Whether the range covers no bytes. Never true for a well-formed range.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.end < self.start
    }

    /// The `Content-Range` value for a 206 response.
    #[must_use]
    pub fn content_range(self, total: u64) -> String {
        format!("bytes {}-{}/{}", self.start, self.end, total)
    }
}

/// Parses a single-range `Range` header against a known total size.
///
/// `tiny_http` does not implement Range, so this is ours to get right — and it is
/// load-bearing rather than incidental. The browser sends `Range` for two reasons: a `<video>`
/// preview element always opens with `bytes=0-`, and the publish page uploads by streaming
/// windows rather than materialising a multi-gigabyte `Blob`.
///
/// Returns `None` for a syntactically invalid header (the caller should ignore it and send
/// 200) and `Some(Err(()))` for a syntactically valid but unsatisfiable one (416).
///
/// Multi-range requests are deliberately unsupported: `multipart/byteranges` is real work and
/// no browser needs it here, so the caller serves the whole file instead.
#[must_use]
#[allow(clippy::result_unit_err)]
pub fn parse_range_header(header: &str, total: u64) -> Option<Result<RangeSpec, ()>> {
    let spec = header.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        // Multi-range. Signal "not parseable" so the caller falls back to a 200.
        return None;
    }
    let (start_str, end_str) = spec.split_once('-')?;
    let start_str = start_str.trim();
    let end_str = end_str.trim();

    if total == 0 {
        return Some(Err(()));
    }

    // `bytes=-500` means the *last* 500 bytes, not "up to byte 500".
    if start_str.is_empty() {
        let suffix: u64 = end_str.parse().ok()?;
        if suffix == 0 {
            return Some(Err(()));
        }
        let len = suffix.min(total);
        return Some(Ok(RangeSpec {
            start: total - len,
            end: total - 1,
        }));
    }

    let start: u64 = start_str.parse().ok()?;
    if start >= total {
        return Some(Err(()));
    }

    let end = if end_str.is_empty() {
        total - 1
    } else {
        let parsed: u64 = end_str.parse().ok()?;
        parsed.min(total - 1)
    };

    if end < start {
        return Some(Err(()));
    }
    Some(Ok(RangeSpec { start, end }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_open_ended_range_runs_to_the_last_byte() {
        // What a <video> element sends when it opens a source.
        let parsed = parse_range_header("bytes=0-", 1000).unwrap().unwrap();
        assert_eq!(parsed, RangeSpec { start: 0, end: 999 });
        assert_eq!(parsed.len(), 1000);
    }

    #[test]
    fn a_bounded_range_is_inclusive_at_both_ends() {
        let parsed = parse_range_header("bytes=0-99", 1000).unwrap().unwrap();
        assert_eq!(parsed.len(), 100);
    }

    #[test]
    fn a_suffix_range_counts_back_from_the_end() {
        let parsed = parse_range_header("bytes=-500", 1000).unwrap().unwrap();
        assert_eq!(
            parsed,
            RangeSpec {
                start: 500,
                end: 999
            }
        );
    }

    #[test]
    fn a_suffix_longer_than_the_file_yields_the_whole_file() {
        let parsed = parse_range_header("bytes=-5000", 1000).unwrap().unwrap();
        assert_eq!(parsed, RangeSpec { start: 0, end: 999 });
    }

    #[test]
    fn an_end_past_the_file_is_clamped_rather_than_rejected() {
        let parsed = parse_range_header("bytes=900-99999", 1000)
            .unwrap()
            .unwrap();
        assert_eq!(
            parsed,
            RangeSpec {
                start: 900,
                end: 999
            }
        );
    }

    #[test]
    fn a_start_past_the_file_is_unsatisfiable() {
        assert_eq!(parse_range_header("bytes=2000-", 1000), Some(Err(())));
    }

    #[test]
    fn a_reversed_range_is_unsatisfiable() {
        assert_eq!(parse_range_header("bytes=500-100", 1000), Some(Err(())));
    }

    #[test]
    fn garbage_is_not_parseable_so_the_caller_sends_the_whole_file() {
        assert_eq!(parse_range_header("bytes=abc", 1000), None);
        assert_eq!(parse_range_header("items=0-10", 1000), None);
        assert_eq!(parse_range_header("", 1000), None);
    }

    #[test]
    fn multi_range_is_declined_rather_than_half_implemented() {
        // Serving a 200 beats emitting malformed multipart/byteranges.
        assert_eq!(parse_range_header("bytes=0-99,200-299", 1000), None);
    }

    #[test]
    fn content_range_reports_the_total() {
        let spec = RangeSpec { start: 0, end: 99 };
        assert_eq!(spec.content_range(1000), "bytes 0-99/1000");
    }

    #[test]
    fn an_empty_file_satisfies_nothing() {
        assert_eq!(parse_range_header("bytes=0-", 0), Some(Err(())));
    }
}
