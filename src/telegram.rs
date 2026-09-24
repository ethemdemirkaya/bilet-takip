use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::json;
use std::thread::sleep;
use std::time::Duration;

/// Telegram tek mesaj sınırı 4096 karakter; biraz pay bırakıyoruz.
const MAX_LEN: usize = 3900;

pub struct Telegram {
    client: Client,
    token: String,
    pub chat_id: String,
    dry_run: bool,
}

#[derive(Debug, Deserialize)]
pub struct Update {
    pub update_id: i64,
    pub message: Option<Message>,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    pub chat: Chat,
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Chat {
    pub id: i64,
}

impl Telegram {
    pub fn new(token: String, chat_id: String, dry_run: bool) -> Result<Self> {
        let client = Client::builder().timeout(Duration::from_secs(45)).build()?;
        Ok(Self { client, token, chat_id, dry_run })
    }

    fn url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{method}", self.token)
    }

    /// HTML biçimli mesaj gönderir; çok uzunsa satır sınırlarından böler.
    pub fn send(&self, html: &str) -> Result<()> {
        for part in split(html) {
            self.send_one(&part)?;
            sleep(Duration::from_millis(1100)); // sohbet başına ~1 mesaj/sn sınırı
        }
        Ok(())
    }

    fn send_one(&self, html: &str) -> Result<()> {
        if self.dry_run {
            println!("──── [TELEGRAM] ────\n{html}\n");
            return Ok(());
        }
        let body = json!({
            "chat_id": self.chat_id,
            "text": html,
            "parse_mode": "HTML",
            "link_preview_options": {"is_disabled": true},
        });
        for _ in 0..3 {
            let resp: serde_json::Value = self.client.post(self.url("sendMessage")).json(&body).send()?.json()?;
            if resp["ok"] == true {
                return Ok(());
            }
            if let Some(wait) = resp["parameters"]["retry_after"].as_u64() {
                sleep(Duration::from_secs(wait + 1));
                continue;
            }
            bail!("Telegram sendMessage hatası: {resp}");
        }
        bail!("Telegram sendMessage: çok fazla deneme")
    }

    /// `wait_secs` > 0 ise long polling: yeni mesaj gelene kadar en fazla bu kadar bekler.
    pub fn get_updates(&self, offset: i64, wait_secs: u64) -> Result<Vec<Update>> {
        let resp: serde_json::Value = self
            .client
            .get(self.url("getUpdates"))
            .query(&[("offset", offset.to_string()), ("timeout", wait_secs.to_string()), ("allowed_updates", "[\"message\"]".into())])
            .send()?
            .json()?;
        if resp["ok"] != true {
            bail!("Telegram getUpdates hatası: {resp}");
        }
        serde_json::from_value(resp["result"].clone()).context("getUpdates yanıtı çözülemedi")
    }
}

fn split(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    for line in text.split_inclusive('\n') {
        if cur.chars().count() + line.chars().count() > MAX_LEN && !cur.is_empty() {
            parts.push(std::mem::take(&mut cur));
        }
        cur.push_str(line);
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

#[cfg(test)]
mod tests {
    #[test]
    fn splits_long_text_on_lines() {
        let text = "satır\n".repeat(2000);
        let parts = super::split(&text);
        assert!(parts.len() > 1);
        assert!(parts.iter().all(|p| p.chars().count() <= super::MAX_LEN));
        assert_eq!(parts.concat(), text);
    }
}
