//! Embedded HTTP server for `ratchet observe --ui`.
//!
//! Feature-gated (`ui`). Runs on the calling thread so the CLI can
//! block on it directly; a background thread handles the periodic
//! re-observation. Two endpoints:
//!
//! - `GET /` — returns the HTML export template with the current
//!   report JSON inlined (so the page still renders on first paint).
//!   The embedded JS polls `/api/report` for live updates.
//! - `GET /api/report` — returns the latest [`ObserveReport`] as
//!   JSON.
//!
//! Binds to `127.0.0.1:<port>` by default. Remote-accessible mode is
//! a flag away, but every caller should know what they're doing —
//! the CLI exposes `--ui-host 0.0.0.0` explicitly for the hosted /
//! self-hosted case.

use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use tiny_http::{Header, Method, Response, Server};

use crate::export::render_html;
use crate::report::ObserveReport;

/// Shared slot for the latest report. The observe loop writes, the
/// HTTP handler reads.
pub type ReportSlot = Arc<RwLock<ObserveReport>>;

/// Serve `ReportSlot` on `addr` until the process is interrupted.
pub fn serve(addr: SocketAddr, slot: ReportSlot) -> Result<()> {
    let server = Server::http(addr).map_err(|e| anyhow::anyhow!("binding {addr}: {e}"))?;
    eprintln!("ratchet observe --ui: http://{}", addr);

    for mut req in server.incoming_requests() {
        // Drain any request body so keep-alive plays nice. We ignore
        // the contents — every endpoint we serve is GET-shaped.
        let _ = drain(&mut req);
        match (req.method(), req.url()) {
            (Method::Get, "/") | (Method::Get, "/index.html") => {
                respond_html(req, &slot)?;
            }
            (Method::Get, "/api/report") => {
                respond_json(req, &slot)?;
            }
            _ => {
                let resp = Response::from_string("not found")
                    .with_status_code(404)
                    .with_header(header("content-type", "text/plain; charset=utf-8"));
                let _ = req.respond(resp);
            }
        }
    }
    Ok(())
}

fn respond_html(req: tiny_http::Request, slot: &ReportSlot) -> Result<()> {
    let html = {
        let guard = slot.read().map_err(|_| anyhow::anyhow!("slot poisoned"))?;
        render_html(&guard).context("rendering ui html")?
    };
    let resp = Response::from_string(html)
        .with_header(header("content-type", "text/html; charset=utf-8"))
        .with_header(header("cache-control", "no-store"));
    req.respond(resp).context("responding to /")
}

fn respond_json(req: tiny_http::Request, slot: &ReportSlot) -> Result<()> {
    let body = {
        let guard = slot.read().map_err(|_| anyhow::anyhow!("slot poisoned"))?;
        serde_json::to_vec(&*guard).context("serialising report for /api/report")?
    };
    let resp = Response::from_data(body)
        .with_header(header("content-type", "application/json"))
        .with_header(header("cache-control", "no-store"));
    req.respond(resp).context("responding to /api/report")
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("header name/value are ASCII")
}

fn drain(req: &mut tiny_http::Request) -> std::io::Result<()> {
    // `Request::as_reader()` returns a `&mut std::io::Read` via an
    // inherent impl; pull in the trait explicitly so clippy doesn't
    // see the import as unused and so the call site stays readable
    // with `read_to_end` rather than a manual byte loop.
    #[allow(unused_imports)]
    use std::io::Read as _;
    let mut sink = Vec::new();
    std::io::copy(req.as_reader(), &mut sink)?;
    Ok(())
}
