//! Streaming accounting avoids a second owned copy of serialized plan data.
use super::*;
use std::io::{self, Write};

pub(super) const MAX_DRAFT_BYTES: usize = 128 * 1024 * 1024;

struct Sink {
    bytes: usize,
    limit: usize,
    exceeded: bool,
    digest: Option<Sha256>,
}
impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes) {
            self.exceeded = true;
            return Err(io::Error::other("JSON budget exceeded"));
        }
        self.bytes += bytes.len();
        if let Some(digest) = &mut self.digest {
            digest.update(bytes);
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn account(value: &impl Serialize, limit: usize, digest: bool) -> ServiceResult<Sink> {
    let mut sink = Sink {
        bytes: 0,
        limit,
        exceeded: false,
        digest: digest.then(Sha256::new),
    };
    if let Err(error) = serde_json::to_writer(&mut sink, value) {
        return Err(if sink.exceeded {
            ServiceError::new("quota", "Serialized draft exceeds the 128 MiB budget")
        } else {
            error.into()
        });
    }
    Ok(sink)
}
pub(super) fn size(value: &impl Serialize) -> ServiceResult<usize> {
    Ok(account(value, MAX_DRAFT_BYTES, false)?.bytes)
}
/// Account for compact wire JSON without holding another serialized copy.
pub(super) fn fits_wire_cap(value: &Value) -> ServiceResult<bool> {
    match account(value, MAX_ARGUMENT_BYTES, false) {
        Ok(_) => Ok(true),
        Err(error) if error.code == "quota" => Ok(false),
        Err(error) => Err(error),
    }
}
/// Check a response before committing handles that its caller must receive.
/// The counting sink stops at the wire cap without allocating serialized JSON.
pub(super) fn check_response(value: &Value) -> ServiceResult<()> {
    if fits_wire_cap(value)? {
        Ok(())
    } else {
        Err(ServiceError::new(
            "response_too_large",
            "Response exceeds 1 MiB; no new private handles were retained",
        ))
    }
}
pub(super) fn digest(value: &impl Serialize) -> ServiceResult<String> {
    let sink = account(value, MAX_DRAFT_BYTES, true)?;
    Ok(format!(
        "{:x}",
        sink.digest.expect("digest enabled").finalize()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn size_and_digest_match_wire_json_including_escaping_and_unicode() {
        let value = json!({"z":"a\n\"\\é", "a":[null, true, 42]});
        let bytes = serde_json::to_vec(&value).unwrap();
        assert_eq!(size(&value).unwrap(), bytes.len());
        assert_eq!(digest(&value).unwrap(), hash(&bytes));
        assert!(account(&value, bytes.len(), true).is_ok());
        assert_eq!(
            account(&value, bytes.len() - 1, false).err().unwrap().code,
            "quota"
        );
    }
    #[test]
    fn budget_stops_serialization_before_visiting_all_elements() {
        use serde::ser::SerializeSeq;
        struct Many(std::cell::Cell<usize>);
        impl Serialize for Many {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut seq = serializer.serialize_seq(Some(1_000_000))?;
                for _ in 0..1_000_000 {
                    self.0.set(self.0.get() + 1);
                    seq.serialize_element(&"expansion\n\"é")?;
                }
                seq.end()
            }
        }
        let many = Many(std::cell::Cell::new(0));
        assert_eq!(account(&many, 128, true).err().unwrap().code, "quota");
        assert!(many.0.get() < 20);
    }
}
