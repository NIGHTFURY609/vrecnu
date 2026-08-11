/// Failures specific to audio capture.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AudioError {
    /// No render endpoint to open in loopback mode.
    #[error("no default audio output device to capture system audio from")]
    NoDefaultOutput,
    /// No capture endpoint.
    #[error("no audio input device available")]
    NoInputDevice,
    /// The platform or backend cannot do loopback capture.
    #[error("this system does not support WASAPI loopback capture")]
    LoopbackUnsupported,
    /// The device rejected the requested format and no conversion is possible.
    #[error("device {device} does not support {rate} Hz / {channels} channels")]
    UnsupportedConfig {
        /// Device name.
        device: String,
        /// Requested sample rate.
        rate: u32,
        /// Requested channel count.
        channels: u16,
    },
    /// The device vanished mid-recording — unplugged, or switched by the OS.
    #[error("audio device {name} disconnected during recording")]
    DeviceDisconnected {
        /// Device name.
        name: String,
    },
    /// Anything the backend raised that has no better classification.
    #[error("audio backend failed: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}
