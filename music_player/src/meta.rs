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

/// Computes a dark, muted background color from cover pixels (the average
/// color scaled down toward black) for the Now Playing screen.
fn average_bg_color(rgba: &image::RgbaImage) -> egui::Color32 {
    // Sample every few pixels — averaging all of them is unnecessary and a
    // visible cost when loading a large library.
    let (mut r, mut g, mut b, mut count) = (0u64, 0u64, 0u64, 0u64);
    for px in rgba.pixels().step_by(7) {
        r += px[0] as u64;
        g += px[1] as u64;
        b += px[2] as u64;
        count += 1;
    }
    if count == 0 {
        return egui::Color32::from_rgb(73, 73, 30);
    }
    // Darken so the background stays readable behind white lyrics text.
    let darken = 0.40;
    let scale = |sum: u64| ((sum / count) as f32 * darken) as u8;
    egui::Color32::from_rgb(scale(r), scale(g), scale(b))
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

/// Reads title, artist and cover art for the track at `path`.
///
/// This never fails: missing tags or an undecodable cover simply yield the
/// file-name title and `None` for the optional fields. `ctx` is needed to
/// upload the decoded cover as a texture.
pub fn read_track_meta(ctx: &egui::Context, path: &str) -> TrackMeta {
    use id3::TagLike;

    // Default title: the file name without its extension.
    let fallback_title = std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    let mut title = fallback_title;
    let mut artist: Option<String> = None;
    let mut cover: Option<egui::TextureHandle> = None;
    let mut bg: Option<egui::Color32> = None;

    if let Ok(tag) = id3::Tag::read_from_path(path) {
        if let Some(t) = tag.title() {
            if !t.trim().is_empty() {
                title = t.trim().to_string();
            }
        }
        if let Some(a) = tag.artist() {
            if !a.trim().is_empty() {
                artist = Some(a.trim().to_string());
            }
        }

        // Use the first embedded picture as the cover, scaled to 256x256 and
        // uploaded as a texture. Decode failures leave `cover` as `None`.
        //
        // 256px is enough for the largest place a cover is shown (the 240px
        // playlist header) while using ~30% less VRAM than 300px. `Triangle`
        // (bilinear) resizing is several times cheaper than `Lanczos3` and
        // visually indistinguishable at these sizes — this is the main cost of
        // loading a large library, so it matters most on weak machines.
        if let Some(pic) = tag.pictures().next() {
            if let Ok(img) = image::load_from_memory(&pic.data) {
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
