//! Bubilet: şehir sayfası (Next.js) içindeki RSC akışında `"citySlug":"<şehir>","events":[...]`
//! listesi gelir. Her etkinliğin seansları ve seans başına bilet kategorileri (indirimli fiyatlarıyla)
//! `platform.api.bubilet.com.tr` üzerinden alınır.

use crate::config::Config;
use crate::http::Http;
use crate::model::{fmt_date_short, guess_category, tl_to_kurus, tr_offset, Event, SourceId};
use anyhow::{Context, Result};
use chrono::{DateTime, Timelike};
use regex::Regex;
use serde::Deserialize;
use std::collections::BTreeMap;

const API: &str = "https://platform.api.bubilet.com.tr";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BbEvent {
    id: i64,
    name: String,
    slug: String,
    #[serde(default)]
    dates: Vec<String>,
    #[serde(default)]
    price: f64,
    discounted_price: Option<f64>,
    #[serde(default)]
    is_free_ticket: bool,
    #[serde(default)]
    is_sold_out: bool,
    #[serde(default)]
    is_marked_sold_out: bool,
    #[serde(default)]
    venues: Vec<BbVenue>,
}

#[derive(Deserialize)]
struct BbVenue {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sessions {
    sessions: Vec<Session>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    session_id: i64,
    #[serde(default)]
    city_slug: String,
    date: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Tickets {
    session_tickets: Vec<Ticket>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Ticket {
    seat_group_name: String,
    price: f64,
    discounted_price: Option<f64>,
    #[serde(default)]
    is_free_ticket: bool,
    #[serde(default)]
    remaining_tickets: i64,
    #[serde(default = "yes")]
    is_active: bool,
    #[serde(default)]
    is_marked_sold_out: bool,
}

fn yes() -> bool {
    true
}

pub fn fetch(http: &Http, cfg: &Config) -> Result<Vec<Event>> {
    let slug = &cfg.city_slug;
    let html = http.get_text(&format!("https://www.bubilet.com.tr/{slug}"))?;
    let stream = rsc_stream(&html)?;
    let mut events = parse_stream(&stream, slug)?;
    let city_id = city_id(&stream, slug).context("Bubilet: şehir kimliği bulunamadı (site yapısı değişmiş olabilir)")?;

    // Bilet kategorileri alınamayan etkinlik bu taramada atlanır: eksik veriyle karşılaştırılıp
    // yanlış bildirim gitmesin, önceki kaydı olduğu gibi kalsın.
    events.retain_mut(|ev| {
        if ev.tiers.is_empty() {
            return true; // ücretsiz ya da fiyatsız
        }
        match fetch_tiers(http, ev, city_id, slug) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("  Bubilet bilet kategorileri alınamadı ({}): {e:#}", ev.title);
                false
            }
        }
    });
    Ok(events)
}

/// Etkinliğin şehirdeki tüm seanslarının bilet kategorilerini alıp `ev`'e yazar.
fn fetch_tiers(http: &Http, ev: &mut Event, city_id: i64, slug: &str) -> Result<()> {
    http.pause();
    let sessions: Sessions = serde_json::from_value(http.get_json(&format!("{API}/v2/event/{}/city/{city_id}/sessions", ev.id))?)
        .context("seans listesi çözülemedi")?;
    let sessions: Vec<Session> = sessions.sessions.into_iter().filter(|s| s.city_slug.is_empty() || s.city_slug == slug).collect();

    let mut all = Vec::new();
    for s in &sessions {
        http.pause();
        let tickets: Tickets =
            serde_json::from_value(http.get_json(&format!("{API}/v2/session/{}/city/{city_id}/tickets", s.session_id))?)
                .context("bilet listesi çözülemedi")?;
        // Birden fazla seans varsa kategoriler seansa göre ayrılır: "22 Kas 12:30 · Protokol"
        let prefix = if sessions.len() > 1 {
            DateTime::parse_from_rfc3339(&s.date).ok().map(|d| {
                let d = d.with_timezone(&tr_offset());
                format!("{} {:02}:{:02} · ", fmt_date_short(&d), d.hour(), d.minute())
            })
        } else {
            None
        };
        all.push((prefix.unwrap_or_default(), tickets.session_tickets));
    }
    apply_tickets(ev, &all);
    Ok(())
}

/// Seansların biletlerini etkinliğin kategorilerine çevirir. Satışta bilet kalmadıysa tükendi sayılır.
fn apply_tickets(ev: &mut Event, sessions: &[(String, Vec<Ticket>)]) {
    let mut tiers: BTreeMap<String, i64> = BTreeMap::new();
    let mut list_prices: BTreeMap<String, i64> = BTreeMap::new();
    let mut any_ticket = false;
    for (prefix, tickets) in sessions {
        for t in tickets {
            any_ticket = true;
            if t.is_free_ticket || !t.is_active || t.is_marked_sold_out || t.remaining_tickets <= 0 {
                continue;
            }
            let now = t.discounted_price.filter(|p| *p > 0.0).unwrap_or(t.price);
            if now <= 0.0 {
                continue;
            }
            let name = format!("{prefix}{}", t.seat_group_name.trim());
            let (now, list) = (tl_to_kurus(now), tl_to_kurus(t.price));
            // Aynı adlı birden fazla bilet varsa en ucuzu
            if tiers.get(&name).is_some_and(|p| *p <= now) {
                continue;
            }
            tiers.insert(name.clone(), now);
            if list > now {
                list_prices.insert(name, list);
            } else {
                list_prices.remove(&name);
            }
        }
    }
    if !any_ticket {
        return; // bilet listesi boş: liste sayfasındaki fiyat kalsın
    }
    ev.sold_out = ev.sold_out || tiers.is_empty();
    ev.tiers = tiers;
    ev.list_prices = list_prices;
}

/// Next.js RSC parçalarını (self.__next_f.push([1,"<JSON string>"])) tek metinde birleştirir.
fn rsc_stream(html: &str) -> Result<String> {
    let re = Regex::new(r#"self\.__next_f\.push\(\[1,"((?:[^"\\]|\\.)*)"\]\)"#).unwrap();
    let mut stream = String::new();
    for cap in re.captures_iter(html) {
        let chunk: String = serde_json::from_str(&format!("\"{}\"", &cap[1])).context("Bubilet: RSC parçası çözülemedi")?;
        stream.push_str(&chunk);
    }
    Ok(stream)
}

fn city_id(stream: &str, city_slug: &str) -> Option<i64> {
    let re = Regex::new(&format!(r#""cityId":(\d+),"cityName":"[^"]*","citySlug":"{}""#, regex::escape(city_slug))).unwrap();
    re.captures(stream)?[1].parse().ok()
}

/// Şehir sayfasındaki etkinlik listesi (bilet kategorileri olmadan, sadece en uygun fiyatla).
#[cfg(test)]
fn parse(html: &str, city_slug: &str) -> Result<Vec<Event>> {
    parse_stream(&rsc_stream(html)?, city_slug)
}

fn parse_stream(stream: &str, city_slug: &str) -> Result<Vec<Event>> {
    let marker = format!("\"citySlug\":\"{city_slug}\",\"events\":");
    let pos = stream.find(&marker).context("Bubilet: sayfada etkinlik listesi bulunamadı (site yapısı değişmiş olabilir)")?;
    let mut de = serde_json::Deserializer::from_str(&stream[pos + marker.len()..]).into_iter::<Vec<BbEvent>>();
    let events = de.next().context("Bubilet: etkinlik listesi boş")?.context("Bubilet: etkinlik listesi çözülemedi")?;

    Ok(events
        .into_iter()
        .map(|e| {
            let current = e.discounted_price.filter(|p| *p > 0.0).unwrap_or(e.price);
            let mut tiers = BTreeMap::new();
            let mut list_prices = BTreeMap::new();
            if !e.is_free_ticket && current > 0.0 {
                tiers.insert("En uygun bilet".to_string(), tl_to_kurus(current));
                if e.price > current {
                    list_prices.insert("En uygun bilet".to_string(), tl_to_kurus(e.price));
                }
            }
            let title = e.name.trim().to_string();
            Event {
                source: SourceId::Bubilet,
                id: e.id.to_string(),
                category: guess_category(&title),
                title,
                venue: e.venues.first().map(|v| v.name.trim().to_string()).unwrap_or_default(),
                date: e.dates.first().and_then(|d| DateTime::parse_from_rfc3339(d).ok()),
                url: format!("https://www.bubilet.com.tr/{city_slug}/etkinlik/{}", e.slug),
                tiers,
                sold_out: e.is_sold_out || e.is_marked_sold_out,
                list_prices,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fixture() {
        let html = std::fs::read_to_string("fixtures/bubilet_kayseri.html").unwrap();
        let events = parse(&html, "kayseri").unwrap();
        assert!(events.len() >= 20, "{} etkinlik", events.len());
        let blok3 = events.iter().find(|e| e.title == "Blok3").unwrap();
        assert_eq!(blok3.min_price(), Some(70_000));
        assert_eq!(blok3.url, "https://www.bubilet.com.tr/kayseri/etkinlik/-blok3-");
        assert!(blok3.venue.starts_with("Kumsmall"));
        assert!(blok3.date.is_some());
        assert_eq!(blok3.discounts(), vec![("En uygun bilet".into(), 100_000, 70_000)]);
        let karsu = events.iter().find(|e| e.title == "Karsu").unwrap();
        assert!(karsu.discounts().is_empty());
        assert_eq!(city_id(&rsc_stream(&html).unwrap(), "kayseri"), Some(38));
    }

    #[test]
    fn parses_sessions_fixture() {
        let s: Sessions = serde_json::from_str(&std::fs::read_to_string("fixtures/bubilet_sessions_14662.json").unwrap()).unwrap();
        assert_eq!(s.sessions.len(), 1);
        assert_eq!(s.sessions[0].session_id, 288145);
        assert_eq!(s.sessions[0].city_slug, "kayseri");
    }

    fn emir_can() -> Event {
        let html = std::fs::read_to_string("fixtures/bubilet_kayseri.html").unwrap();
        parse(&html, "kayseri").unwrap().into_iter().find(|e| e.id == "14662").unwrap()
    }

    #[test]
    fn tickets_give_per_block_discounts() {
        let t: Tickets = serde_json::from_str(&std::fs::read_to_string("fixtures/bubilet_tickets_288145.json").unwrap()).unwrap();
        let mut ev = emir_can();
        apply_tickets(&mut ev, &[(String::new(), t.session_tickets)]);
        assert_eq!(ev.tiers.len(), 27);
        assert_eq!(ev.tiers.get("A1 Blok-Protokol"), Some(&500_000));
        let d = ev.discounts();
        assert!(d.contains(&("B3 Blok-1. Kategori".into(), 385_000, 269_500)), "{d:?}");
        assert!(d.contains(&("D3 Balkon-6. Kategori".into(), 155_000, 108_500)));
        assert!(!d.iter().any(|(n, ..)| n == "B1 Blok-1. Kategori"), "indirimsiz blok");
        assert_eq!(ev.min_price(), Some(108_500));
    }

    fn ticket(name: &str, price: f64, discounted: f64, remaining: i64) -> Ticket {
        Ticket {
            seat_group_name: name.into(),
            price,
            discounted_price: Some(discounted),
            is_free_ticket: false,
            remaining_tickets: remaining,
            is_active: true,
            is_marked_sold_out: false,
        }
    }

    #[test]
    fn sold_out_tickets_skipped_and_sessions_prefixed() {
        let mut ev = emir_can();
        apply_tickets(
            &mut ev,
            &[
                ("26 Eyl 13:00 · ".into(), vec![ticket("Tam Bilet", 600.0, 600.0, 17), ticket("Erken", 400.0, 400.0, 0)]),
                ("4 Eki 16:30 · ".into(), vec![ticket("Tam Bilet", 600.0, 450.0, 21)]),
            ],
        );
        assert_eq!(ev.tiers.keys().collect::<Vec<_>>(), ["26 Eyl 13:00 · Tam Bilet", "4 Eki 16:30 · Tam Bilet"]);
        assert_eq!(ev.discounts(), vec![("4 Eki 16:30 · Tam Bilet".into(), 60_000, 45_000)]);
        assert!(!ev.sold_out);

        apply_tickets(&mut ev, &[(String::new(), vec![ticket("Tam Bilet", 600.0, 600.0, 0)])]);
        assert!(ev.sold_out && ev.tiers.is_empty());
    }
}
