use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};
use tagrex_core::provider::{FetchedImage, ProviderError};
use tagrex_providers_bandcamp::BandcampProvider;

// A loopback-only SOCKS server: no external service, DNS, credentials or media.
fn exercise(
    proxy_scheme: &str,
    reply: Option<u8>,
    fetch: impl FnOnce(&str, &str) -> Result<FetchedImage, ProviderError>,
) {
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    target.set_nonblocking(true).unwrap();
    let target_addr = target.local_addr().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let proxy = format!("{proxy_scheme}://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "no SOCKS connection");
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(err) => panic!("accept: {err}"),
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
        let mut connect = [0; 4];
        stream.read_exact(&mut connect).unwrap();
        assert_eq!(&connect[..3], &[5, 1, 0]);
        match connect[3] {
            1 => {
                let mut address = [0; 4];
                stream.read_exact(&mut address).unwrap();
                assert_eq!(address, [127, 0, 0, 1]);
            }
            3 => {
                let mut len = [0];
                stream.read_exact(&mut len).unwrap();
                let mut address = vec![0; len[0] as usize];
                stream.read_exact(&mut address).unwrap();
                assert_eq!(address, b"127.0.0.1");
            }
            other => panic!("unexpected address type: {other}"),
        }
        let mut port = [0; 2];
        stream.read_exact(&mut port).unwrap();
        assert_eq!(u16::from_be_bytes(port), target_addr.port());
        if let Some(status) = reply {
            stream
                .write_all(&[5, status, 0, 1, 127, 0, 0, 1, 0, 0])
                .unwrap();
            if status == 0 {
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                    assert!(request.len() < 8192);
                }
                assert!(request.starts_with(b"GET /cover.jpg HTTP/1.1\r\n"));
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: 3\r\nConnection: close\r\n\r\nimg").unwrap();
            }
        } else {
            // A truncated CONNECT response must be an ordinary transport error.
            stream.write_all(&[5, 0]).unwrap();
        }
    });
    let result = fetch(&proxy, &format!("http://{target_addr}/cover.jpg"));
    server.join().unwrap();
    if reply == Some(0) {
        assert_eq!(result.unwrap().data, b"img");
    } else {
        assert!(
            matches!(result, Err(ProviderError::Network(_))),
            "{result:?}"
        );
    }
    assert!(
        matches!(target.accept(), Err(err) if err.kind() == std::io::ErrorKind::WouldBlock),
        "must not fall back to a direct request"
    );
}

#[test]
fn negotiates_socks5_and_socks5h() {
    for scheme in ["socks5", "socks5h"] {
        exercise(scheme, Some(0), fetch);
    }
}

#[test]
fn rejected_and_truncated_negotiations_are_errors_without_direct_fallback() {
    exercise("socks5", Some(5), fetch);
    exercise("socks5", None, fetch);
}

#[test]
fn a_refused_proxy_connection_is_an_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy = format!("socks5://{}", listener.local_addr().unwrap());
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    target.set_nonblocking(true).unwrap();
    drop(listener);
    assert!(matches!(
        fetch(
            &proxy,
            &format!("http://{}/cover.jpg", target.local_addr().unwrap())
        ),
        Err(ProviderError::Network(_))
    ));
    assert!(
        matches!(target.accept(), Err(err) if err.kind() == std::io::ErrorKind::WouldBlock),
        "must not connect directly when the proxy is unreachable"
    );
}

fn fetch(proxy: &str, url: &str) -> Result<FetchedImage, ProviderError> {
    BandcampProvider::with_proxy(Some(proxy))?.fetch_image(url)
}
