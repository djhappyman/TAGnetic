//! Unauthenticated artwork downloads for paired external agents.
//! Provider tokens never enter this client. Every actual connection resolves
//! once through a public-address policy; redirects repeat the same checks.
use std::io::Cursor;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::{Duration, Instant};

use base64::Engine;
use tagrex_commands::CoverArtDto;
use tagrex_service::ServiceError;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_PIXELS: u64 = 24_000_000;
const DOWNLOAD_BUDGET: Duration = Duration::from_secs(45);

#[derive(Debug)]
struct PublicResolver;

impl Resolver for PublicResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let addresses = DefaultResolver::default().resolve(uri, config, timeout)?;
        // Reject mixed public/private answers too. The connector receives only
        // these checked addresses, so it never performs a second DNS lookup.
        if addresses.iter().any(|address| !public_ip(address.ip())) {
            return Err(ureq::Error::HostNotFound);
        }
        Ok(addresses)
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || a == 127
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && (b == 168 || (b == 0 && (c == 0 || c == 2)) || (b == 88 && c == 99)))
        || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
        || (a == 203 && b == 0 && c == 113))
}

fn public_v6(ip: Ipv6Addr) -> bool {
    if let Some(ip) = ip.to_ipv4_mapped() {
        return public_v4(ip);
    }
    let segments = ip.segments();
    // Only ordinary global-unicast destinations. Exclude documentation,
    // Teredo, 6to4 and protocol-assignment prefixes that can tunnel locally.
    segments[0] & 0xe000 == 0x2000
        && segments[0] != 0x2002
        && !(segments[0] == 0x2001 && (segments[1] < 0x0200 || segments[1] == 0x0db8))
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => public_v4(ip),
        IpAddr::V6(ip) => public_v6(ip),
    }
}

fn failure(message: &str) -> ServiceError {
    ServiceError::new("artwork_download_refused", message)
}

fn validate_url(value: &str) -> Result<url::Url, ServiceError> {
    let url = url::Url::parse(value).map_err(|_| failure("Artwork URL is invalid."))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
        || url.host_str().is_none()
    {
        return Err(failure(
            "Artwork must use a public HTTPS address on the standard port.",
        ));
    }
    if let Some(host) = url.host_str() {
        if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
            if !public_ip(ip) {
                return Err(failure("Private artwork addresses are not allowed."));
            }
        }
        if host.eq_ignore_ascii_case("localhost")
            || host.ends_with(".localhost")
            || host.ends_with(".local")
        {
            return Err(failure("Local artwork addresses are not allowed."));
        }
    }
    Ok(url)
}

pub fn fetch(url: &str) -> Result<CoverArtDto, ServiceError> {
    let started = Instant::now();
    let mut url = validate_url(url)?;
    for hop in 0..=5 {
        let remaining = DOWNLOAD_BUDGET
            .checked_sub(started.elapsed())
            .ok_or_else(|| failure("Artwork download timed out."))?;
        let config = ureq::Agent::config_builder()
            .https_only(true)
            .proxy(None)
            .max_redirects(0)
            .timeout_global(Some(remaining))
            .build();
        let agent = ureq::Agent::with_parts(config, DefaultConnector::default(), PublicResolver);
        let mut response = agent
            .get(url.as_str())
            .header(
                "User-Agent",
                concat!("TAGnetic/", env!("CARGO_PKG_VERSION")),
            )
            .header("Accept", "image/jpeg, image/png, image/webp")
            .header("Accept-Encoding", "identity")
            .call()
            .map_err(|_| failure("Artwork could not be downloaded safely."))?;
        if response.status().is_redirection() {
            if hop == 5 {
                return Err(failure("Artwork redirected too many times."));
            }
            let next = response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| failure("Artwork redirect has no valid destination."))?;
            let target = url
                .join(next)
                .map_err(|_| failure("Artwork redirect is invalid."))?;
            url = validate_url(target.as_str())?;
            continue;
        }
        if !response.status().is_success() {
            return Err(failure("The source did not return artwork."));
        }
        if response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|bytes| bytes > MAX_BYTES as u64)
        {
            return Err(failure("Artwork exceeds the 16 MiB download limit."));
        }
        let bytes = response
            .body_mut()
            .with_config()
            .limit(MAX_BYTES as u64)
            .read_to_vec()
            .map_err(|_| failure("Artwork is incomplete or exceeds the download limit."))?;
        let format = image::guess_format(&bytes)
            .map_err(|_| failure("The response is not recognized artwork."))?;
        if !matches!(
            format,
            image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP
        ) {
            return Err(failure("Artwork must be PNG, JPEG or WebP."));
        }
        let (width, height) = image::ImageReader::with_format(Cursor::new(&bytes), format)
            .into_dimensions()
            .map_err(|_| failure("Artwork dimensions could not be read."))?;
        if width > 8192 || height > 8192 || u64::from(width) * u64::from(height) > MAX_PIXELS {
            return Err(failure("Artwork exceeds the image dimension limit."));
        }
        return Ok(CoverArtDto {
            mime: format.to_mime_type().to_owned(),
            data_base64: base64::engine::general_purpose::STANDARD
                .encode(bytes)
                .into(),
            kind: "front".into(),
            ..Default::default()
        });
    }
    Err(failure("Artwork could not be downloaded safely."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_local_reserved_and_tunneled_destinations() {
        for address in [
            "0.0.0.0",
            "10.1.2.3",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "172.31.1.1",
            "192.168.0.1",
            "192.0.2.1",
            "198.19.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "::ffff:127.0.0.1",
            "2002:7f00:1::",
            "2001:db8::1",
        ] {
            assert!(!public_ip(address.parse().unwrap()), "{address}");
        }
        for address in [
            "1.1.1.1",
            "8.8.8.8",
            "104.16.12.34",
            "2606:4700:4700::1111",
            "2001:4860:4860::8888",
        ] {
            assert!(public_ip(address.parse().unwrap()), "{address}");
        }
    }

    #[test]
    fn url_checks_apply_to_relative_redirect_targets_too() {
        for url in [
            "http://i.discogs.com/a.jpg",
            "https://user:secret@i.discogs.com/a.jpg",
            "https://127.0.0.1/a.jpg",
            "https://[::1]/a.jpg",
            "https://0x7f000001/a.jpg",
            "https://localhost/a.jpg",
            "https://a.local/a.jpg",
            "https://i.discogs.com:444/a.jpg",
        ] {
            assert!(validate_url(url).is_err(), "{url}");
        }
        let original = validate_url("https://i.discogs.com/image.jpg").unwrap();
        let redirected = original.join("//127.0.0.1/private").unwrap();
        assert!(validate_url(redirected.as_str()).is_err());
        assert!(validate_url(original.join("../other.jpg").unwrap().as_str()).is_ok());
    }
}
