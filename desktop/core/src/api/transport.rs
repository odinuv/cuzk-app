use crate::error::CoreError;
use std::time::Duration;

/// HTTP GET abstraction. `Err` is used only for network errors and timeouts;
/// any received HTTP response (including 4xx/5xx) is `Ok((status, body))`.
pub trait Transport: Send + Sync {
    fn get(&self, url: &str, api_key: &str) -> Result<(u16, String), CoreError>;
}

pub struct ReqwestTransport {
    client: reqwest::blocking::Client,
}

impl ReqwestTransport {
    pub fn new() -> Result<Self, CoreError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| CoreError::Network(e.without_url().to_string()))?;
        Ok(Self { client })
    }
}

impl Transport for ReqwestTransport {
    fn get(&self, url: &str, api_key: &str) -> Result<(u16, String), CoreError> {
        let net = |e: reqwest::Error| CoreError::Network(e.without_url().to_string());
        let resp = self
            .client
            .get(url)
            .header("ApiKey", api_key)
            .header("Accept", "application/json")
            .send()
            .map_err(net)?;
        let status = resp.status().as_u16();
        let body = resp.text().map_err(net)?;
        Ok((status, body))
    }
}

#[cfg(any(test, feature = "test-util"))]
pub use mock::MockTransport;

#[cfg(any(test, feature = "test-util"))]
mod mock {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    type Reply = Result<(u16, String), CoreError>;

    #[derive(Default)]
    struct State {
        routes: Vec<(String, VecDeque<Reply>)>,
        urls: Vec<String>,
        keys: Vec<String>,
    }

    /// Routes by path prefix (relative to the base URL, query ignored) to FIFO
    /// queues of responses. Longest matching prefix wins. An unrouted or
    /// exhausted request panics so tests fail loudly.
    #[derive(Default)]
    pub struct MockTransport {
        state: Mutex<State>,
    }

    impl MockTransport {
        pub fn new() -> Self {
            Self::default()
        }

        fn queue(&self, prefix: &str, r: Reply) {
            let mut s = self.state.lock().unwrap();
            if let Some((_, q)) = s.routes.iter_mut().find(|(p, _)| p == prefix) {
                q.push_back(r);
            } else {
                s.routes.push((prefix.to_string(), VecDeque::from([r])));
            }
        }

        pub fn push(&self, prefix: &str, status: u16, body: &str) {
            self.queue(prefix, Ok((status, body.to_string())));
        }

        pub fn push_network_error(&self, prefix: &str, msg: &str) {
            self.queue(prefix, Err(CoreError::Network(msg.to_string())));
        }

        pub fn requested_urls(&self) -> Vec<String> {
            self.state.lock().unwrap().urls.clone()
        }

        pub fn api_keys_seen(&self) -> Vec<String> {
            self.state.lock().unwrap().keys.clone()
        }
    }

    impl Transport for MockTransport {
        fn get(&self, url: &str, api_key: &str) -> Result<(u16, String), CoreError> {
            let mut s = self.state.lock().unwrap();
            s.urls.push(url.to_string());
            s.keys.push(api_key.to_string());
            let path = url
                .split_once("/api/v1")
                .map(|(_, p)| p)
                .unwrap_or(url)
                .split('?')
                .next()
                .unwrap_or("")
                .to_string();
            let best = s
                .routes
                .iter_mut()
                .filter(|(p, _)| path.starts_with(p.as_str()))
                .max_by_key(|(p, _)| p.len());
            match best.and_then(|(_, q)| q.pop_front()) {
                Some(r) => r,
                None => panic!("MockTransport: no queued response for path {path}"),
            }
        }
    }
}
