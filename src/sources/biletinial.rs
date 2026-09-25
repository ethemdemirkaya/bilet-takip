//! Biletinial: kategori+şehir liste sayfasındaki JSON-LD ItemList'ten etkinlik linkleri alınır,
//! her etkinliğin detay sayfasındaki seanslardan (JSON-LD Event + data-ticketPrices) şehre ait olanlar çıkarılır.

use crate::config::Config;
use crate::http::Http;
use crate::model::{fold, guess_category, parse_tl, tr_offset, tl_to_kurus, Event, SourceId};
use anyhow::{bail, Result};
use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone};
use scraper::{Html, Selector};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

const BASE: &str = "https://biletinial.com";

pub fn fetch(http: &Http, cfg: &Config) -> Result<Vec<Event>> {
    let mut links: Vec<(String, String)> = Vec::new(); // (url, kategori)
    for cat in &cfg.sources.biletinial_categories {
        let html = http.get_text(&format!("{BASE}/tr-tr/{cat}/{}", cfg.city_slug))?;
        for url in parse_list(&html) {
            if !links.iter().any(|(u, _)| *u == url) {
                links.push((url, cat.clone()));
            }
        }
        http.pause();
    }

    let mut events = Vec::new();
    let mut failures = 0;
    for (url, cat) in &links {
        match http.get_text(url) {
            Ok(html) => events.extend(parse_detail(&html, url, cat, &cfg.city)),
            Err(e) => {
                eprintln!("  Biletinial detay alınamadı: {e:#}");
                failures += 1;
            }
        }
        http.pause();
    }
    if !links.is_empty() && failures == links.len() {
        bail!("Biletinial: hiçbir detay sayfası alınamadı");
    }
    Ok(events)
}

fn ld_json_blocks(doc: &Html) -> Vec<Value> {
    let sel = Selector::parse(r#"script[type="application/ld+json"]"#).unwrap();
    doc.select(&sel)
        .filter_map(|s| serde_json::from_str::<Value>(&s.inner_html()).ok())
        .flat_map(|v| match v {
            Value::Array(a) => a,
            v => vec![v],
        })
        .collect()
}

/// Liste sayfasındaki etkinlik URL'leri.
pub fn parse_list(html: &str) -> Vec<String> {
    let doc = Html::parse_document(html);
    ld_json_blocks(&doc)
        .iter()
        .filter(|v| v["@type"] == "ItemList")
        .flat_map(|v| v["itemListElement"].as_array().cloned().unwrap_or_default())
        .filter_map(|item| item["url"].as_str().map(str::to_string))
        .collect()
}

#[derive(Deserialize)]
struct TicketPrices {
    venue_name: String,
    #[serde(default)]
    prices: Vec<TicketPrice>,
}

#[derive(Deserialize)]
struct TicketPrice {
    name: String,
    price: String,
}

/// Detay sayfasındaki seanslardan verilen şehirdekiler.
pub fn parse_detail(html: &str, url: &str, cat: &str, city: &str) -> Vec<Event> {
    let doc = Html::parse_document(html);
    let city_f = fold(city);

    // Seans başına kategori fiyatları: (seans zamanı "YYYY-MM-DDTHH:MM", mekan) -> kategoriler
    let block_sel = Selector::parse("div.ed-biletler__sehir__gun__fiyat").unwrap();
    let valid_sel = Selector::parse(r#"meta[itemprop="validFrom"]"#).unwrap();
    let tip_sel = Selector::parse("a.ticket_price_tooltip").unwrap();
    let mut session_prices: Vec<(String, String, BTreeMap<String, i64>)> = Vec::new();
    for block in doc.select(&block_sel) {
        let Some(when) = block.select(&valid_sel).next().and_then(|m| m.value().attr("content")) else { continue };
        let Some(tp) = block
            .select(&tip_sel)
            .next()
            .and_then(|a| a.value().attr("data-ticketprices"))
            .and_then(|j| serde_json::from_str::<TicketPrices>(j).ok())
        else {
            continue;
        };
        let tiers = tp
            .prices
            .iter()
            .filter_map(|p| parse_tl(&p.price).filter(|k| *k > 0).map(|k| (p.name.trim().to_string(), k)))
            .collect();
        session_prices.push((when.chars().take(16).collect(), fold(&tp.venue_name), tiers));
    }

    let slug = url.trim_end_matches('/').rsplit('/').next().unwrap_or(url);
    let mut out = Vec::new();
    for ev in ld_json_blocks(&doc).iter().filter(|v| v["@type"] == "Event") {
        let loc = &ev["location"];
        let locality = loc["address"]["addressLocality"].as_str().unwrap_or("");
        if fold(locality.trim()) != city_f {
            continue;
        }
        let Some(start) = ev["startDate"].as_str() else { continue };
        let date = parse_date(start);
        let when: String = start.chars().take(16).collect();
        let venue = loc["name"].as_str().unwrap_or("").trim().to_string();
        let offers = &ev["offers"];
        let availability = offers["availability"].as_str().unwrap_or("");
        let sold_out = availability.contains("SoldOut") || availability.contains("OutOfStock");

        let venue_f = fold(&venue);
        let mut tiers = session_prices
            .iter()
            .find(|(w, v, _)| *w == when && (v.contains(&venue_f) || v.ends_with(&city_f)))
            .map(|(_, _, t)| t.clone())
            .unwrap_or_default();
        if tiers.is_empty() && !sold_out {
            if let Some(p) = offers["price"].as_f64().filter(|p| *p > 0.0) {
                tiers.insert("En uygun bilet".to_string(), tl_to_kurus(p));
            }
        }

        let title = ev["name"].as_str().unwrap_or(slug).trim().to_string();
        let category = match cat {
            "muzik" => "Konser".to_string(),
            "tiyatro" if !fold(&title).contains("stand") => "Tiyatro".to_string(),
            _ => guess_category(&title),
        };
        out.push(Event {
            source: SourceId::Biletinial,
            id: format!("{slug}@{when}"),
            title,
            venue,
            date,
            category,
            url: url.to_string(),
            tiers,
            sold_out,
            list_prices: Default::default(),
        });
    }
    out
}

fn parse_date(s: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(s).ok().or_else(|| {
        let naive = NaiveDateTime::parse_from_str(&s.chars().take(16).collect::<String>(), "%Y-%m-%dT%H:%M").ok()?;
        tr_offset().from_local_datetime(&naive).single()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_list_fixture() {
        let html = std::fs::read_to_string("fixtures/biletinial_list_muzik.html").unwrap();
        let links = parse_list(&html);
        assert!(links.len() >= 5);
        assert!(links.contains(&"https://biletinial.com/tr-tr/muzik/karsu-konseri".to_string()));
    }

    #[test]
    fn parses_detail_fixture_only_city_sessions() {
        let html = std::fs::read_to_string("fixtures/biletinial_detail_karsu.html").unwrap();
        let url = "https://biletinial.com/tr-tr/muzik/karsu-konseri";
        let events = parse_detail(&html, url, "muzik", "Kayseri");
        assert_eq!(events.len(), 1, "sadece Kayseri seansı gelmeli");
        let e = &events[0];
        assert_eq!(e.id, "karsu-konseri@2026-11-27T21:00");
        assert!(e.venue.contains("Erciyes"));
        assert_eq!(e.min_price(), Some(145_000));
        assert!(e.tiers.len() >= 5, "kategori fiyatları: {:?}", e.tiers);
        assert_eq!(e.category, "Konser");
    }
}
