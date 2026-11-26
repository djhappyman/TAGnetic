//! Which document each window may display.
//!
//! The shell serves the main app, audio tools, and a browser for Beatport's
//! own sign-in pages while the user authorizes an account. Tauri lets a
//! webview navigate anywhere unless a plugin hook says otherwise. The UI
//! renders provider-supplied links, so
//! without a policy a middle-click, the context menu's "Open Link" or a
//! hyperlink dropped on the window could replace the app with a remote
//! document: no address bar, no way back, and the Tagnetic chrome wrapped
//! around attacker-chosen content.
//!
//! The rule is per window label and fails closed:
//!
//! - `main` may only show the document the app itself serves: the custom
//!   protocol origin (`tauri://localhost` on Linux and macOS,
//!   `http://tauri.localhost` on Windows) and, in a development build, the
//!   exact origin of `build.devUrl`. Paths are free; origins are not. A
//!   look-alike host, a non-default port or embedded credentials are denied.
//! - Each `audio-tool-*` window may only show its own audio tools page at
//!   the same trusted origins as the main window.
//! - `beatport-login` may only show `https:` documents on the standard port,
//!   plus the blank document. The authorize page, the redirect that carries
//!   the code and whatever Beatport's own identity pages redirect through are
//!   all https; a host allowlist for that flow cannot be verified without a
//!   live sign-in, so the scheme — not a guessed host list — is the boundary.
//! - Any other label gets no navigation at all. Handing every future window
//!   the `main` allowlist, or every future window `https:`, is exactly the
//!   mistake this policy exists to prevent: a new window must be added here
//!   deliberately.
//!
//! What this deliberately does not do: it does not add OAuth `state` or PKCE
//! (the sign-in window still accepts a code from any page that can navigate it
//! to the callback), it does not prune the capability set, and it does not hand
//! denied URLs to the system browser — the existing validated `open` calls
//! keep that role. Those are separate decisions, and the first needs a live
//! Beatport sign-in before it can be called safe.

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime, Url, Webview};

use crate::audio_studio::{AudioToolKind, TOOL_DOCUMENT};

/// Label of the window that shows the app itself, matching `tauri.conf.json`.
const MAIN_WINDOW: &str = "main";

/// Install the navigation policy.
pub(crate) fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("tagrex-navigation")
        .on_navigation(|webview, url| {
            let development_origin = development_origin(webview);
            allows(webview.label(), url, development_origin.as_ref())
        })
        .build()
}

/// Whether `label`'s window may navigate to `url`.
///
/// `development_origin` is the trusted origin of the Vite dev server, present
/// only in a development build with a loopback `build.devUrl`; a production
/// build passes `None` and therefore trusts no server.
fn allows(label: &str, url: &Url, development_origin: Option<&Url>) -> bool {
    if is_blank_document(url) {
        return label == MAIN_WINDOW || label == crate::BEATPORT_LOGIN_WINDOW;
    }
    // Credentials never belong in a trusted origin: `https://evil@beatport.com`
    // and `tauri://user@localhost` still carry a real host, but accepting them
    // would mean accepting a URL whose obvious reading differs from its origin.
    if has_credentials(url) {
        return false;
    }
    if label == MAIN_WINDOW {
        return is_app_document(url)
            || development_origin.is_some_and(|origin| same_origin(url, origin));
    }
    if let Some(kind) = AudioToolKind::ALL
        .into_iter()
        .find(|kind| kind.label() == label)
    {
        return url.path().strip_prefix('/') == Some(TOOL_DOCUMENT)
            && url.query() == Some(kind.query())
            && (is_app_document(url)
                || development_origin.is_some_and(|origin| same_origin(url, origin)));
    }
    if label == crate::BEATPORT_LOGIN_WINDOW {
        return is_https_document(url);
    }
    false
}

/// The origin of the configured dev server, when this build may use one.
///
/// Read from the running config rather than repeated as a literal so
/// `tauri.dev.conf.json` stays the single source of the port. `tauri::is_dev()`
/// is Tauri's own production switch (`custom-protocol`), so a packaged build
/// never trusts a server even if one were configured.
fn development_origin<R: Runtime>(webview: &Webview<R>) -> Option<Url> {
    if !tauri::is_dev() {
        return None;
    }
    webview
        .config()
        .build
        .dev_url
        .as_ref()
        .and_then(valid_development_origin)
}

/// Accept a configured dev URL only when it is an http(s) origin on this
/// machine. A dev server is a local tool; a remote or non-loopback `devUrl`
/// would turn development convenience into a trusted external origin.
fn valid_development_origin(url: &Url) -> Option<Url> {
    let allowed_scheme = matches!(url.scheme(), "http" | "https");
    (allowed_scheme && is_loopback_host(url) && !has_credentials(url)).then(|| url.clone())
}

/// The document Tauri serves the app from. Both desktop spellings are allowed
/// because the choice is made by the platform at compile time, and neither can
/// be served by a host the app did not start.
fn is_app_document(url: &Url) -> bool {
    match url.scheme() {
        "tauri" => host_is(url, "localhost") && url.port().is_none(),
        "http" => host_is(url, "tauri.localhost") && matches!(url.port(), None | Some(80)),
        _ => false,
    }
}

/// An `https:` document on the standard port: the sign-in window's whole
/// allowlist, apart from the blank document.
fn is_https_document(url: &Url) -> bool {
    url.scheme() == "https" && matches!(url.port_or_known_default(), Some(443))
}

/// `about:blank`, exactly. A window may start on a blank document that has no
/// origin of its own; anything else under the `about:` scheme is not that.
fn is_blank_document(url: &Url) -> bool {
    url.as_str() == "about:blank"
}

fn has_credentials(url: &Url) -> bool {
    !url.username().is_empty() || url.password().is_some()
}

fn host_is(url: &Url, expected: &str) -> bool {
    url.host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case(expected))
}

/// Loopback only: `localhost`, `127.0.0.0/8` or `::1`. IPv6 hosts arrive in
/// brackets from [`Url::host_str`], so they are unwrapped before parsing.
fn is_loopback_host(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let address = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    address
        .parse::<std::net::IpAddr>()
        .is_ok_and(|address| address.is_loopback())
}

/// Scheme, host and effective port. Paths, queries and fragments are ignored,
/// which is what makes a dev server usable while keeping a different port or a
/// look-alike host out.
fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme().eq_ignore_ascii_case(b.scheme())
        && a.host_str()
            .zip(b.host_str())
            .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
        && a.port_or_known_default() == b.port_or_known_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(value: &str) -> Url {
        Url::parse(value).expect("test URL should parse")
    }

    /// Mirrors `build.devUrl` in `tauri.dev.conf.json`.
    fn development_origin() -> Url {
        url("http://127.0.0.1:1420")
    }

    #[test]
    fn the_main_window_shows_only_the_documents_the_app_serves() {
        let cases = [
            ("tauri://localhost", true),
            ("tauri://localhost/index.html", true),
            ("tauri://localhost/#/library?track=1", true),
            ("http://tauri.localhost", true),
            ("http://tauri.localhost/", true),
            ("http://tauri.localhost:80/index.html", true),
            ("about:blank", true),
            // A development server means nothing to a production build.
            ("http://127.0.0.1:1420/", false),
            // Anything that is not the app's own document.
            ("https://www.discogs.com/release/1", false),
            ("https://tauri.localhost/", false),
            ("file:///etc/passwd", false),
            ("data:text/html,hello", false),
            ("javascript:alert(1)", false),
            ("tagrex://localhost/", false),
            // Look-alike hosts and non-default ports.
            ("http://tauri.localhost.evil/", false),
            ("http://evil.tauri.localhost/", false),
            ("http://tauri.localhost:8080/", false),
            ("tauri://localhost:80/", false),
            ("tauri://localhost.evil/", false),
            // Credentials are not part of a trusted origin.
            ("tauri://user@localhost/", false),
            ("http://user:secret@tauri.localhost/", false),
        ];
        for (case, expected) in cases {
            assert_eq!(allows(MAIN_WINDOW, &url(case), None), expected, "{case}");
        }
    }

    #[test]
    fn a_development_build_trusts_exactly_the_dev_server_origin() {
        let origin = development_origin();
        let allowed = [
            "http://127.0.0.1:1420/",
            "http://127.0.0.1:1420/index.html",
            "http://127.0.0.1:1420/#/settings",
        ];
        for case in allowed {
            assert!(allows(MAIN_WINDOW, &url(case), Some(&origin)), "{case}");
        }
        let denied = [
            "http://127.0.0.1:1421/",
            "http://localhost:1420/",
            "https://127.0.0.1:1420/",
            "http://127.0.0.1.evil:1420/",
            "http://127.0.0.1:1420@evil.com/",
            "http://user@127.0.0.1:1420/",
        ];
        for case in denied {
            assert!(!allows(MAIN_WINDOW, &url(case), Some(&origin)), "{case}");
        }
    }

    #[test]
    fn an_audio_tool_shows_only_its_own_page() {
        let origin = development_origin();
        for kind in AudioToolKind::ALL {
            let page = format!("/{TOOL_DOCUMENT}?{}", kind.query());
            for host in ["tauri://localhost", "http://tauri.localhost"] {
                assert!(allows(kind.label(), &url(&format!("{host}{page}")), None));
            }
            let dev_page = origin.join(&page).unwrap();
            assert!(allows(kind.label(), &dev_page, Some(&origin)));
            assert!(!allows(kind.label(), &dev_page, None));
            for host in ["https://example.com", "http://127.0.0.1:1421"] {
                assert!(!allows(
                    kind.label(),
                    &url(&format!("{host}{page}")),
                    Some(&origin)
                ));
            }
            assert!(!allows(
                kind.label(),
                &url("tauri://localhost/index.html"),
                None
            ));
            for other in AudioToolKind::ALL
                .into_iter()
                .filter(|other| other.label() != kind.label())
            {
                assert!(!allows(other.label(), &dev_page, Some(&origin)));
            }
        }
    }

    #[test]
    fn the_sign_in_window_shows_only_https_on_the_standard_port() {
        let allowed = [
            "https://api.beatport.com/v4/auth/o/authorize/?response_type=code",
            "https://api.beatport.com/v4/auth/o/post-message/?code=abc",
            "https://www.beatport.com/",
            // HTTPS identity redirects are allowed without a guessed host list.
            "https://identity.example.com/",
            "https://api.beatport.com:443/v4/auth/o/post-message/",
            "about:blank",
        ];
        for case in allowed {
            assert!(
                allows(crate::BEATPORT_LOGIN_WINDOW, &url(case), None),
                "{case}"
            );
        }
        let denied = [
            "http://api.beatport.com/v4/auth/o/post-message/?code=abc",
            "https://api.beatport.com:8443/",
            "https://user@api.beatport.com/",
            "file:///etc/passwd",
            "data:text/html,hello",
            "javascript:alert(1)",
            "tagrex://localhost/",
        ];
        for case in denied {
            assert!(
                !allows(crate::BEATPORT_LOGIN_WINDOW, &url(case), None),
                "{case}"
            );
        }
    }

    #[test]
    fn windows_the_policy_does_not_know_get_no_navigation() {
        let origin = development_origin();
        for case in [
            "https://api.beatport.com/",
            "https://www.beatport.com/",
            "tauri://localhost/",
            "http://127.0.0.1:1420/",
            "about:blank",
        ] {
            assert!(
                !allows("some-future-window", &url(case), Some(&origin)),
                "{case}"
            );
        }
    }

    #[test]
    fn only_a_loopback_development_origin_is_trusted() {
        for case in [
            "http://127.0.0.1:1420",
            "http://localhost:1420",
            "http://[::1]:1420",
            "https://127.0.0.1:1420",
        ] {
            assert!(valid_development_origin(&url(case)).is_some(), "{case}");
        }
        for case in [
            "https://example.com/",
            "http://192.168.1.10:1420/",
            "http://0.0.0.0:1420/",
            "http://user:secret@127.0.0.1:1420/",
            "ftp://127.0.0.1:1420/",
            "file:///tmp/dev",
        ] {
            assert!(valid_development_origin(&url(case)).is_none(), "{case}");
        }
    }

    #[test]
    fn the_configured_development_server_is_trusted_and_exact() {
        // The port is configuration, not a constant here, so read the file the
        // dev build is given and prove this policy accepts its origin.
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.dev.conf.json"))
                .expect("the dev config should parse");
        let dev_url = config["build"]["devUrl"]
            .as_str()
            .expect("the dev config should name a devUrl");
        let configured = url(dev_url);
        let origin =
            valid_development_origin(&configured).expect("the dev server should be loopback");
        assert!(allows(MAIN_WINDOW, &configured, Some(&origin)));

        let host = configured
            .host_str()
            .expect("the dev URL should name a host");
        let mut look_alike = configured.clone();
        look_alike
            .set_host(Some(&format!("{host}.evil")))
            .expect("the look-alike host should be valid");
        assert!(!allows(MAIN_WINDOW, &look_alike, Some(&origin)));

        if let Some(port) = configured.port() {
            let mut other_port = configured.clone();
            other_port
                .set_port(Some(port + 1))
                .expect("the other port should be valid");
            assert!(!allows(MAIN_WINDOW, &other_port, Some(&origin)));
        }
    }
}
