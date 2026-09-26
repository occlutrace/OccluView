//! The shipped update agent must refuse plain HTTP.
//!
//! SECURITY.md tells a reader the product makes two ordinary HTTPS GETs and
//! nothing else. `ureq` follows five redirects by default, so a legitimate HTTPS
//! host answering with an `http://` Location would be followed down — inside the
//! signature boundary, but outside the promise made to the reader.
//!
//! This must be an integration test. The agent is built with
//! `.https_only(!cfg!(test))` in `src/lib.rs`, and `cfg!(test)` is true only
//! inside the crate's own unit tests. An integration test links the crate as an
//! ordinary dependency, so the flag is `https_only(true)` here: this drives the
//! same agent a shipped build uses.
//!
//! The test opens a loopback HTTP listener, points the real `check_with` entry
//! point at it, and asserts the request never arrives.

#![allow(clippy::expect_used)]

use occluview_update::check_with;
use std::io::Read;
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

/// A listener that reports whether anything ever connects to it.
fn observing_listener() -> (String, mpsc::Receiver<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
    let port = listener.local_addr().expect("local addr").port();
    let (connected_tx, connected_rx) = mpsc::channel();

    std::thread::spawn(move || {
        // Answer at most one connection with a minimal response, so a client
        // that does connect is measured as "it connected" rather than timing
        // out, and the assertion is about the refusal, not about latency.
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = connected_tx.send(());
            let mut sink = Vec::new();
            let _ = stream.read_to_end(&mut sink);
        }
    });

    (format!("http://127.0.0.1:{port}/latest.json"), connected_rx)
}

#[test]
fn the_shipped_update_agent_refuses_to_reach_a_plain_http_manifest() {
    let (manifest_url, connected) = observing_listener();
    let signature_url = format!("{manifest_url}.minisig");

    // An empty key list is irrelevant: the refusal has to happen at the
    // transport, before any key or signature is consulted. If plain HTTP were
    // allowed the fetch would succeed and the failure would instead be about
    // verification, which is a different (and acceptable) error. The assertion
    // that separates the two is whether a connection was opened at all.
    let outcome = check_with(&manifest_url, &signature_url, &[], "0.0.1");

    assert!(
        connected.recv_timeout(Duration::from_millis(750)).is_err(),
        "the update agent opened a connection to a plain-HTTP manifest URL. \
         A shipped build must refuse plain HTTP outright, or an https host can \
         redirect the updater onto it and SECURITY.md's promise is broken"
    );
    assert!(
        outcome.is_err(),
        "and the refusal must surface as an error rather than a silent None"
    );
}
