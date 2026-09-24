//! Per-track metadata: title, artist and embedded cover art.
//!
//! Reading ID3 tags and decoding cover images is relatively slow, so this work
//! happens on a background thread (see [`crate::app::App::new`]). Results travel
//! back to the UI thread as [`LoaderMsg`] values over an `mpsc` channel.

use crate::scanner::Playlist;
use eframe::egui;

/// Display metadata for a single track, ready to be drawn by the UI.
pub struct TrackMeta {
    /// Track title (falls back to the file stem when no tag is present).
    pub title: String,
    /// Artist name, if known.
    pub artist: Option<String>,
    /// Decoded cover art uploaded as a GPU texture, if the file embeds one.
    pub cover: Option<egui::TextureHandle>,
    /// A dark, muted color derived from the cover, used as the adaptive
    /// background of the Now Playing screen. `None` when there is no cover.
    pub bg: Option<egui::Color32>,
}

/// RGB (each 0..1) → HSV (`h` in 0..360, `s`/`v` in 0..1).
fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max <= 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

/// Computes a dark background color that visually matches a cover, for the Now
/// Playing screen and the window tint.
///
/// A flat average of every pixel turns muddy gray-brown for colorful art, so
/// instead the cover's **dominant vibrant hue** is extracted (pixels binned by
/// hue, weighted by saturation × brightness) and rendered as a consistent dark,
/// saturated tone. Falls back to the darkened average for near-grayscale art.
pub(crate) fn average_bg_color(rgba: &image::RgbaImage) -> egui::Color32 {
    // Weighted RGB accumulators per 30° hue bin, plus a plain average fallback.
    let mut bins: [(f32, f32, f32, f32); 12] = [(0.0, 0.0, 0.0, 0.0); 12];
    let (mut ar, mut ag, mut ab, mut count) = (0u64, 0u64, 0u64, 0u64);

    for px in rgba.pixels().step_by(5) {
        ar += px[0] as u64;
        ag += px[1] as u64;
        ab += px[2] as u64;
        count += 1;

        let (r, g, b) = (px[0] as f32 / 255.0, px[1] as f32 / 255.0, px[2] as f32 / 255.0);
        let (h, s, v) = rgb_to_hsv(r, g, b);
        // Ignore near-gray and near-black/white pixels — they carry no useful hue.
        if s < 0.18 || v < 0.15 || v > 0.97 {
            continue;
        }
        let w = s * v; // favor clean, bright, saturated color
        let bin = ((h / 30.0) as usize).min(11);
        bins[bin].0 += r * w;
        bins[bin].1 += g * w;
        bins[bin].2 += b * w;
        bins[bin].3 += w;
    }

    if count == 0 {
        return egui::Color32::from_rgb(40, 40, 46);
    }

    // The heaviest hue bin, if any pixels were vibrant enough.
    let best = bins.iter().copied().max_by(|a, b| a.3.total_cmp(&b.3));
    if let Some((sr, sg, sb, w)) = best {
        if w > 0.0 {
            let (h, s, _) = rgb_to_hsv(sr / w, sg / w, sb / w);
            // Render as a consistent dark, moderately-saturated tone.
            return crate::theme::hsv(h, s.clamp(0.42, 0.8), 0.34);
        }
    }

    // Grayscale art: darkened plain average.
    let darken = 0.40;
    let scale = |sum: u64| ((sum / count) as f32 * darken) as u8;
    egui::Color32::from_rgb(scale(ar), scale(ag), scale(ab))
}

/// Normalizes text to Unicode NFC (canonical composition).
///
/// Some tags store a base letter plus a separate combining mark (e.g. "и" + a
/// combining breve). NFC fuses them into the single precomposed character
/// ("й") that our bundled font can actually render.
pub fn nfc(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfc().collect()
}

/// Everything read from a file's tags, before any texture upload. Kept separate
/// from [`TrackMeta`] so it can be produced on a plain worker thread that has no
/// access to the egui context.
#[derive(Default)]
pub struct RawMeta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub year: Option<i32>,
    pub genre: Option<String>,
    pub track_number: Option<u32>,
    pub duration: Option<std::time::Duration>,
    /// Raw bytes of the embedded cover art, still encoded (JPEG/PNG/…).
    pub cover_bytes: Option<Vec<u8>>,
}

/// Reads *all* available metadata for `path`, for **any** audio format.
///
/// Backed by `lofty`, which understands MP3/ID3, FLAC, MP4/M4A, Ogg Vorbis,
/// Opus, WAV, AIFF, WavPack, APE and more — so title, artist, album, cover art
/// and duration are recovered from every file the player can import, not just
/// MP3s. Never fails: an unreadable or tagless file simply yields an empty
/// [`RawMeta`] and the caller falls back to the file name.
pub fn read_raw_meta(path: &str) -> RawMeta {
    use lofty::file::{AudioFile, TaggedFileExt};
    use lofty::prelude::Accessor;

    let mut out = RawMeta::default();

    let Ok(tagged) = lofty::read_from_path(path) else {
        return out;
    };

    // Audio properties (duration, bitrate…) live outside the tags and are
    // available even for files that carry no tags at all.
    let dur = tagged.properties().duration();
    if !dur.is_zero() {
        out.duration = Some(dur);
    }

    // Prefer the format's primary tag (e.g. ID3v2 over ID3v1); fall back to
    // whichever tag exists.
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    if let Some(tag) = tag {
        let clean = |s: Option<std::borrow::Cow<str>>| {
            s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
        };
        out.title = clean(tag.title());
        out.artist = clean(tag.artist());
        out.album = clean(tag.album());
        out.genre = clean(tag.genre());
        // `date()` yields a full timestamp (year + optional month/day) across
        // every format's date field; we only surface the year.
        out.year = tag.date().map(|d| d.year as i32);
        out.track_number = tag.track();

        // First embedded picture, preferring an explicit front cover.
        let pics = tag.pictures();
        let pic = pics
            .iter()
            .find(|p| p.pic_type() == lofty::picture::PictureType::CoverFront)
            .or_else(|| pics.first());
        if let Some(pic) = pic {
            out.cover_bytes = Some(pic.data().to_vec());
        }
    }

    out
}

/// Reads title, artist, album, cover art and other tags for the track at
/// `path`, for any audio format.
///
/// This never fails: missing tags or an undecodable cover simply yield the
/// file-name title and `None` for the optional fields. `ctx` is needed to
/// upload the decoded cover as a texture.
pub fn read_track_meta(ctx: &egui::Context, path: &str) -> TrackMeta {
    // Default title: the file name without its extension.
    let fallback_title = std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    let raw = read_raw_meta(path);

    let title = raw.title.unwrap_or(fallback_title);
    let artist = raw.artist;

    let mut cover: Option<egui::TextureHandle> = None;
    let mut bg: Option<egui::Color32> = None;

    // Decode the embedded cover, scaled to 256x256 and uploaded as a texture.
    // Decode failures leave `cover` as `None`.
    //
    // 256px is enough for the largest place a cover is shown (the 240px
    // playlist header) while using ~30% less VRAM than 300px. `Triangle`
    // (bilinear) resizing is several times cheaper than `Lanczos3` and
    // visually indistinguishable at these sizes — this is the main cost of
    // loading a large library, so it matters most on weak machines.
    if let Some(bytes) = &raw.cover_bytes {
        if let Ok(img) = image::load_from_memory(bytes) {
            let img = img.resize_to_fill(256, 256, image::imageops::FilterType::Triangle);
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            // Adaptive Now Playing background, derived from the pixels.
            bg = Some(average_bg_color(&rgba));
            let color =
                egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba.as_raw());
            cover = Some(ctx.load_texture(
                format!("cover:{}", path),
                color,
                egui::TextureOptions::LINEAR,
            ));
        }
    }

    // Normalize text so combining marks render correctly (see `nfc`).
    let title = nfc(&title);
    let artist = artist.map(|a| nfc(&a));

    TrackMeta { title, artist, cover, bg }
}

/// Messages sent from background loader threads to the UI thread.
pub enum LoaderMsg {
    /// The initial set of scanned playlists (sent once, early in startup).
    Playlists(Vec<Playlist>),
    /// Metadata for a single track, keyed by its file path. Sent one at a time
    /// as covers/tags finish loading, so the UI can fill cards in as they go.
    Meta(String, TrackMeta),
}

