//! Escaping writes directly to the sink, without allocating expanded cells.
use super::*;
use std::{
    borrow::Cow,
    fmt::{self, Write},
};

/// A borrowed export description. Rendering never writes to audio files.
pub enum Format<'a> {
    M3u(&'a [PlaylistTrack]),
    Cue(&'a [CueTrack]),
    Csv(&'a [TrackFile]),
    Html(&'a [TrackFile]),
    Xml(&'a [TrackFile]),
    Report(&'a [TrackFile], &'a Mask),
}
/// The sink refused an output because its byte budget or allocation failed.
#[derive(Debug, thiserror::Error)]
#[error("Export exceeds its output budget or available memory")]
pub struct OutputLimit;

struct BoundedText {
    text: String,
    limit: usize,
}
impl Write for BoundedText {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let required = self.text.len().checked_add(value.len()).ok_or(fmt::Error)?;
        if required > self.limit {
            return Err(fmt::Error);
        }
        if required > self.text.capacity() {
            let target = required
                .max(self.text.capacity().saturating_mul(2).max(4096))
                .min(self.limit);
            self.text
                .try_reserve_exact(target - self.text.len())
                .map_err(|_| fmt::Error)?;
        }
        self.text.push_str(value);
        Ok(())
    }
}
impl Format<'_> {
    /// Render at most `limit` UTF-8 bytes, including headers and escaping.
    /// Nothing is returned on failure, so callers cannot publish a partial file.
    pub fn render_bounded(&self, limit: usize) -> Result<String, OutputLimit> {
        let mut sink = BoundedText {
            text: String::new(),
            limit,
        };
        self.write_to(&mut sink).map_err(|_| OutputLimit)?;
        Ok(sink.text)
    }
    /// Stream to an arbitrary text sink, stopping on its first error.
    pub fn write_to(&self, out: &mut dyn Write) -> fmt::Result {
        match self {
            Self::M3u(tracks) => write_m3u(out, tracks),
            Self::Cue(tracks) => write_cue(out, tracks),
            Self::Csv(tracks) => write_csv(out, tracks),
            Self::Html(tracks) => write_html(out, tracks),
            Self::Xml(tracks) => write_xml(out, tracks),
            Self::Report(tracks, mask) => write_report(out, tracks, mask),
        }
    }
}

// Write unchanged runs as slices. Escaping a large value never creates a second
// large string, and the bounded sink stops even in the middle of a cell.
fn escaped(
    out: &mut dyn Write,
    value: &str,
    replacement: impl Fn(char) -> Option<&'static str>,
) -> fmt::Result {
    let mut start = 0;
    for (i, c) in value.char_indices() {
        if let Some(text) = replacement(c) {
            out.write_str(&value[start..i])?;
            out.write_str(text)?;
            start = i + c.len_utf8();
        }
    }
    out.write_str(&value[start..])
}
fn csv_cell(out: &mut dyn Write, value: &str) -> fmt::Result {
    let formula = value.starts_with(['=', '+', '-', '@', '\t', '\r']);
    let quoted = formula || value.contains([',', '"', '\n', '\r']);
    if quoted {
        out.write_char('"')?;
    }
    if formula {
        out.write_char('\'')?;
    }
    escaped(out, value, |c| (c == '"').then_some("\"\""))?;
    if quoted {
        out.write_char('"')?;
    }
    Ok(())
}
fn html_cell(out: &mut dyn Write, value: &str) -> fmt::Result {
    escaped(out, value, |c| match c {
        '&' => Some("&amp;"),
        '<' => Some("&lt;"),
        '>' => Some("&gt;"),
        '"' => Some("&quot;"),
        '\'' => Some("&#39;"),
        _ => None,
    })
}
fn xml_cell(out: &mut dyn Write, value: &str) -> fmt::Result {
    escaped(out, value, |c| match c {
        '&' => Some("&amp;"),
        '<' => Some("&lt;"),
        '>' => Some("&gt;"),
        // XML readers normalize literal CR and CRLF sequences to LF.
        '\r' => Some("&#13;"),
        '\t' | '\n' => None,
        '\u{FFFE}' | '\u{FFFF}' => Some(""),
        c if c < ' ' => Some(""),
        _ => None,
    })
}
fn cue_cell(out: &mut dyn Write, value: &str) -> fmt::Result {
    out.write_char('"')?;
    escaped(out, value, |c| (c == '"').then_some("'"))?;
    out.write_char('"')
}
fn column<'a>(track: &'a TrackFile, name: &str, field: &Option<TagField>) -> Cow<'a, str> {
    match field {
        Some(field) => Cow::Borrowed(
            track
                .tags
                .get(field)
                .map(String::as_str)
                .unwrap_or_default(),
        ),
        None if name == "File" => track
            .path
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default(),
        None => track.path.to_string_lossy(),
    }
}
pub(super) fn write_m3u(out: &mut dyn Write, tracks: &[PlaylistTrack]) -> fmt::Result {
    out.write_str("#EXTM3U\n")?;
    for track in tracks {
        write!(out, "#EXTINF:{},", track.duration_secs)?;
        if !track.artist.is_empty() {
            write!(out, "{} - ", track.artist)?;
        }
        writeln!(out, "{}\n{}", track.title, track.path)?;
    }
    Ok(())
}
pub(super) fn write_cue(out: &mut dyn Write, tracks: &[CueTrack]) -> fmt::Result {
    if let Some(first) = tracks.first() {
        for (command, field) in [("REM GENRE", TagField::Genre), ("REM DATE", TagField::Year)] {
            if let Some(value) = first.tags.get(&field).filter(|v| !v.is_empty()) {
                write!(out, "{command} ")?;
                cue_cell(out, value)?;
                out.write_char('\n')?;
            }
        }
        for (command, value) in [
            (
                "PERFORMER",
                first
                    .tags
                    .get(&TagField::AlbumArtist)
                    .or_else(|| first.tags.get(&TagField::Artist)),
            ),
            ("TITLE", first.tags.get(&TagField::Album)),
        ] {
            if let Some(value) = value.filter(|v| !v.is_empty()) {
                write!(out, "{command} ")?;
                cue_cell(out, value)?;
                out.write_char('\n')?;
            }
        }
    }
    for (index, track) in tracks.iter().enumerate() {
        out.write_str("FILE ")?;
        cue_cell(out, &track.path)?;
        writeln!(
            out,
            " {}\n  TRACK {:02} AUDIO",
            cue_file_type(&track.path),
            index + 1
        )?;
        let fallback;
        let title = match track.tags.get(&TagField::Title).filter(|v| !v.is_empty()) {
            Some(title) => title.as_str(),
            None => {
                fallback = cue_file_stem(&track.path);
                &fallback
            }
        };
        out.write_str("    TITLE ")?;
        cue_cell(out, title)?;
        out.write_char('\n')?;
        if let Some(value) = track.tags.get(&TagField::Artist).filter(|v| !v.is_empty()) {
            out.write_str("    PERFORMER ")?;
            cue_cell(out, value)?;
            out.write_char('\n')?;
        }
        if let Some(code) = track.tags.get(&TagField::Isrc).and_then(|v| cue_isrc(v)) {
            writeln!(out, "    ISRC {code}")?;
        }
        out.write_str("    INDEX 01 00:00:00\n")?;
    }
    Ok(())
}
pub(super) fn write_csv(out: &mut dyn Write, tracks: &[TrackFile]) -> fmt::Result {
    for (i, (name, _)) in CSV_COLUMNS.iter().enumerate() {
        if i > 0 {
            out.write_char(',')?;
        }
        csv_cell(out, name)?;
    }
    out.write_str("\r\n")?;
    for track in tracks {
        for (i, (name, field)) in CSV_COLUMNS.iter().enumerate() {
            if i > 0 {
                out.write_char(',')?;
            }
            csv_cell(out, &column(track, name, field))?;
        }
        out.write_str("\r\n")?;
    }
    Ok(())
}
pub(super) fn write_html(out: &mut dyn Write, tracks: &[TrackFile]) -> fmt::Result {
    out.write_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n")?;
    out.write_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n")?;
    out.write_str("<title>Tagnetic export</title>\n<style>\n")?;
    out.write_str(
        "body{font:14px/1.5 system-ui,sans-serif;margin:2rem;color:#1a1a1a}\
         h1{font-size:1.2rem;margin:0 0 .25rem}\
         p.count{color:#666;margin:0 0 1rem}\
         table{border-collapse:collapse;width:100%;font-size:13px}\
         th,td{border:1px solid #ddd;padding:4px 8px;text-align:left;vertical-align:top}\
         th{background:#f4f4f5;position:sticky;top:0}\
         tr:nth-child(even) td{background:#fafafa}\n",
    )?;
    out.write_str("</style>\n</head>\n<body>\n<h1>Tagnetic export</h1>\n")?;
    writeln!(
        out,
        "<p class=\"count\">{} track{}</p>",
        tracks.len(),
        if tracks.len() == 1 { "" } else { "s" }
    )?;
    out.write_str("<table>\n<thead>\n<tr>")?;
    for (name, _) in CSV_COLUMNS {
        out.write_str("<th>")?;
        html_cell(out, name)?;
        out.write_str("</th>")?;
    }
    out.write_str("</tr>\n</thead>\n<tbody>\n")?;
    for track in tracks {
        out.write_str("<tr>")?;
        for (name, field) in &CSV_COLUMNS {
            out.write_str("<td>")?;
            html_cell(out, &column(track, name, field))?;
            out.write_str("</td>")?;
        }
        out.write_str("</tr>\n")?;
    }
    out.write_str("</tbody>\n</table>\n</body>\n</html>\n")
}
pub(super) fn write_xml(out: &mut dyn Write, tracks: &[TrackFile]) -> fmt::Result {
    out.write_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n")?;
    writeln!(out, "<library count=\"{}\">", tracks.len())?;
    for track in tracks {
        out.write_str("  <track>\n")?;
        for (name, field) in &CSV_COLUMNS {
            let value = column(track, name, field);
            if value.is_empty() && field.is_some() {
                continue;
            }
            let element = xml_element_name(name);
            write!(out, "    <{element}>")?;
            xml_cell(out, &value)?;
            writeln!(out, "</{element}>")?;
        }
        out.write_str("  </track>\n")?;
    }
    out.write_str("</library>\n")
}
pub(super) fn write_report(out: &mut dyn Write, tracks: &[TrackFile], mask: &Mask) -> fmt::Result {
    for track in tracks {
        let file = FileContext::read(mask, track);
        let Ok(tags) = lenient_mask_tags(&track.tags, mask) else {
            continue;
        };
        if let Ok(line) = mask.render_with(&tags, &file) {
            out.write_str(&line)?;
            out.write_char('\n')?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_format_counts_headers_unicode_and_escaping_before_returning_output() {
        let tracks = [TrackFile {
            path: "Folder/日本語 & <file>.mp3".into(),
            format: crate::model::AudioFormat::Mp3,
            tags: TagMap::from([
                (
                    TagField::Title,
                    "=HYPERLINK(\"x\")\n日 & < > ' \u{0001}".into(),
                ),
                (TagField::Artist, "Artist".into()),
            ]),
        }];
        let cues = [CueTrack {
            path: "日\".mp3".into(),
            tags: tracks[0].tags.clone(),
        }];
        let playlists = [PlaylistTrack {
            path: "日.mp3".into(),
            title: "Title".into(),
            artist: "Artist".into(),
            duration_secs: 42,
        }];
        let mask = Mask::parse("%artist% - %title%").unwrap();
        for format in [
            Format::Csv(&tracks),
            Format::Html(&tracks),
            Format::Xml(&tracks),
            Format::Report(&tracks, &mask),
            Format::Cue(&cues),
            Format::M3u(&playlists),
        ] {
            let mut expected = String::new();
            format.write_to(&mut expected).unwrap();
            assert!(!expected.is_empty());
            assert_eq!(format.render_bounded(expected.len()).unwrap(), expected);
            assert!(format.render_bounded(expected.len() - 1).is_err());
            assert!(format.render_bounded(0).is_err());
        }
    }
    #[test]
    fn expanded_cells_stop_inside_the_cell_and_leave_bounded_scratch() {
        let text = "\"&<日".repeat(100_000);
        let mut sink = BoundedText {
            text: String::new(),
            limit: 512,
        };
        assert!(html_cell(&mut sink, &text).is_err());
        assert!(sink.text.len() <= 512);
        assert!(sink.text.capacity() <= 512);
        assert!(sink.text.is_char_boundary(sink.text.len()));
        let mut sink = BoundedText {
            text: String::new(),
            limit: 512,
        };
        assert!(csv_cell(&mut sink, &text).is_err());
        assert!(sink.text.len() <= 512);
        assert!(sink.text.capacity() <= 512);
    }
    #[test]
    fn a_sink_failure_stops_at_the_first_refused_write() {
        struct Refuse(usize);
        impl Write for Refuse {
            fn write_str(&mut self, _: &str) -> fmt::Result {
                self.0 += 1;
                Err(fmt::Error)
            }
        }
        let tracks = [PlaylistTrack {
            path: "x.mp3".into(),
            title: "Title".into(),
            artist: "Artist".into(),
            duration_secs: 42,
        }];
        let mut sink = Refuse(0);
        assert!(Format::M3u(&tracks).write_to(&mut sink).is_err());
        assert_eq!(sink.0, 1);
    }
}

#[cfg(test)]
mod mask_memory_tests {
    use super::*;
    #[test]
    fn reports_prepare_only_referenced_tags_and_implicit_side_inputs() {
        let tags = TagMap::from([
            (TagField::Artist, "Artist".into()),
            (TagField::MediaType, "Vinyl".into()),
            (TagField::DiscNumber, "2".into()),
            (
                TagField::Custom("lyrics".into()),
                "x".repeat(2 * 1024 * 1024),
            ),
        ]);
        let mask = Mask::parse("%side% %artist%").unwrap();
        let prepared = lenient_mask_tags(&tags, &mask).unwrap();
        assert!(!prepared.contains_key(&TagField::Custom("lyrics".into())));
        assert_eq!(
            mask.render(&prepared).unwrap(),
            mask.render(&lenient_tags(&tags)).unwrap()
        );
        let mask = Mask::parse("%custom:lyrics%").unwrap();
        assert!(lenient_mask_tags(&tags, &mask).is_err());
    }
}
