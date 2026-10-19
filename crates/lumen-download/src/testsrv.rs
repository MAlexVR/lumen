//! Tiny scriptable HTTP server for tests (loopback only, no network).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderMap, Response, StatusCode};
use axum::Router;

#[derive(Debug, Clone)]
pub struct Recorded {
    pub path_and_query: String,
    pub headers: HeaderMap,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    }
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn html(body: &str) -> Self {
        Self::bytes(200, "text/html; charset=utf-8", body.as_bytes().to_vec())
    }

    pub fn json(body: &str) -> Self {
        Self::bytes(200, "application/json", body.as_bytes().to_vec())
    }

    pub fn bytes(status: u16, content_type: &str, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: vec![("content-type".into(), content_type.into())],
            body,
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

type Handler = dyn Fn(&Recorded, usize) -> Reply + Send + Sync;

pub struct TestServer {
    pub port: u16,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl TestServer {
    /// `handler(request, n)` gets the zero-based count of earlier requests to
    /// the same path (without query).
    pub async fn start(
        handler: impl Fn(&Recorded, usize) -> Reply + Send + Sync + 'static,
    ) -> Self {
        let requests: Arc<Mutex<Vec<Recorded>>> = Arc::default();
        let counts: Arc<Mutex<HashMap<String, usize>>> = Arc::default();
        let handler: Arc<Handler> = Arc::new(handler);
        let log = Arc::clone(&requests);
        let app = Router::new().fallback(move |req: Request| {
            let handler = Arc::clone(&handler);
            let log = Arc::clone(&log);
            let counts = Arc::clone(&counts);
            async move {
                let pq = req
                    .uri()
                    .path_and_query()
                    .map(ToString::to_string)
                    .unwrap_or_default();
                let rec = Recorded {
                    path_and_query: pq,
                    headers: req.headers().clone(),
                };
                let path = req.uri().path().to_string();
                let nth = {
                    let mut c = counts.lock().unwrap_or_else(|e| e.into_inner());
                    let n = c.entry(path).or_insert(0);
                    let cur = *n;
                    *n += 1;
                    cur
                };
                log.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(rec.clone());
                let reply = handler(&rec, nth);
                let mut b = Response::builder()
                    .status(StatusCode::from_u16(reply.status).unwrap_or(StatusCode::OK));
                for (k, v) in &reply.headers {
                    b = b.header(k, v);
                }
                b.body(Body::from(reply.body)).unwrap_or_default()
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self { port, requests }
    }

    /// Base URL using the `localhost` name (trusted by the client config).
    pub fn base(&self) -> String {
        format!("http://localhost:{}", self.port)
    }

    /// Same server under a different host name (not trusted with cookies).
    pub fn foreign_base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn count(&self) -> usize {
        self.requests().len()
    }
}
