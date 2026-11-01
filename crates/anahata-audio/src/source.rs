use std::fs::File;
use std::path::{Path, PathBuf};
use symphonia::core::audio::{AudioBufferRef, SampleBuffer, SignalSpec};
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::{Time, TimeBase};

use crate::channels::downmix_to_stereo;
use crate::error::{AudioError, AudioResult};

pub struct Source {
    path: PathBuf,
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    sample_rate: u32,
    channels: usize,
    sample_buf: Option<SampleBuffer<f32>>,
    stereo_buffer: Vec<f32>,
    total_duration_secs: Option<f64>,
}

impl Source {
    pub fn open<P: AsRef<Path>>(path: P) -> AudioResult<Self> {
        let path_buf = path.as_ref().to_path_buf();
        let file = File::open(&path_buf).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AudioError::FileNotFound(path_buf.clone())
            } else {
                AudioError::Io(e)
            }
        })?;

        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path_buf.extension().and_then(|s| s.to_str()) {
            hint.with_extension(ext);
        }

        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let metadata_opts = MetadataOptions::default();

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &metadata_opts)
            .map_err(|e| AudioError::UnsupportedFormat(format!("Probe error: {e}")))?;

        let reader = probed.format;

        // Find the first valid audio track
        let track = reader
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| {
                AudioError::UnsupportedFormat("No supported audio tracks found in file".to_string())
            })?;

        let track_id = track.id;
        let time_base = track.codec_params.time_base;
        let sample_rate = track
            .codec_params
            .sample_rate
            .unwrap_or(44100);

        let channels = track
            .codec_params
            .channels
            .map(|c| c.count())
            .unwrap_or(2);

        let total_duration_secs = if let (Some(tb), Some(n_frames)) =
            (track.codec_params.time_base, track.codec_params.n_frames)
        {
            let time = tb.calc_time(n_frames);
            Some(time.seconds as f64 + time.frac)
        } else {
            None
        };

        let decoder_opts = DecoderOptions::default();
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &decoder_opts)
            .map_err(|e| AudioError::DecodeFailed(format!("Decoder init error: {e}")))?;

        Ok(Self {
            path: path_buf,
            reader,
            decoder,
            track_id,
            time_base,
            sample_rate,
            channels,
            sample_buf: None,
            stereo_buffer: Vec::new(),
            total_duration_secs,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn duration_secs(&self) -> Option<f64> {
        self.total_duration_secs
    }

    pub fn time_base(&self) -> Option<TimeBase> {
        self.time_base
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Seeks to the specified position in seconds.
    pub fn seek(&mut self, time_seconds: f64) -> AudioResult<()> {
        let seek_to = SeekTo::Time {
            time: Time::from(time_seconds),
            track_id: Some(self.track_id),
        };

        self.reader
            .seek(SeekMode::Accurate, seek_to)
            .map_err(|e| AudioError::SeekError(format!("Seek failed: {e}")))?;

        self.decoder.reset();
        Ok(())
    }

    /// Decodes the next packet and returns interleaved stereo f32 samples.
    /// Returns `Ok(None)` on normal EOF.
    pub fn next_packet_stereo(&mut self) -> AudioResult<Option<&[f32]>> {
        loop {
            let packet = match self.reader.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(None);
                }
                Err(SymphoniaError::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(e) => {
                    return Err(AudioError::DecodeFailed(format!("Error reading packet: {e}")));
                }
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            match self.decoder.decode(&packet) {
                Ok(audio_buf_ref) => {
                    Self::convert_audio_buffer(
                        audio_buf_ref,
                        &mut self.sample_buf,
                        &mut self.stereo_buffer,
                    );
                    return Ok(Some(&self.stereo_buffer));
                }
                Err(SymphoniaError::DecodeError(e)) => {
                    log::warn!("Decode packet error (skipping packet): {e}");
                    continue;
                }
                Err(SymphoniaError::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(e) => {
                    return Err(AudioError::DecodeFailed(format!("Unrecoverable decode error: {e}")));
                }
            }
        }
    }

    fn convert_audio_buffer(
        audio_buf: AudioBufferRef,
        sample_buf: &mut Option<SampleBuffer<f32>>,
        stereo_buffer: &mut Vec<f32>,
    ) {
        let spec = *audio_buf.spec();
        let num_frames = audio_buf.frames();

        if sample_buf.is_none() || sample_buf.as_ref().unwrap().capacity() < num_frames {
            *sample_buf = Some(SampleBuffer::<f32>::new(
                num_frames as u64,
                SignalSpec::new(spec.rate, spec.channels),
            ));
        }

        let sbuf = sample_buf.as_mut().unwrap();
        sbuf.copy_interleaved_ref(audio_buf);

        let samples = sbuf.samples();
        let channels = spec.channels.count();
        downmix_to_stereo(samples, channels, stereo_buffer);
    }
}
