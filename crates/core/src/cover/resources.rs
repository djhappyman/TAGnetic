//! Conservative working-set admission and bounded encoded-only result caching.
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    io::{self, Write},
    sync::{Arc, LazyLock, Mutex},
};

pub(super) const WORK_BYTES: u64 = 256 * 1024 * 1024;
pub(super) const DECODE_BYTES: usize = 32 * 1024 * 1024;
pub(super) const ENCODE_BYTES: usize = 16 * 1024 * 1024;
pub(super) static PROCESS: Mutex<()> = Mutex::new(());
pub(super) static CACHE: LazyLock<Mutex<Cache>> =
    LazyLock::new(|| Mutex::new(Cache::new(32, 64 * 1024 * 1024)));
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Key {
    digest: [u8; 32],
    max: u32,
    quality: u8,
}
impl Key {
    pub fn new(data: &[u8], max: u32, quality: u8) -> Self {
        Self {
            digest: Sha256::digest(data).into(),
            max,
            quality,
        }
    }
}
pub(super) struct Cache {
    entries: VecDeque<(Key, Arc<[u8]>)>,
    bytes: usize,
    max_entries: usize,
    max_bytes: usize,
}
impl Cache {
    fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
            max_entries,
            max_bytes,
        }
    }
    pub fn get(&mut self, key: &Key) -> Option<Arc<[u8]>> {
        let index = self.entries.iter().position(|(k, _)| k == key)?;
        let entry = self.entries.remove(index)?;
        let bytes = entry.1.clone();
        self.entries.push_back(entry);
        Some(bytes)
    }
    pub fn insert(&mut self, key: Key, data: Arc<[u8]>) {
        if let Some(index) = self.entries.iter().position(|(k, _)| k == &key) {
            if let Some((_, old)) = self.entries.remove(index) {
                self.bytes -= old.len() + 128;
            }
        }
        let bytes = data.len().saturating_add(128);
        if self.max_entries == 0 || bytes > self.max_bytes {
            return;
        }
        while self.entries.len() >= self.max_entries || self.bytes > self.max_bytes - bytes {
            if let Some((_, old)) = self.entries.pop_front() {
                self.bytes -= old.len() + 128;
            } else {
                break;
            }
        }
        self.bytes += bytes;
        self.entries.push_back((key, data));
    }
}
fn dimensions(w: u32, h: u32, max: u32) -> (u64, u64) {
    let longer = u64::from(w.max(h)).max(1);
    let bound = u64::from(max).min(longer);
    (
        (u64::from(w) * bound).div_ceil(longer).max(1),
        (u64::from(h) * bound).div_ceil(longer).max(1),
    )
}
/// Includes decoded pixels, resize scratch, RGB/output copies and decoder quota.
/// Decoder internals/allocator overhead are implementation dependent, so this
/// is conservative admission accounting rather than an OS-level memory limit.
pub(super) fn admitted(w: u32, h: u32, max: u32, rows: bool) -> bool {
    if w == 0 || h == 0 || max == 0 {
        return false;
    }
    // Row-wise decoding saves memory but must also bound decompression work.
    if u64::from(w) * u64::from(h) > 512_000_000 {
        return false;
    }
    let (sw, sh) = if rows {
        dimensions(w, h, max.saturating_mul(2))
    } else {
        (u64::from(w), u64::from(h))
    };
    let (tw, th) = dimensions(w, h, max);
    let decoded = sw.saturating_mul(sh).saturating_mul(8);
    let scratch = sw
        .saturating_mul(th)
        .max(tw.saturating_mul(sh))
        .saturating_mul(16);
    let final_images = tw.saturating_mul(th).saturating_mul(11);
    let row_scratch = if rows {
        u64::from(w)
            .saturating_mul(16)
            .saturating_add(sw.saturating_mul(32))
    } else {
        0
    };
    decoded
        .saturating_add(scratch)
        .saturating_add(final_images)
        .saturating_add(row_scratch)
        .saturating_add(DECODE_BYTES as u64)
        .saturating_add(2 * ENCODE_BYTES as u64)
        <= WORK_BYTES
}
pub(super) struct Encoded(pub Vec<u8>);
impl Write for Encoded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > ENCODE_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other("Encoded artwork exceeds budget"));
        }
        let required = self.0.len() + bytes.len();
        if required > self.0.capacity() {
            let target = required
                .max(self.0.capacity().saturating_mul(2).max(4096))
                .min(ENCODE_BYTES);
            self.0
                .try_reserve_exact(target - self.0.len())
                .map_err(|_| io::Error::other("Insufficient memory for encoded artwork"))?;
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admission_handles_extreme_dimensions_and_preserves_streaming_png_support() {
        assert!(!admitted(u32::MAX, u32::MAX, u32::MAX, false));
        assert!(!admitted(30_000, 30_000, 500, false));
        assert!(admitted(20_000, 20_000, 500, true));
        assert!(!admitted(30_000, 30_000, 500, true));
        assert!(admitted(2000, 1000, 500, false));
        assert!(!admitted(u32::MAX, 1, 500, true));
        assert!(!admitted(0, 1, 1, true));
    }
    #[test]
    fn encoded_cache_is_lru_bounded_and_keyed_by_content_and_options() {
        let keys = [
            Key::new(b"a", 500, 85),
            Key::new(b"b", 500, 85),
            Key::new(b"a", 600, 85),
            Key::new(b"a", 500, 90),
        ];
        let data: Arc<[u8]> = vec![1; 16].into();
        let mut c = Cache::new(2, 300);
        c.insert(keys[0].clone(), data.clone());
        c.insert(keys[1].clone(), data.clone());
        assert!(Arc::ptr_eq(&c.get(&keys[0]).unwrap(), &data));
        c.insert(keys[2].clone(), data.clone());
        assert!(c.get(&keys[1]).is_none());
        assert!(c.get(&keys[3]).is_none());
        assert!(c.bytes <= 300);
        c.insert(keys[3].clone(), vec![0; 400].into());
        assert!(c.get(&keys[3]).is_none());
        assert_eq!(
            c.bytes,
            c.entries.iter().map(|(_, v)| v.len() + 128).sum::<usize>()
        );
    }
    #[test]
    fn encoder_sink_refuses_before_crossing_the_limit() {
        let mut out = Encoded(Vec::new());
        out.write_all(&[1; 100]).unwrap();
        assert!(out.write_all(&vec![2; ENCODE_BYTES]).is_err());
        assert_eq!(out.0.len(), 100);
    }
}
