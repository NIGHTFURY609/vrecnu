use std::path::Path;

use vrecnu_types::{EditSpec, Size};

use crate::H264Encoder;

/// Everything the live encoder needs to build its command line.
#[derive(Debug, Clone)]
pub struct LiveEncodeParams<'a> {
    /// Capture resolution.
    pub size: Size,
    /// Target frame rate.
    pub fps: u32,
    /// Target video bitrate in bits per second.
    pub video_bitrate: u32,
    /// Which H.264 encoder the startup probe selected.
    pub encoder: H264Encoder,
    /// Named pipe carrying 48 kHz stereo `s16le`, if audio is being recorded.
    pub audio_pipe: Option<&'a str>,
    /// Named pipe carrying BGRA camera frames for the burned-in bubble, if enabled.
    pub camera_pipe: Option<&'a str>,
    /// Size of the camera overlay in pixels.
    pub camera_size: Option<Size>,
    /// Where to write the fragmented MP4.
    pub output: &'a Path,
}

/// Builds the live-encode command line.
///
/// The fragmented-MP4 flags are the important part: with `+frag_keyframe+empty_moov` the file
/// is playable at every instant, so a crash costs the tail of a recording rather than all of
/// it. `-thread_queue_size 1024` is equally non-negotiable — the default of 8 is far too small
/// for 8 MB video frames and causes real, reported drops under perfectly normal load.
#[must_use]
pub fn live_encode_args(params: &LiveEncodeParams<'_>) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-loglevel".into(),
        "warning".into(),
        "-progress".into(),
        "pipe:2".into(),
        "-stats_period".into(),
        "1".into(),
        // ---- input 0: raw screen frames on stdin ----
        "-f".into(),
        "rawvideo".into(),
        "-pixel_format".into(),
        "bgra".into(),
        "-video_size".into(),
        format!("{}x{}", params.size.width, params.size.height),
        "-framerate".into(),
        params.fps.to_string(),
        "-thread_queue_size".into(),
        "1024".into(),
        "-i".into(),
        "pipe:0".into(),
    ];

    if let Some(pipe) = params.audio_pipe {
        args.extend([
            "-f".into(),
            "s16le".into(),
            "-ar".into(),
            "48000".into(),
            "-ac".into(),
            "2".into(),
            "-thread_queue_size".into(),
            "1024".into(),
            "-i".into(),
            pipe.to_string(),
        ]);
    }

    if let (Some(pipe), Some(size)) = (params.camera_pipe, params.camera_size) {
        args.extend([
            "-f".into(),
            "rawvideo".into(),
            "-pixel_format".into(),
            "bgra".into(),
            "-video_size".into(),
            format!("{}x{}", size.width, size.height),
            "-framerate".into(),
            params.fps.to_string(),
            "-thread_queue_size".into(),
            "512".into(),
            "-i".into(),
            pipe.to_string(),
        ]);
    }

    // Map explicitly. The camera input index depends on whether audio is present, and relying
    // on FFmpeg's default stream selection here would silently pick the wrong video stream.
    let camera_index = if params.audio_pipe.is_some() { 2 } else { 1 };
    if params.camera_pipe.is_some() {
        args.extend([
            "-filter_complex".into(),
            format!("[0:v][{camera_index}:v]overlay=x=W-w-32:y=H-h-32:shortest=0[v]"),
            "-map".into(),
            "[v]".into(),
        ]);
    } else {
        args.extend(["-map".into(), "0:v:0".into()]);
    }
    if params.audio_pipe.is_some() {
        args.extend(["-map".into(), "1:a:0".into()]);
    }

    args.extend([
        "-c:v".into(),
        params.encoder.ffmpeg_name().into(),
        "-b:v".into(),
        params.video_bitrate.to_string(),
        "-maxrate".into(),
        (params.video_bitrate + params.video_bitrate / 3).to_string(),
        "-bufsize".into(),
        (params.video_bitrate * 2).to_string(),
        "-profile:v".into(),
        params.encoder.profile_value().into(),
        "-g".into(),
        (params.fps * 2).to_string(),
        "-pix_fmt".into(),
        "nv12".into(),
    ]);

    if params.audio_pipe.is_some() {
        args.extend([
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "128k".into(),
            "-ar".into(),
            "48000".into(),
            "-ac".into(),
            "2".into(),
        ]);
    }

    args.extend([
        "-movflags".into(),
        // `default_base_moof`, not `default_base_is_moof`. FFmpeg's flag is named after the
        // effect ("set the default-base-is-moof flag"), and the plausible-looking longer spelling
        // is rejected by the muxer at header-write time — i.e. after recording has begun.
        "+frag_keyframe+empty_moov+default_base_moof".into(),
        "-frag_duration".into(),
        "2000000".into(),
        "-y".into(),
        params.output.display().to_string(),
    ]);

    args
}

/// Builds the finalize/edit command line.
///
/// `+faststart` moves `moov` to the front, which is what lets Drive's preview player seek —
/// the exact failure ADR-003 §3.2 was written about. When the edit changes nothing, this takes
/// the `-c copy` path and becomes a remux measured in milliseconds rather than a re-encode.
#[must_use]
pub fn render_args(
    input: &Path,
    output: &Path,
    edit: &EditSpec,
    encoder: H264Encoder,
    video_bitrate: u32,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-loglevel".into(),
        "warning".into(),
        "-progress".into(),
        "pipe:2".into(),
    ];

    // `-ss` before `-i` seeks by keyframe index rather than decoding to the cut point, which
    // is what keeps a trim fast on a long recording.
    if let Some(start) = edit.start_ms {
        if start > 0 {
            args.extend(["-ss".into(), format!("{:.3}", start as f64 / 1000.0)]);
        }
    }
    if let Some(end) = edit.end_ms {
        args.extend(["-to".into(), format!("{:.3}", end as f64 / 1000.0)]);
    }
    args.extend(["-i".into(), input.display().to_string()]);

    if edit.is_passthrough() {
        args.extend(["-c".into(), "copy".into()]);
    } else {
        if !edit.video.is_identity() {
            args.extend([
                "-vf".into(),
                format!(
                    "eq=brightness={:.3}:contrast={:.3}:saturation={:.3}",
                    edit.video.brightness, edit.video.contrast, edit.video.saturation
                ),
            ]);
        }
        let mut audio_chain: Vec<String> = Vec::new();
        if edit.audio.denoise {
            audio_chain.push("highpass=f=90".into());
        }
        if (edit.audio.gain - 1.0).abs() > f32::EPSILON {
            audio_chain.push(format!("volume={:.3}", edit.audio.gain));
        }
        if edit.audio.normalize {
            audio_chain.push("loudnorm=I=-16:TP=-1.5:LRA=11".into());
        }
        if !audio_chain.is_empty() {
            args.extend(["-af".into(), audio_chain.join(",")]);
        }

        if edit.video.is_identity() {
            args.extend(["-c:v".into(), "copy".into()]);
        } else {
            args.extend([
                "-c:v".into(),
                encoder.ffmpeg_name().into(),
                "-b:v".into(),
                video_bitrate.to_string(),
                "-profile:v".into(),
                encoder.profile_value().into(),
                "-pix_fmt".into(),
                "nv12".into(),
            ]);
        }
        if audio_chain.is_empty() {
            args.extend(["-c:a".into(), "copy".into()]);
        } else {
            args.extend(["-c:a".into(), "aac".into(), "-b:a".into(), "128k".into()]);
        }
    }

    args.extend([
        "-movflags".into(),
        "+faststart".into(),
        "-y".into(),
        output.display().to_string(),
    ]);
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use vrecnu_types::{AudioAdjust, VideoAdjust};

    fn params<'a>(
        output: &'a Path,
        audio: Option<&'a str>,
        camera: Option<&'a str>,
    ) -> LiveEncodeParams<'a> {
        LiveEncodeParams {
            size: Size {
                width: 1920,
                height: 1080,
            },
            fps: 30,
            video_bitrate: 6_000_000,
            encoder: H264Encoder::MediaFoundation,
            audio_pipe: audio,
            camera_pipe: camera,
            camera_size: camera.map(|_| Size {
                width: 240,
                height: 240,
            }),
            output,
        }
    }

    fn joined(args: &[String]) -> String {
        args.join(" ")
    }

    #[test]
    fn live_encode_writes_fragmented_mp4() {
        // The most important flags in the file: they are what makes a mid-recording crash
        // cost the tail of a video instead of the whole thing.
        let out = PathBuf::from("raw.mp4");
        let args = live_encode_args(&params(&out, None, None));
        let text = joined(&args);
        assert!(
            text.contains("+frag_keyframe+empty_moov+default_base_moof"),
            "{text}"
        );
        // Spelled `default_base_is_moof`, this whole line is rejected by the muxer and the
        // recording never starts. Verified against ffmpeg 7.1.
        assert!(!text.contains("default_base_is_moof"), "{text}");
        assert!(text.contains("-frag_duration 2000000"), "{text}");
    }

    #[test]
    fn live_encode_raises_the_thread_queue_far_above_the_default() {
        // FFmpeg's default of 8 is far too small for 8 MB frames and causes genuine drops.
        let out = PathBuf::from("raw.mp4");
        let args = live_encode_args(&params(&out, None, None));
        assert!(joined(&args).contains("-thread_queue_size 1024"));
    }

    #[test]
    fn live_encode_declares_bgra_at_the_capture_size() {
        let out = PathBuf::from("raw.mp4");
        let args = live_encode_args(&params(&out, None, None));
        let text = joined(&args);
        assert!(text.contains("-pixel_format bgra"), "{text}");
        assert!(text.contains("-video_size 1920x1080"), "{text}");
    }

    #[test]
    fn audio_arrives_as_a_second_input_and_is_mapped() {
        let out = PathBuf::from("raw.mp4");
        let args = live_encode_args(&params(&out, Some(r"\\.\pipe\vrecnu-audio"), None));
        let text = joined(&args);
        assert!(text.contains(r"-i \\.\pipe\vrecnu-audio"), "{text}");
        assert!(text.contains("-map 1:a:0"), "{text}");
        assert!(text.contains("-c:a aac"), "{text}");
    }

    #[test]
    fn the_camera_overlay_indexes_correctly_without_audio() {
        // With no audio the camera is input 1, not 2. Getting this wrong overlays the screen
        // onto itself and silently drops the webcam.
        let out = PathBuf::from("raw.mp4");
        let args = live_encode_args(&params(&out, None, Some(r"\\.\pipe\cam")));
        assert!(
            joined(&args).contains("[0:v][1:v]overlay="),
            "{}",
            joined(&args)
        );
    }

    #[test]
    fn the_camera_overlay_indexes_correctly_with_audio() {
        let out = PathBuf::from("raw.mp4");
        let args = live_encode_args(&params(&out, Some(r"\\.\pipe\aud"), Some(r"\\.\pipe\cam")));
        assert!(
            joined(&args).contains("[0:v][2:v]overlay="),
            "{}",
            joined(&args)
        );
    }

    #[test]
    fn h264_mf_is_the_encoder_named() {
        let out = PathBuf::from("raw.mp4");
        let args = live_encode_args(&params(&out, None, None));
        let text = joined(&args);
        assert!(text.contains("-c:v h264_mf"), "{text}");
        assert!(
            !text.contains("libx264"),
            "libx264 cannot exist in an LGPL build"
        );
    }

    #[test]
    fn the_profile_is_written_in_the_dialect_the_chosen_encoder_speaks() {
        // `-profile:v high` makes h264_mf fail at encoder-open, which happens *after* capture
        // has started. Observed against a real ffmpeg, not inferred from documentation.
        let out = PathBuf::from("raw.mp4");
        let mut mf = params(&out, None, None);
        mf.encoder = H264Encoder::MediaFoundation;
        assert!(joined(&live_encode_args(&mf)).contains("-profile:v 100"));

        let mut openh264 = params(&out, None, None);
        openh264.encoder = H264Encoder::OpenH264;
        assert!(joined(&live_encode_args(&openh264)).contains("-profile:v high"));
    }

    #[test]
    fn a_re_encoding_render_uses_the_same_profile_dialect() {
        // The live path and the render path pick encoders independently; fixing only one leaves
        // the identical failure waiting in export.
        let edit = EditSpec {
            video: VideoAdjust {
                brightness: 0.2,
                ..VideoAdjust::default()
            },
            ..EditSpec::default()
        };
        let args = render_args(
            Path::new("in.mp4"),
            Path::new("out.mp4"),
            &edit,
            H264Encoder::MediaFoundation,
            6_000_000,
        );
        assert!(
            joined(&args).contains("-profile:v 100"),
            "{}",
            joined(&args)
        );
    }

    #[test]
    fn an_empty_edit_takes_the_copy_fast_path() {
        let args = render_args(
            Path::new("in.mp4"),
            Path::new("out.mp4"),
            &EditSpec::default(),
            H264Encoder::MediaFoundation,
            6_000_000,
        );
        let text = joined(&args);
        assert!(text.contains("-c copy"), "{text}");
        assert!(text.contains("+faststart"), "{text}");
    }

    #[test]
    fn every_render_moves_moov_to_the_front() {
        // Without +faststart, Drive's preview player cannot seek — ADR-003 §3.2's failure.
        let edit = EditSpec {
            video: VideoAdjust {
                brightness: 0.1,
                ..VideoAdjust::default()
            },
            ..EditSpec::default()
        };
        let args = render_args(
            Path::new("in.mp4"),
            Path::new("out.mp4"),
            &edit,
            H264Encoder::MediaFoundation,
            6_000_000,
        );
        assert!(joined(&args).contains("+faststart"));
    }

    #[test]
    fn a_trim_seeks_before_the_input_for_speed() {
        let edit = EditSpec {
            start_ms: Some(3_500),
            end_ms: Some(92_000),
            ..EditSpec::default()
        };
        let args = render_args(
            Path::new("in.mp4"),
            Path::new("out.mp4"),
            &edit,
            H264Encoder::MediaFoundation,
            6_000_000,
        );
        let ss = args.iter().position(|a| a == "-ss").expect("-ss present");
        let input = args.iter().position(|a| a == "-i").expect("-i present");
        assert!(ss < input, "-ss must precede -i to seek by keyframe");
        assert!(joined(&args).contains("-to 92.000"));
    }

    #[test]
    fn audio_only_edits_leave_the_video_stream_untouched() {
        // Re-encoding video to change loudness would be a pointless quality loss.
        let edit = EditSpec {
            audio: AudioAdjust {
                normalize: true,
                ..AudioAdjust::default()
            },
            ..EditSpec::default()
        };
        let args = render_args(
            Path::new("in.mp4"),
            Path::new("out.mp4"),
            &edit,
            H264Encoder::MediaFoundation,
            6_000_000,
        );
        let text = joined(&args);
        assert!(text.contains("-c:v copy"), "{text}");
        assert!(text.contains("loudnorm"), "{text}");
    }

    #[test]
    fn video_only_edits_leave_the_audio_stream_untouched() {
        let edit = EditSpec {
            video: VideoAdjust {
                saturation: 1.2,
                ..VideoAdjust::default()
            },
            ..EditSpec::default()
        };
        let args = render_args(
            Path::new("in.mp4"),
            Path::new("out.mp4"),
            &edit,
            H264Encoder::MediaFoundation,
            6_000_000,
        );
        let text = joined(&args);
        assert!(text.contains("-c:a copy"), "{text}");
        assert!(text.contains("eq=brightness"), "{text}");
    }

    #[test]
    fn the_audio_filter_chain_is_ordered_denoise_gain_normalize() {
        let edit = EditSpec {
            audio: AudioAdjust {
                gain: 1.5,
                normalize: true,
                denoise: true,
            },
            ..EditSpec::default()
        };
        let args = render_args(
            Path::new("in.mp4"),
            Path::new("out.mp4"),
            &edit,
            H264Encoder::MediaFoundation,
            6_000_000,
        );
        let chain = args
            .iter()
            .position(|a| a == "-af")
            .map(|i| args[i + 1].clone())
            .expect("-af present");
        // Normalising before boosting would undo the loudness target.
        assert_eq!(
            chain,
            "highpass=f=90,volume=1.500,loudnorm=I=-16:TP=-1.5:LRA=11"
        );
    }
}
