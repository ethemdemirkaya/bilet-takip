use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_LANGUAGE};
use std::thread::sleep;
use std::time::Duration;

const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36";

/// Tarayıcı gibi davranan, kibar (istekler arası bekleyen) HTTP istemcisi.
pub struct Http {
    client: Client,
    delay: Duration,
}

impl Http {
    pub fn new(timeout_secs: u64, delay_ms: u64) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,application/json;q=0.8,*/*;q=0.7"),
        );
        headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("tr-TR,tr;q=0.9,en;q=0.5"));
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .default_headers(headers)
            .cookie_store(true)
            .gzip(true)
            .brotli(true)
            .timeout(Duration::from_secs(timeout_secs))
            .build()?;
        Ok(Self { client, delay: Duration::from_millis(delay_ms) })
    }

    /// İstekler arası nezaket beklemesi.
    pub fn pause(&self) {
        sleep(self.delay);
    }

    /// Sayfayı metin olarak indirir. Geçici hatalarda 2 kez daha dener.
    pub fn get_text(&self, url: &str) -> Result<String> {
        let mut last_err = None;
        for attempt in 0..3 {
            if attempt > 0 {
                sleep(Duration::from_secs(2 * attempt));
            }
            match self.try_get(url) {
                Ok(body) => return Ok(body),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap())
    }

    fn try_get(&self, url: &str) -> Result<String> {
        let resp = self.client.get(url).send().with_context(|| format!("istek başarısız: {url}"))?;
        let status = resp.status();
        let body = resp.text().with_context(|| format!("yanıt okunamadı: {url}"))?;
        if body.contains("<title>Just a moment...</title>") {
            bail!("Cloudflare bot koruması engelledi ({status}): {url}");
        }
        if !status.is_success() {
            bail!("HTTP {status}: {url}");
        }
        Ok(body)
    }

    pub fn get_json(&self, url: &str) -> Result<serde_json::Value> {
        let text = self.get_text(url)?;
        serde_json::from_str(&text).with_context(|| format!("JSON çözülemedi: {url}"))
    }
}
