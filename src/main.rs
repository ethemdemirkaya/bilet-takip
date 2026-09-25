//! Bilet Takip: Kayseri etkinlik bilet fiyatlarını tarar, değişiklikleri Telegram'dan bildirir.
//!
//! Kullanım:
//!   bilet-takip                 bir kez tarar, bekleyen komutları cevaplar, çıkar
//!   bilet-takip --listen 19     bir kez tarar, sonra 19 dakika Telegram'ı dinler (GitHub Actions)
//!   bilet-takip --loop 20       hiç kapanmaz: Telegram'ı sürekli dinler, 20 dakikada bir tarar (PC / sunucu)
//!   bilet-takip --dry-run       Telegram'a göndermez, mesajları konsola yazar
//!   bilet-takip chat-id         bota yazanların chat ID'lerini gösterir

mod config;
mod diff;
mod http;
mod matcher;
mod messages;
mod model;
mod sources;
mod store;
mod telegram;

use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use config::{Config, Thresholds};
use diff::Change;
use model::{tr_offset, SourceId};
use std::path::{Path, PathBuf};
use std::time::Instant;
use store::State;
use telegram::Telegram;

const STATE_PATH: &str = "state/state.json";
/// Tek taramada gönderilecek en fazla değişiklik mesajı (kalanı özetlenir).
const MAX_CHANGE_MESSAGES: usize = 25;
/// Telegram long polling bekleme süresi (saniye).
const POLL_SECS: u64 = 20;

fn main() {
    use_exe_dir_if_needed();
    load_dotenv();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("HATA: {e:#}");
        std::process::exit(1);
    }
}

fn flag_minutes(args: &[String], name: &str) -> Option<u64> {
    let i = args.iter().position(|a| a == name)?;
    Some(args.get(i + 1).and_then(|m| m.parse().ok()).unwrap_or(20))
}

fn run(args: &[String]) -> Result<()> {
    let dry_run = args.iter().any(|a| a == "--dry-run");
    let token = std::env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default();
    let chat_id = std::env::var("TELEGRAM_CHAT_ID").unwrap_or_default();
    if !dry_run && token.is_empty() {
        anyhow::bail!("TELEGRAM_BOT_TOKEN tanımlı değil (.env ya da ortam değişkeni)");
    }
    let tg = Telegram::new(token, chat_id, dry_run)?;

    if args.iter().any(|a| a == "chat-id") {
        for u in tg.get_updates(0, 0)? {
            if let Some(m) = u.message {
                println!("chat_id = {}  mesaj: {:?}", m.chat.id, m.text.unwrap_or_default());
            }
        }
        return Ok(());
    }
    if !dry_run && tg.chat_id.is_empty() {
        anyhow::bail!("TELEGRAM_CHAT_ID tanımlı değil. Bota /start yazıp `bilet-takip chat-id` çalıştırın.");
    }

    let cfg = Config::load(Path::new("config.toml"))?;
    let state_path = PathBuf::from(STATE_PATH);
    let mut app = App {
        state: State::load(&state_path)?,
        http: http::Http::new(cfg.http.timeout_secs, cfg.http.delay_ms)?,
        enabled: sources::enabled(&cfg),
        cfg,
        tg,
        state_path,
    };

    let loop_every = flag_minutes(args, "--loop").map(|m| std::time::Duration::from_secs(m.max(5) * 60));
    let listen_for = flag_minutes(args, "--listen").map(|m| std::time::Duration::from_secs(m * 60));

    if let Some(every) = loop_every {
        eprintln!("Sürekli mod: Telegram sürekli dinleniyor, siteler {} dakikada bir taranıyor. Durdurmak için Ctrl+C.", every.as_secs() / 60);
    }

    // İlk tarama her modda hemen yapılır.
    app.scan_logged(loop_every.is_some())?;
    if dry_run {
        return Ok(());
    }

    let started = Instant::now();
    let mut next_scan = loop_every.map(|e| Instant::now() + e);
    loop {
        if let Some(limit) = listen_for {
            if started.elapsed() >= limit {
                break;
            }
        }
        if loop_every.is_none() && listen_for.is_none() {
            app.poll(0)?; // tek seferlik: sadece bekleyen komutlar
            break;
        }

        match app.poll(POLL_SECS) {
            Ok(true) => app.scan_logged(true)?, // /tara komutu
            Ok(false) => {}
            Err(e) => {
                eprintln!("Telegram dinlenemedi: {e:#}");
                std::thread::sleep(std::time::Duration::from_secs(10));
            }
        }

        if let (Some(every), Some(at)) = (loop_every, next_scan) {
            if Instant::now() >= at {
                app.scan_logged(true)?;
                next_scan = Some(Instant::now() + every);
            }
        }
    }
    Ok(())
}

struct App {
    cfg: Config,
    tg: Telegram,
    http: http::Http,
    state: State,
    state_path: PathBuf,
    enabled: Vec<SourceId>,
}

impl App {
    /// Tarar; sürekli modda hata programı durdurmaz, sadece yazılır.
    fn scan_logged(&mut self, keep_going: bool) -> Result<()> {
        eprintln!("\n=== Tarama {} ===", chrono::Local::now().format("%d.%m.%Y %H:%M"));
        match self.scan() {
            Err(e) if keep_going => {
                eprintln!("HATA: {e:#}");
                Ok(())
            }
            r => r,
        }
    }

    fn today(&self) -> chrono::NaiveDate {
        Utc::now().with_timezone(&tr_offset()).date_naive()
    }

    fn save(&mut self) -> Result<()> {
        if self.state.keepalive.map(|k| Utc::now() - k > Duration::days(7)).unwrap_or(true) {
            self.state.keepalive = Some(Utc::now());
        }
        self.state.save(&self.state_path).context("durum kaydedilemedi")
    }

    /// Tüm kaynakları tarar, değişiklikleri bildirir ve durumu kaydeder.
    fn scan(&mut self) -> Result<()> {
        let today = self.today();
        let thresholds = Thresholds {
            min_drop_percent: self.state.min_drop_percent_override.unwrap_or(self.cfg.thresholds.min_drop_percent),
            min_drop_tl: self.cfg.thresholds.min_drop_tl,
        };
        let mut changes: Vec<Change> = Vec::new();
        let mut alerts: Vec<String> = Vec::new();
        let mut started: Vec<(SourceId, usize)> = Vec::new();

        for &src in &self.enabled {
            eprintln!("→ {src} taranıyor...");
            let result = sources::fetch(src, &self.http, &self.cfg).and_then(|evs| {
                if evs.is_empty() {
                    anyhow::bail!("hiç etkinlik dönmedi (site yapısı değişmiş olabilir)")
                }
                Ok(evs)
            });
            let health = self.state.health.entry(src).or_default();
            match result {
                Ok(evs) => {
                    eprintln!("  {} etkinlik", evs.len());
                    if health.alerted {
                        alerts.push(messages::source_up(src));
                    }
                    *health = store::Health { last_count: evs.len(), ..Default::default() };
                    let first = !self.state.initialized_sources.contains(&src);
                    let count = evs.len();
                    changes.extend(diff::apply(&mut self.state, src, evs, &thresholds, today));
                    if first {
                        started.push((src, count));
                    }
                }
                Err(e) => {
                    eprintln!("  HATA: {e:#}");
                    health.consecutive_failures += 1;
                    health.last_error = Some(format!("{e:#}").chars().take(300).collect());
                    if health.consecutive_failures >= 3 && !health.alerted {
                        health.alerted = true;
                        alerts.push(messages::source_down(src, health));
                    }
                }
            }
        }
        self.state.prune(today);
        // Bildirim göndermeden önce kaydet: gönderim yarıda kalırsa aynı değişiklik tekrar bildirilmez.
        self.save()?;

        let notify = &self.cfg.notify;
        let changes: Vec<Change> = changes
            .into_iter()
            .filter(|c| match c {
                Change::PriceDrop(..) => notify.price_drop,
                Change::Discount(_) => notify.discount,
                Change::New(_) => notify.new_event,
                Change::BackInStock(_) => notify.back_in_stock,
            })
            .collect();
        eprintln!("{} değişiklik", changes.len());

        if !started.is_empty() {
            let list: Vec<String> = started.iter().map(|(s, n)| format!("{s}: {n} etkinlik")).collect();
            self.tg.send(&format!(
                "✅ <b>Takip başladı</b> ({})\n{}\n\nBundan sonra indirimleri, fiyat düşüşlerini ve tekrar satışa çıkan biletleri bildireceğim. /yardim",
                self.cfg.city,
                list.join("\n")
            ))?;
        }
        for c in changes.iter().take(MAX_CHANGE_MESSAGES) {
            self.tg.send(&messages::change(c, &self.state))?;
        }
        if changes.len() > MAX_CHANGE_MESSAGES {
            self.tg.send(&format!("…ve {} değişiklik daha. Hepsi için /liste", changes.len() - MAX_CHANGE_MESSAGES))?;
        }
        for a in &alerts {
            self.tg.send(a)?;
        }
        Ok(())
    }

    /// Telegram komutlarını bekler ve cevaplar. `/tara` geldiyse true döner.
    fn poll(&mut self, wait_secs: u64) -> Result<bool> {
        let updates = self.tg.get_updates(self.state.telegram_offset, wait_secs)?;
        if updates.is_empty() {
            return Ok(false);
        }
        let today = self.today();
        let mut scan_requested = false;
        for u in updates {
            self.state.telegram_offset = u.update_id + 1;
            let Some(msg) = u.message else { continue };
            if msg.chat.id.to_string() != self.tg.chat_id {
                continue; // sadece sahibine cevap ver
            }
            let text = msg.text.unwrap_or_default();
            let mut parts = text.trim().splitn(2, ' ');
            let cmd = parts.next().unwrap_or("").split('@').next().unwrap_or("").to_lowercase();
            let arg = parts.next().map(str::trim).filter(|a| !a.is_empty());
            let cfg = &self.cfg;
            let percent = self.state.min_drop_percent_override.unwrap_or(cfg.thresholds.min_drop_percent);
            let reply = match cmd.as_str() {
                "/start" | "/yardim" | "/help" => messages::HELP.to_string(),
                "/liste" => messages::list(&self.state, today, None),
                "/indirim" => messages::discounts(&self.state, today),
                "/ara" => match arg {
                    Some(q) => messages::list(&self.state, today, Some(q)),
                    None => "Kullanım: /ara karsu".into(),
                },
                "/durum" => messages::status(&self.state, &self.enabled, percent, cfg.thresholds.min_drop_tl),
                "/tara" => {
                    scan_requested = true;
                    "🔎 Tarama başlıyor, değişiklik olursa haber vereceğim.".into()
                }
                "/esik" => match arg.and_then(|a| a.trim_start_matches('%').replace(',', ".").parse::<f64>().ok()) {
                    Some(p) if (0.0..=90.0).contains(&p) => {
                        self.state.min_drop_percent_override = Some(p);
                        format!(
                            "✅ Bildirim eşiği %{p} olarak ayarlandı (ya da {} düşüş).",
                            model::fmt_tl((cfg.thresholds.min_drop_tl * 100.0) as i64)
                        )
                    }
                    _ => format!("Şu anki eşik: %{percent}\nKullanım: /esik 10"),
                },
                _ => "Bilinmeyen komut. /yardim".into(),
            };
            self.tg.send(&reply)?;
        }
        self.save()?;
        Ok(scan_requested)
    }
}

/// Exe çift tıklanınca çalışma klasörü farklı olabilir; config.toml'u exe'nin yanında ara.
fn use_exe_dir_if_needed() {
    if Path::new("config.toml").exists() {
        return;
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        if dir.join("config.toml").exists() {
            let _ = std::env::set_current_dir(dir);
        }
    }
}

/// Basit .env okuyucu (yerel çalıştırma için). Var olan ortam değişkenlerini ezmez.
fn load_dotenv() {
    let Ok(text) = std::fs::read_to_string(".env") else { return };
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let (k, v) = (k.trim(), v.trim().trim_matches('"'));
            if !k.is_empty() && std::env::var_os(k).is_none() {
                std::env::set_var(k, v);
            }
        }
    }
}
