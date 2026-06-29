//! Audio playback engine, a thin wrapper around `rodio`.
//!
//! [`Player`] owns the OS audio stream and a single `Sink` (rodio's playback
//! queue). The UI never touches rodio directly; it calls [`Player::play`],
//! [`Player::pause`], [`Player::seek`] and friends.
//!
//! Seeking is the tricky part: rodio has no native "jump to position", so we
//! decode and discard samples up to the target. That is slow, so it runs on a
//! background thread. An atomic "operation id" makes sure that if the user
//! seeks again before the first seek finishes, the stale thread quietly drops
//! its result instead of starting playback from the wrong place.

use crate::lab::dsp::{self, SharedAnalysis, Tap};
use rodio::{Decoder, OutputStream, Sink, Source};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Owns the audio output and exposes simple transport controls.
pub struct Player {
    /// Output stream; kept alive for the player's lifetime (dropping it stops
    /// all audio). Underscored because it is never read directly.
    _stream: OutputStream,
    /// Stream handle, retained alongside the stream. Never read directly.
    _stream_handle: rodio::OutputStreamHandle,
    /// The playback queue. `Arc` so background seek threads can append to it.
    sink: Arc<Sink>,
    /// Monotonically increasing id identifying the most recent transport
    /// operation. Background seeks compare against this to detect that they
    /// have been superseded and should abort. See [`Player::seek`].
    current_operation_id: Arc<AtomicU64>,
    /// Live ring buffer of the samples flowing to the sink, shared with the Lab
    /// window. Every source is wrapped in a [`Tap`] that writes into this.
    analysis: SharedAnalysis,
}

impl Player {
    /// Initializes the default audio device and a sink at 50% volume.
    ///
    /// # Panics
    /// Panics if no audio output device is available or the sink cannot be
    /// created — without audio the player has nothing to do.
    pub fn new() -> Self {
        let (stream, handle) =
            OutputStream::try_default().expect("❌ Failed to initialize audio output device");
        let sink = Sink::try_new(&handle).expect("❌ Failed to create audio sink");
        sink.set_volume(0.5);

        println!("🔊 Audio system ready.");

        Self {
            _stream: stream,
            _stream_handle: handle,
            sink: Arc::new(sink),
            current_operation_id: Arc::new(AtomicU64::new(0)),
            analysis: dsp::new_shared(),
        }
    }

    /// Returns a clone of the shared analysis buffer for the Lab window to read.
    pub fn analysis(&self) -> SharedAnalysis {
        self.analysis.clone()
    }

    /// Loads and starts playing the file at `path`, replacing whatever was
    /// playing. Returns the track's total duration when it can be determined.
    ///
    /// Duration comes from rodio when available; for MP3s that report `None`,
    /// we fall back to the `mp3-duration` crate. Returns `None` if the file
    /// cannot be opened or decoded.
    pub fn play(&self, path: &str) -> Option<Duration> {
        // Invalidate any in-flight background seek so it does not resurrect the
        // previous track on top of this one.
        self.current_operation_id.fetch_add(1, Ordering::SeqCst);

        self.sink.stop();
        println!("▶️ Loading track: {}", path);

        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) => {
                println!("❌ Failed to open file: {:?}", e);
                return None;
            }
        };

        let source = match Decoder::new(BufReader::new(file)) {
            Ok(s) => s,
            Err(e) => {
                println!("❌ Failed to decode audio: {:?}", e);
                return None;
            }
        };

        // Only use rodio's *instant* duration here. For MP3s it is usually
        // `None`, and the fallback (scanning the whole file with `mp3-duration`)
        // is slow — running it here would freeze the UI on every track change.
        // The caller computes that fallback off-thread; see `App::play_track`.
        let duration = source.total_duration();

        println!("📊 Decoded successfully. Duration: {:?}", duration);
        // Mirror the samples into the shared analysis buffer as they play.
        self.sink.append(Tap::new(source, self.analysis.clone()));
        self.sink.play();
        duration
    }

    /// Streams audio directly from `url` and starts playing it, **without ever
    /// downloading the whole file first** — the instant-play path for the
    /// YouTube tab.
    ///
    /// `ffmpeg` reads the remote stream and transcodes it to raw PCM on the fly;
    /// we feed that PCM straight into the sink as it arrives. This is how we play
    /// YouTube's AAC/Opus audio (which rodio cannot decode directly) with no
    /// download wait. `duration_hint` (known from the search result) is returned
    /// for the progress bar, since a live stream reports no length. Returns
    /// `None` if ffmpeg could not be started.
    pub fn play_stream(
        &self,
        ffmpeg: &Path,
        url: &str,
        start: Duration,
        duration_hint: Option<Duration>,
    ) -> Option<Duration> {
        // Supersede any previous track (mirrors `play`/`seek`).
        self.current_operation_id.fetch_add(1, Ordering::SeqCst);
        self.sink.stop();
        println!("🌐 Streaming via ffmpeg: {}", url);

        let mut cmd = Command::new(ffmpeg);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        cmd.args(["-hide_banner", "-loglevel", "error"]);
        // Seek into the stream (fast input seek) when starting past the beginning.
        if start > Duration::ZERO {
            cmd.arg("-ss").arg(format!("{:.3}", start.as_secs_f64()));
        }
        cmd.arg("-i")
            .arg(url)
            // Decode to 44.1 kHz stereo signed-16 PCM on stdout.
            .args(["-vn", "-ar", "44100", "-ac", "2", "-f", "s16le", "pipe:1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                println!("❌ Failed to start ffmpeg: {:?}", e);
                return None;
            }
        };
        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            return None;
        };

        let source = FfmpegSource {
            child,
            stdout: BufReader::with_capacity(64 * 1024, stdout),
            sample_rate: 44_100,
            channels: 2,
        };
        self.sink.append(Tap::new(source, self.analysis.clone()));
        self.sink.play();
        duration_hint
    }

    /// Pauses playback (position is preserved).
    pub fn pause(&self) {
        self.sink.pause();
    }

    /// Resumes playback after a pause.
    pub fn resume(&self) {
        self.sink.play();
    }

    /// Sets output volume in the range `0.0..=1.0`.
    pub fn set_volume(&self, volume: f32) {
        self.sink.set_volume(volume);
    }

    /// Seeks `path` to `position` by decoding and discarding leading samples.
    ///
    /// The heavy decode loop runs on its own OS thread so the GUI stays
    /// responsive. The sink is stopped immediately for instant feedback, then
    /// the background thread re-appends the source positioned at `position` —
    /// but only if no newer transport operation has happened in the meantime
    /// (tracked via [`Self::current_operation_id`]). A superseded thread exits
    /// without touching the sink.
    pub fn seek(&self, path: &str, position: Duration) {
        // Claim a unique id for this seek. `fetch_add` returns the previous
        // value, so our id is that + 1.
        let op_id = self.current_operation_id.fetch_add(1, Ordering::SeqCst) + 1;

        // Stop the old audio right away so the player reacts instantly.
        self.sink.stop();

        // Clone the Arcs so the background thread can use them safely.
        let sink_clone = Arc::clone(&self.sink);
        let id_clone = Arc::clone(&self.current_operation_id);
        let analysis = self.analysis.clone();
        let path_clone = path.to_string();

        thread::spawn(move || {
            let Ok(file) = File::open(&path_clone) else {
                return;
            };
            let Ok(mut source) = Decoder::new(BufReader::new(file)) else {
                return;
            };

            let sample_rate = source.sample_rate();
            let channels = source.channels();
            let secs = position.as_secs_f32();
            let samples_to_skip = (secs * sample_rate as f32 * channels as f32) as usize;

            // Discard samples up to the target position. This runs in parallel
            // and does not block the UI thread.
            for _ in 0..samples_to_skip {
                let _ = source.next();
            }

            // Only start playing if this is still the most recent operation.
            // If the user seeked again, `id_clone` will have advanced and this
            // now-stale thread simply returns.
            if id_clone.load(Ordering::SeqCst) == op_id {
                sink_clone.append(Tap::new(source, analysis));
                sink_clone.play();
            }
        });
    }
}

/// A rodio [`Source`] that reads raw `s16le` PCM from a running `ffmpeg`
/// process's stdout — the audio for a streamed YouTube track.
///
/// ffmpeg fetches and transcodes the remote stream on demand, so samples are
/// produced roughly as fast as they are consumed and nothing is fully
/// downloaded. Dropping the source (when the next track starts) kills ffmpeg.
struct FfmpegSource {
    child: Child,
    stdout: BufReader<ChildStdout>,
    sample_rate: u32,
    channels: u16,
}

impl Iterator for FfmpegSource {
    type Item = i16;

    fn next(&mut self) -> Option<i16> {
        let mut bytes = [0u8; 2];
        // A short read at end-of-stream (ffmpeg exited / pipe closed) ends playback.
        match self.stdout.read_exact(&mut bytes) {
            Ok(()) => Some(i16::from_le_bytes(bytes)),
            Err(_) => None,
        }
    }
}

impl Source for FfmpegSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

impl Drop for FfmpegSource {
    fn drop(&mut self) {
        // Stop the transcoder as soon as this track is replaced.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
