//! Strict, bounded text conversion shared by native import and output tools.
//! Newlines are preserved. A codepage is explicit; host-dependent "ANSI" is
//! never guessed. UTF-16 output is written as code units, not via encoding_rs's
//! UTF-16 output encoding (which is UTF-8).
use encoding_rs::{DecoderResult, EncoderResult, Encoding};
use thiserror::Error;

const MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    Codepage(&'static Encoding),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TextError {
    #[error("Unknown or ambiguous text encoding; choose UTF-8, UTF-16LE, UTF-16BE or an explicit codepage")]
    Encoding,
    #[error("The byte-order marker conflicts with the selected encoding")]
    Bom,
    #[error("Input contains invalid encoded text")]
    Invalid,
    #[error("The selected encoding cannot represent character {0:?}")]
    Unmappable(char),
    #[error("Text exceeds the configured byte limit")]
    Limit,
    #[error("Text conversion was cancelled")]
    Cancelled,
}

impl TextEncoding {
    pub fn from_label(label: &str) -> Result<Self, TextError> {
        if label.len() > 64 {
            return Err(TextError::Encoding);
        }
        match label.trim().to_ascii_lowercase().as_str() {
            "utf-16le" => Ok(Self::Utf16Le),
            "utf-16be" => Ok(Self::Utf16Be),
            "ansi" | "utf-16" | "replacement" => Err(TextError::Encoding),
            label => {
                let encoding = Encoding::for_label(label.as_bytes()).ok_or(TextError::Encoding)?;
                if encoding == encoding_rs::UTF_8 {
                    Ok(Self::Utf8)
                } else if encoding == encoding_rs::UTF_16LE
                    || encoding == encoding_rs::UTF_16BE
                    || encoding == encoding_rs::REPLACEMENT
                {
                    Err(TextError::Encoding)
                } else {
                    Ok(Self::Codepage(encoding))
                }
            }
        }
    }

    fn encoding(self) -> &'static Encoding {
        match self {
            Self::Utf8 => encoding_rs::UTF_8,
            Self::Utf16Le => encoding_rs::UTF_16LE,
            Self::Utf16Be => encoding_rs::UTF_16BE,
            Self::Codepage(encoding) => encoding,
        }
    }

    fn bom(self) -> &'static [u8] {
        match self {
            Self::Utf8 => &[0xef, 0xbb, 0xbf],
            Self::Utf16Le => &[0xff, 0xfe],
            Self::Utf16Be => &[0xfe, 0xff],
            Self::Codepage(_) => &[],
        }
    }
}

fn append(output: &mut Vec<u8>, part: &[u8], limit: usize) -> Result<(), TextError> {
    let required = output
        .len()
        .checked_add(part.len())
        .filter(|size| *size <= limit)
        .ok_or(TextError::Limit)?;
    if required > output.capacity() {
        // Grow geometrically for incremental encoders, but never request an
        // allocation beyond the output limit. Reserve fallibly before copying.
        let target = required.max(output.capacity().saturating_mul(2)).min(limit);
        output
            .try_reserve_exact(target - output.len())
            .map_err(|_| TextError::Limit)?;
    }
    output.extend_from_slice(part);
    Ok(())
}

/// Decode with no replacement characters or BOM-driven encoding switch.
/// Matching leading BOMs are consumed; conflicting Unicode BOMs are rejected.
pub fn decode(
    bytes: &[u8],
    encoding: TextEncoding,
    max_output: usize,
) -> Result<String, TextError> {
    decode_with_cancel(bytes, encoding, max_output, || false)
}

/// Cancel between bounded decoding chunks; no partial text is returned.
pub fn decode_with_cancel(
    bytes: &[u8],
    encoding: TextEncoding,
    max_output: usize,
    cancelled: impl Fn() -> bool,
) -> Result<String, TextError> {
    if cancelled() {
        return Err(TextError::Cancelled);
    }
    if bytes.len() > MAX_BYTES {
        return Err(TextError::Limit);
    }
    let limit = max_output.min(MAX_BYTES);
    let bom = [
        TextEncoding::Utf8,
        TextEncoding::Utf16Le,
        TextEncoding::Utf16Be,
    ]
    .into_iter()
    .find(|candidate| bytes.starts_with(candidate.bom()));
    let bytes = if let Some(bom) = bom {
        if bom != encoding {
            return Err(TextError::Bom);
        }
        &bytes[bom.bom().len()..]
    } else {
        bytes
    };
    let mut decoder = encoding.encoding().new_decoder_without_bom_handling();
    let mut input = bytes;
    let mut output = Vec::new();
    loop {
        if cancelled() {
            return Err(TextError::Cancelled);
        }
        let mut chunk = [0u8; 4096];
        let (result, read, written) =
            decoder.decode_to_utf8_without_replacement(input, &mut chunk, true);
        append(&mut output, &chunk[..written], limit)?;
        input = &input[read..];
        match result {
            DecoderResult::InputEmpty => break,
            DecoderResult::OutputFull => (),
            DecoderResult::Malformed(..) => return Err(TextError::Invalid),
        }
    }
    if cancelled() {
        return Err(TextError::Cancelled);
    }
    String::from_utf8(output).map_err(|_| TextError::Invalid)
}

/// Encode without replacement/numeric entities. A BOM is allowed only for
/// explicit Unicode encodings and emitted exactly once at the beginning.
pub fn encode(
    text: &str,
    encoding: TextEncoding,
    bom: bool,
    max_output: usize,
) -> Result<Vec<u8>, TextError> {
    encode_with_cancel(text, encoding, bom, max_output, || false)
}

/// Cancel between bounded encoding chunks; no partial byte buffer is returned.
pub fn encode_with_cancel(
    text: &str,
    encoding: TextEncoding,
    bom: bool,
    max_output: usize,
    cancelled: impl Fn() -> bool,
) -> Result<Vec<u8>, TextError> {
    if cancelled() {
        return Err(TextError::Cancelled);
    }
    if text.len() > MAX_BYTES {
        return Err(TextError::Limit);
    }
    let limit = max_output.min(MAX_BYTES);
    let mut output = Vec::new();
    if bom {
        if matches!(encoding, TextEncoding::Codepage(_)) {
            return Err(TextError::Bom);
        }
        append(&mut output, encoding.bom(), limit)?;
    }
    match encoding {
        TextEncoding::Utf8 => append(&mut output, text.as_bytes(), limit)?,
        TextEncoding::Utf16Le | TextEncoding::Utf16Be => {
            for (index, unit) in text.encode_utf16().enumerate() {
                if index.is_multiple_of(1024) && cancelled() {
                    return Err(TextError::Cancelled);
                }
                let bytes = if encoding == TextEncoding::Utf16Le {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                };
                append(&mut output, &bytes, limit)?;
            }
        }
        TextEncoding::Codepage(encoding) => {
            // Do not let direct enum construction bypass the UTF-16 policy.
            if encoding == encoding_rs::UTF_16LE
                || encoding == encoding_rs::UTF_16BE
                || encoding == encoding_rs::REPLACEMENT
            {
                return Err(TextError::Encoding);
            }
            let mut encoder = encoding.new_encoder();
            let mut input = text;
            loop {
                if cancelled() {
                    return Err(TextError::Cancelled);
                }
                let mut chunk = [0u8; 4096];
                let (result, read, written) =
                    encoder.encode_from_utf8_without_replacement(input, &mut chunk, true);
                append(&mut output, &chunk[..written], limit)?;
                input = &input[read..];
                match result {
                    EncoderResult::InputEmpty => break,
                    EncoderResult::OutputFull => (),
                    EncoderResult::Unmappable(c) => return Err(TextError::Unmappable(c)),
                }
            }
        }
    }
    if cancelled() {
        return Err(TextError::Cancelled);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_during_conversion_never_returns_partial_output() {
        use std::cell::Cell;
        let text = "日本語😀".repeat(10000);
        let calls = Cell::new(0);
        let cancelled = || {
            calls.set(calls.get() + 1);
            calls.get() > 3
        };
        assert_eq!(
            encode_with_cancel(&text, TextEncoding::Utf16Le, true, 1_000_000, cancelled),
            Err(TextError::Cancelled)
        );
        let bytes = encode(&text, TextEncoding::Utf16Le, true, 1_000_000).unwrap();
        calls.set(0);
        assert_eq!(
            decode_with_cancel(&bytes, TextEncoding::Utf16Le, 1_000_000, cancelled),
            Err(TextError::Cancelled)
        );
    }
    #[test]
    fn unicode_outputs_have_exact_byte_order_and_one_optional_bom() {
        assert_eq!(
            encode("A😀", TextEncoding::Utf16Le, true, 100).unwrap(),
            [0xff, 0xfe, 0x41, 0, 0x3d, 0xd8, 0, 0xde]
        );
        assert_eq!(
            encode("A😀", TextEncoding::Utf16Be, true, 100).unwrap(),
            [0xfe, 0xff, 0, 0x41, 0xd8, 0x3d, 0xde, 0]
        );
        for encoding in [
            TextEncoding::Utf8,
            TextEncoding::Utf16Le,
            TextEncoding::Utf16Be,
        ] {
            for bom in [true, false] {
                let bytes = encode("Björk\r\n日本語 😀\n", encoding, bom, 100).unwrap();
                assert_eq!(
                    decode(&bytes, encoding, 100).unwrap(),
                    "Björk\r\n日本語 😀\n"
                );
            }
        }
    }
    #[test]
    fn strict_codepage_roundtrip_never_substitutes_numeric_entities() {
        let encoding = TextEncoding::from_label("windows-1252").unwrap();
        assert_eq!(encode("£é", encoding, false, 10).unwrap(), [0xa3, 0xe9]);
        assert_eq!(decode(&[0xa3, 0xe9], encoding, 10).unwrap(), "£é");
        assert!(matches!(
            encode("日本語", encoding, false, 100),
            Err(TextError::Unmappable(_))
        ));
        assert_eq!(encode("text", encoding, true, 100), Err(TextError::Bom));
        for label in ["ansi", "utf-16", "unknown", "replacement"] {
            assert_eq!(TextEncoding::from_label(label), Err(TextError::Encoding));
        }
    }
    #[test]
    fn multibyte_and_stateful_codepages_roundtrip_across_chunk_boundaries() {
        let text = "日本語\r\n".repeat(5000);
        for label in ["shift_jis", "iso-2022-jp"] {
            let encoding = TextEncoding::from_label(label).unwrap();
            let bytes = encode(&text, encoding, false, 100_000).unwrap();
            assert_eq!(decode(&bytes, encoding, 100_000).unwrap(), text);
            assert_eq!(
                encode(&text, encoding, false, bytes.len() - 1),
                Err(TextError::Limit)
            );
        }
    }
    #[test]
    fn malformed_text_and_conflicting_boms_fail_without_partial_results() {
        for (bytes, encoding) in [
            (&[0xff][..], TextEncoding::Utf8),
            (&[0][..], TextEncoding::Utf16Le),
            (&[0, 0xd8][..], TextEncoding::Utf16Le),
        ] {
            assert_eq!(decode(bytes, encoding, 100), Err(TextError::Invalid));
        }
        assert_eq!(
            decode(&[0xff, 0xfe, 65, 0], TextEncoding::Utf8, 100),
            Err(TextError::Bom)
        );
        assert_eq!(
            decode(&[0xfe, 0xff, 0, 65], TextEncoding::Utf16Le, 100),
            Err(TextError::Bom)
        );
    }
    #[test]
    fn streaming_and_byte_limits_cover_expansion_and_bom_bytes() {
        let text = "日本語😀".repeat(2000);
        for encoding in [
            TextEncoding::Utf8,
            TextEncoding::Utf16Le,
            TextEncoding::Utf16Be,
        ] {
            let bytes = encode(&text, encoding, true, 100_000).unwrap();
            assert_eq!(decode(&bytes, encoding, 100_000).unwrap(), text);
            assert_eq!(
                decode(&bytes, encoding, text.len() - 1),
                Err(TextError::Limit)
            );
            assert_eq!(
                encode(&text, encoding, true, bytes.len() - 1),
                Err(TextError::Limit)
            );
        }
        assert_eq!(
            encode("", TextEncoding::Utf8, true, 2),
            Err(TextError::Limit)
        );
        assert_eq!(encode("", TextEncoding::Utf8, false, 0).unwrap(), []);
    }

    #[test]
    fn incremental_output_keeps_allocation_within_its_byte_limit() {
        let limit = 10_001;
        let text = "a".repeat(5_000);
        let encoded = encode(&text, TextEncoding::Utf16Le, false, limit).unwrap();
        assert!(encoded.capacity() <= limit);
        assert_eq!(encoded.len(), 10_000);
        assert_eq!(
            decode(&encoded, TextEncoding::Utf16Le, limit).unwrap(),
            text
        );
    }
}
