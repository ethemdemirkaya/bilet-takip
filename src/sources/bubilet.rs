//! Bubilet: şehir sayfası (Next.js) içindeki RSC akışında `"citySlug":"<şehir>","events":[...]`
//! listesi fiyatlarıyla birlikte gelir. Tek istekle tüm şehir.

use crate::config::Config;
use crate::http::Http;
use crate::model::{guess_category, tl_to_kurus, Event, SourceId};
use anyhow::{Context, Result};
use chrono::DateTime;
use regex::Regex;
use serde::Deserialize;
use std::collections::BTreeMap;

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

pub fn fetch(http: &Http, cfg: &Config) -> Result<Vec<Event>> {
    let html = http.get_text(&format!("https://www.bubilet.com.tr/{}", cfg.city_slug))?;
    parse(&html, &cfg.city_slug)
}

pub fn parse(html: &str, city_slug: &str) -> Result<Vec<Event>> {
    // Next.js RSC parçaları: self.__next_f.push([1,"<JSON string>"])
    let re = Regex::new(r#"self\.__next_f\.push\(\[1,"((?:[^"\\]|\\.)*)"\]\)"#).unwrap();
    let mut stream = String::new();
    for cap in re.captures_iter(html) {
        let chunk: String = serde_json::from_str(&format!("\"{}\"", &cap[1])).context("Bubilet: RSC parçası çözülemedi")?;
        stream.push_str(&chunk);
    }

    let marker = format!("\"citySlug\":\"{city_slug}\",\"events\":");
    let pos = stream.find(&marker).context("Bubilet: sayfada etkinlik listesi bulunamadı (site yapısı değişmiş olabilir)")?;
    let mut de = serde_json::Deserializer::from_str(&stream[pos + marker.len()..]).into_iter::<Vec<BbEvent>>();
    let events = de.next().context("Bubilet: etkinlik listesi boş")?.context("Bubilet: etkinlik listesi çözülemedi")?;

    Ok(events
        .into_iter()
        .map(|e| {
            let current = e.discounted_price.filter(|p| *p > 0.0).unwrap_or(e.price);
            let mut tiers = BTreeMap::new();
            if !e.is_free_ticket && current > 0.0 {
                tiers.insert("En uygun bilet".to_string(), tl_to_kurus(current));
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
    }
}
