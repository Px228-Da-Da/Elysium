//! Signal-processing core for the Lab window.
//!
//! Three independent pieces live here, none of which know anything about the
//! UI:
//!
//! * [`Analysis`] — a lock-protected ring buffer of the most recent stereo
//!   samples. The audio thread writes into it (via [`Tap`]); the Lab UI reads
//!   snapshots out of it each frame to drive the live scopes/meters.
//! * [`Tap`] — a `rodio::Source` wrapper that sits between the decoder and the
//!   sink and copies every sample it passes through into an [`Analysis`].
//! * [`fft`] / [`compute_waveform`] — pure math helpers (an in-place radix-2
//!   FFT and the off-thread "whole track" peak waveform).

use rodio::Source;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Number of stereo frames the ring buffer keeps. Power of two so the write
/// index can wrap with a cheap bit-mask. ~0.74 s at 44.1 kHz — plenty for the
/// FFT window and the scopes.
pub const RING: usize = 1 << 15;
const RING_MASK: usize = RING - 1;

/// Rolling history of the audio currently coming out of the sink.
///
/// Shared between the audio thread (writer) and the UI thread (reader) behind a
/// `Mutex`. The writer only ever appends in small batches, so the lock is held
/// for a few microseconds at a time and the UI never blocks meaningfully.
pub struct Analysis {
    pub sample_rate: u32,
    pub channels: u16,
    left: Box<[f32]>,
    right: Box<[f32]>,
    /// Index the next frame will be written to.
    pos: usize,
    /// Total frames ever written. Lets a reader notice when nothing new has
    /// arrived (e.g. while paused).
    pub written: u64,
}

impl Analysis {
    fn new() -> Self {
        Self {
            sample_rate: 44100,
            channels: 2,
            left: vec![0.0; RING].into_boxed_slice(),
            right: vec![0.0; RING].into_boxed_slice(),
            pos: 0,
            written: 0,
        }
    }

    #[inline]
    fn push(&mut self, l: f32, r: f32) {
        self.left[self.pos] = l;
        self.right[self.pos] = r;
        self.pos = (self.pos + 1) & RING_MASK;
        self.written = self.written.wrapping_add(1);
    }

    /// Copies the most recent `n` frames (chronological order, oldest first)
    /// into `out_l` / `out_r`, which are cleared first.
    pub fn recent(&self, n: usize, out_l: &mut Vec<f32>, out_r: &mut Vec<f32>) {
        let n = n.min(RING);
        out_l.clear();
        out_r.clear();
        let start = (self.pos + RING - n) & RING_MASK;
        for i in 0..n {
            let idx = (start + i) & RING_MASK;
            out_l.push(self.left[idx]);
            out_r.push(self.right[idx]);
        }
    }
}

/// Handle shared between the player and the Lab UI.
pub type SharedAnalysis = Arc<Mutex<Analysis>>;

/// Creates a fresh, silent analysis buffer.
pub fn new_shared() -> SharedAnalysis {
    Arc::new(Mutex::new(Analysis::new()))
}

/// A pass-through `Source` that mirrors every sample into a [`SharedAnalysis`].
///
/// Inserted between the decoder and the sink. It yields exactly the samples it
/// receives (so playback is unaffected) while batching frames and flushing them
/// into the shared buffer ~once per 512 frames to keep lock traffic low.
pub struct Tap<S> {
    inner: S,
    shared: SharedAnalysis,
    channels: u16,
    /// Which channel of the current frame we are on.
    ch: u16,
    frame: [f32; 2],
    batch_l: Vec<f32>,
    batch_r: Vec<f32>,
}

impl<S> Tap<S>
where
    S: Source<Item = i16>,
{
    /// Wraps `inner`, recording its format into `shared` up front.
    pub fn new(inner: S, shared: SharedAnalysis) -> Self {
        let channels = inner.channels();
        let sample_rate = inner.sample_rate();
        if let Ok(mut a) = shared.lock() {
            a.sample_rate = sample_rate;
            a.channels = channels;
        }
        Self {
            inner,
            shared,
            channels: channels.max(1),
            ch: 0,
            frame: [0.0; 2],
            batch_l: Vec::with_capacity(1024),
            batch_r: Vec::with_capacity(1024),
        }
    }

    fn flush(&mut self) {
        if self.batch_l.is_empty() {
            return;
        }
        if let Ok(mut a) = self.shared.lock() {
            for i in 0..self.batch_l.len() {
                a.push(self.batch_l[i], self.batch_r[i]);
            }
        }
        self.batch_l.clear();
        self.batch_r.clear();
    }
}

impl<S> Iterator for Tap<S>
where
    S: Source<Item = i16>,
{
    type Item = i16;

    #[inline]
    fn next(&mut self) -> Option<i16> {
        let sample = self.inner.next();
        match sample {
            Some(v) => {
                let f = v as f32 / 32768.0;
                if self.ch < 2 {
                    self.frame[self.ch as usize] = f;
                }
                self.ch += 1;
                if self.ch >= self.channels {
                    let l = self.frame[0];
                    let r = if self.channels >= 2 { self.frame[1] } else { l };
                    self.batch_l.push(l);
                    self.batch_r.push(r);
                    self.ch = 0;
                    if self.batch_l.len() >= 512 {
                        self.flush();
                    }
                }
            }
            None => self.flush(), // end of track: don't lose the tail
        }
        sample
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<S> Source for Tap<S>
where
    S: Source<Item = i16>,
{
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }
    fn channels(&self) -> u16 {
        self.inner.channels()
    }
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}

/// In-place iterative radix-2 Cooley–Tukey FFT.
///
/// `re`/`im` must have the same, power-of-two length. On return they hold the
/// transform in place. Written from scratch so the Lab needs no extra crates.
pub fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    debug_assert_eq!(n, im.len());
    if n < 2 {
        return;
    }

    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    // Butterflies, doubling the transform length each pass.
    let mut len = 2usize;
    while len <= n {
        let ang = -2.0 * std::f32::consts::PI / len as f32;
        let (wlen_re, wlen_im) = (ang.cos(), ang.sin());
        let half = len / 2;
        let mut i = 0usize;
        while i < n {
            let (mut wr, mut wi) = (1.0f32, 0.0f32);
            for k in 0..half {
                let a = i + k;
                let b = a + half;
                let tr = wr * re[b] - wi * im[b];
                let ti = wr * im[b] + wi * re[b];
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let nwr = wr * wlen_re - wi * wlen_im;
                wi = wr * wlen_im + wi * wlen_re;
                wr = nwr;
            }
            i += len;
        }
        len <<= 1;
    }
}

/// Pre-rendered "whole track" peak envelope shown by the waveform widget.
pub struct Waveform {
    /// Source track path — used to drop a result that finished after the user
    /// already moved on to a different track.
    pub path: String,
    /// One normalized (0..1) peak magnitude per horizontal bucket.
    pub peaks: Vec<f32>,
}

/// Shared slot the background decoder writes the finished [`Waveform`] into.
pub type SharedWaveform = Arc<Mutex<Option<Waveform>>>;

/// Decodes the entire file and reduces it to `buckets` peak values.
///
/// Meant to run on a background thread (it decodes the whole track). Returns
/// `None` if the file cannot be opened/decoded or is empty.
pub fn compute_waveform(path: &str, buckets: usize) -> Option<Waveform> {
    use rodio::Decoder;
    use std::fs::File;
    use std::io::BufReader;

    let file = File::open(path).ok()?;
    let decoder = Decoder::new(BufReader::new(file)).ok()?;

    // Stream the decode into coarse "mini-peak" blocks so we never hold the
    // whole track in memory (a 3-minute song is ~16M samples). Block size is in
    // raw interleaved samples; the exact channel layout is irrelevant for a
    // peak envelope, so we skip the per-frame channel bookkeeping.
    const BLOCK: usize = 2048;
    let mut minis: Vec<f32> = Vec::new();
    let mut cur = 0.0f32;
    let mut n = 0usize;
    for s in decoder {
        let a = (s as f32 / 32768.0).abs();
        if a > cur {
            cur = a;
        }
        n += 1;
        if n == BLOCK {
            minis.push(cur);
            cur = 0.0;
            n = 0;
        }
    }
    if n > 0 {
        minis.push(cur);
    }
    if minis.is_empty() {
        return None;
    }

    let buckets = buckets.max(1);
    let mut peaks = vec![0.0f32; buckets];
    let per = minis.len() as f32 / buckets as f32;
    for (i, peak) in peaks.iter_mut().enumerate() {
        let start = (i as f32 * per) as usize;
        let end = (((i + 1) as f32 * per) as usize).clamp(start + 1, minis.len());
        let mut m = 0.0f32;
        for &v in &minis[start..end] {
            if v > m {
                m = v;
            }
        }
        *peak = m;
    }

    // Normalize so the loudest part fills the widget height.
    let max = peaks.iter().cloned().fold(0.0f32, f32::max).max(1e-6);
    for p in peaks.iter_mut() {
        *p /= max;
    }

    Some(Waveform {
        path: path.to_string(),
        peaks,
    })
}
