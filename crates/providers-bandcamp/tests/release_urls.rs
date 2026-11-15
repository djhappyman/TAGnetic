use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};
use tagrex_core::provider::{MetadataProvider, ProviderError, ReleaseId};
use tagrex_providers_bandcamp::BandcampProvider;

const START: &str = "http://artist.bandcamp.com/album/old";
const FINAL: &str = "http://artist.bandcamp.com/album/new";
const HTML: &str = r#"<script data-tralbum="{&quot;artist&quot;:&quot;Artist&quot;,&quot;current&quot;:{&quot;title&quot;:&quot;New&quot;},&quot;trackinfo&quot;:[{&quot;track_num&quot;:1,&quot;title&quot;:&quot;Own&quot;}]}"></script>"#;

// The proxy serves canned HTTP responses itself. No request leaves loopback,
// even though the URL/Host are the production provider's canonical authority.
fn with_pages(pages: Vec<(u16, Option<String>, &'static str)>, run: impl FnOnce(BandcampProvider)) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let proxy = format!("socks5h://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        for (status, location, body) in pages {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "expected proxy connection");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(err) => panic!("{err}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut greeting = [0; 3];
            stream.read_exact(&mut greeting).unwrap();
            assert_eq!(greeting, [5, 1, 0]);
            stream.write_all(&[5, 0]).unwrap();
            let mut connect = [0; 5];
            stream.read_exact(&mut connect).unwrap();
            assert_eq!(&connect[..4], &[5, 1, 0, 3]);
            let mut host = vec![0; connect[4] as usize];
            stream.read_exact(&mut host).unwrap();
            assert_eq!(host, b"artist.bandcamp.com");
            let mut port = [0; 2];
            stream.read_exact(&mut port).unwrap();
            assert_eq!(u16::from_be_bytes(port), 80);
            stream.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
                assert!(request.len() < 8192);
            }
            let location = location
                .map(|url| format!("Location: {url}\r\n"))
                .unwrap_or_default();
            write!(stream, "HTTP/1.1 {status} Fixture\r\n{location}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    run(BandcampProvider::with_proxy(Some(&proxy)).unwrap());
    server.join().unwrap();
}

#[test]
fn follows_validated_release_redirects_and_uses_the_final_identity() {
    with_pages(
        vec![(302, Some("/album/new".into()), ""), (200, None, HTML)],
        |provider| {
            let release = provider.fetch_release(&ReleaseId(START.into())).unwrap();
            assert_eq!(release.id.0, FINAL);
            assert_eq!(release.title, "New");
            assert_eq!(release.tracks.len(), 1);
            assert_eq!(release.tracks[0].title, "Own");
        },
    );
}

#[test]
fn rejects_unrelated_redirects_before_opening_a_connection() {
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    target.set_nonblocking(true).unwrap();
    for location in [
        format!("http://{}//album/new", target.local_addr().unwrap()),
        "http://artist.bandcamp.com.evil.test//album/new".into(),
        "http://localhost//album/new".into(),
        "http://user@artist.bandcamp.com//album/new".into(),
    ] {
        with_pages(vec![(302, Some(location), "")], |provider| {
            assert!(matches!(
                provider.fetch_release(&ReleaseId(START.into())),
                Err(ProviderError::Other(_))
            ));
        });
    }
    assert!(matches!(target.accept(), Err(err) if err.kind() == std::io::ErrorKind::WouldBlock));
}

#[test]
fn rejects_a_loop_and_a_missing_redirect_location() {
    for location in [Some(START.into()), None] {
        with_pages(vec![(302, location, "")], |provider| {
            assert!(matches!(
                provider.fetch_release(&ReleaseId(START.into())),
                Err(ProviderError::Other(_))
            ));
        });
    }
}

#[test]
fn bounds_changing_redirect_chains() {
    let pages = (1..=6)
        .map(|index| (302, Some(format!("/album/next-{index}")), ""))
        .collect();
    with_pages(pages, |provider| {
        assert!(
            matches!(provider.fetch_release(&ReleaseId(START.into())), Err(ProviderError::Other(message)) if message.contains("too many"))
        );
    });
}
