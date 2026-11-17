mod transport;
mod types;

#[cfg(any(test, feature = "test-util"))]
pub use transport::MockTransport;
pub use transport::{ReqwestTransport, Transport};
pub use types::{Envelope, RizeniDef, StavUctu};

use crate::error::CoreError;
use serde::de::DeserializeOwned;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const DEFAULT_BASE_URL: &str = "https://api-kn.cuzk.gov.cz/api/v1";
const MAX_RETRIES: u32 = 2;

pub struct ApiClient {
    transport: Arc<dyn Transport>,
    api_key: String,
    base_url: String,
    retry_delay: Duration,
    calls: AtomicU32,
}

impl ApiClient {
    pub fn new(transport: Arc<dyn Transport>, api_key: String) -> Self {
        Self {
            transport,
            api_key,
            base_url: DEFAULT_BASE_URL.to_string(),
            retry_delay: Duration::from_secs(2),
            calls: AtomicU32::new(0),
        }
    }

    pub fn with_retry_delay(mut self, d: Duration) -> Self {
        self.retry_delay = d;
        self
    }

    pub fn with_base_url(mut self, url: &str) -> Self {
        self.base_url = url.trim_end_matches('/').to_string();
        self
    }

    /// Number of HTTP attempts made, retries included.
    pub fn calls(&self) -> u32 {
        self.calls.load(Ordering::SeqCst)
    }

    fn build_url(&self, path: &str, query: &[(&str, String)]) -> String {
        let mut url = format!("{}{}", self.base_url, path);
        for (i, (k, v)) in query.iter().enumerate() {
            url.push(if i == 0 { '?' } else { '&' });
            url.push_str(&urlencoding::encode(k));
            url.push('=');
            url.push_str(&urlencoding::encode(v));
        }
        url
    }

    fn request_body(&self, path: &str, query: &[(&str, String)]) -> Result<String, CoreError> {
        let url = self.build_url(path, query);
        for attempt in 0..=MAX_RETRIES {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let last = attempt == MAX_RETRIES;
            match self.transport.get(&url, &self.api_key) {
                Err(e) => {
                    if last {
                        return Err(e);
                    }
                }
                Ok((status, body)) => match status {
                    200..=299 => return Ok(body),
                    404 => return Err(CoreError::NotFound),
                    401 | 403 => return Err(CoreError::InvalidApiKey),
                    s if (s == 429 || s >= 500) && !last => {}
                    s => return Err(CoreError::Http { status: s, body }),
                },
            }
            std::thread::sleep(self.retry_delay * 2u32.pow(attempt));
        }
        unreachable!("loop always returns on the last attempt")
    }

    pub fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Envelope<T>, CoreError> {
        let body = self.request_body(path, query)?;
        serde_json::from_str(&body).map_err(|e| CoreError::Http {
            status: 200,
            body: format!("Neplatná odpověď API: {e}"),
        })
    }

    /// `StavUctu` is NOT wrapped in the envelope. Its `provedenoVolani` does
    /// not include this call itself.
    pub fn stav_uctu(&self) -> Result<StavUctu, CoreError> {
        let body = self.request_body("/AplikacniSluzby/StavUctu", &[])?;
        serde_json::from_str(&body).map_err(|e| CoreError::Http {
            status: 200,
            body: format!("Neplatná odpověď StavUctu: {e}"),
        })
    }
}
