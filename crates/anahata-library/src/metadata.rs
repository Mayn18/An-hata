use std::path::Path;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::probe::Probe;
use lofty::tag::Accessor;

use crate::error::{LibraryError, LibraryResult};

#[derive(Debug, Clone, Default)]
pub struct TrackMetadata {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub track_no: Option<u32>,
    pub track_total: Option<u32>,
    pub disc_no: Option<u32>,
    pub disc_total: Option<u32>,
    pub year: Option<u32>,
    pub duration_ms: u64,
    pub bitrate: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
    pub format: String,
    pub codec: Option<String>,
}

pub fn extract_metadata<P: AsRef<Path>>(path: P) -> LibraryResult<TrackMetadata> {
    let path_ref = path.as_ref();
    let tagged_file = Probe::open(path_ref)
        .map_err(|e| LibraryError::Metadata(format!("Probe open error: {e}")))?
        .read()
        .map_err(|e| LibraryError::Metadata(format!("Tag read error: {e}")))?;

    let properties = tagged_file.properties();
    let duration_ms = properties.duration().as_millis() as u64;
    let bitrate = properties.audio_bitrate();
    let sample_rate = properties.sample_rate();
    let channels = properties.channels();

    let format = path_ref
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_lowercase();

    let tag = tagged_file.primary_tag().or_else(|| tagged_file.first_tag());

    let default_title = path_ref
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown Title")
        .to_string();

    let (title, artist, album, album_artist, genre, track_no, track_total, disc_no, disc_total, year) =
        if let Some(t) = tag {
            (
                t.title().map(|s| s.to_string()).unwrap_or(default_title),
                t.artist().map(|s| s.to_string()),
                t.album().map(|s| s.to_string()),
                t.get_string(&lofty::tag::ItemKey::AlbumArtist).map(|s| s.to_string()),
                t.genre().map(|s| s.to_string()),
                t.track(),
                t.track_total(),
                t.disk(),
                t.disk_total(),
                t.year(),
            )
        } else {
            (default_title, None, None, None, None, None, None, None, None, None)
        };

    Ok(TrackMetadata {
        title,
        artist,
        album,
        album_artist,
        genre,
        track_no,
        track_total,
        disc_no,
        disc_total,
        year,
        duration_ms,
        bitrate,
        sample_rate,
        channels,
        format,
        codec: None,
    })
}
