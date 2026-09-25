//! Telegram mesaj metinleri.

use crate::diff::Change;
use crate::matcher::same_event;
use crate::model::{esc, fmt_date, fmt_date_short, fmt_tl, Event, SourceId};
use crate::store::{Health, State};
use chrono::NaiveDate;
use std::collections::BTreeMap;

fn header(e: &Event) -> String {
    let mut s = format!("<b>{}</b>\n", esc(&e.title));
    let mut meta = vec![e.category.clone()];
    if let Some(d) = &e.date {
        meta.push(fmt_date(d));
    }
    if !e.venue.is_empty() {
        meta.push(e.venue.clone());
    }
    s += &format!("📍 {}\n", esc(&meta.join(" · ")));
    s
}

/// Aynı etkinliğin diğer sitelerdeki en düşük fiyatları.
fn other_sites(e: &Event, state: &State) -> String {
    let mut others: Vec<&Event> = state.events.values().map(|t| &t.event).filter(|o| same_event(e, o)).collect();
    others.sort_by_key(|o| o.min_price().unwrap_or(i64::MAX));
    let parts: Vec<String> = others
        .iter()
        .map(|o| match o.min_price() {
            _ if o.sold_out => format!("<a href=\"{}\">{}</a>: tükendi", esc(&o.url), o.source),
            Some(p) => format!("<a href=\"{}\">{}</a>: {}", esc(&o.url), o.source, fmt_tl(p)),
            None => format!("<a href=\"{}\">{}</a>", esc(&o.url), o.source),
        })
        .collect();
    let cheapest_here = match (e.min_price(), others.first().and_then(|o| o.min_price())) {
        (Some(mine), Some(theirs)) => mine < theirs,
        _ => false,
    };
    match (parts.is_empty(), cheapest_here) {
        (true, _) => String::new(),
        (false, true) => format!("🏷️ En ucuz burada. Diğer siteler: {}\n", parts.join(" | ")),
        (false, false) => format!("🏷️ Diğer siteler: {}\n", parts.join(" | ")),
    }
}

fn link(e: &Event) -> String {
    let suffix = match e.source {
        SourceId::Biletinial => "'da",
        SourceId::Bubilet | SourceId::Biletix => "'te",
    };
    format!("🔗 <a href=\"{}\">{}{suffix} aç</a>", esc(&e.url), e.source)
}

pub fn change(c: &Change, state: &State) -> String {
    match c {
        Change::PriceDrop(e, drops) => {
            let mut s = format!("📉 <b>Fiyat düştü</b> — {}", header(e));
            for (tier, old, new) in drops {
                let pct = (old - new) as f64 * 100.0 / *old as f64;
                s += &format!("💸 {}: <s>{}</s> → <b>{}</b> (−%{:.0})\n", esc(tier), fmt_tl(*old), fmt_tl(*new), pct);
            }
            s + &other_sites(e, state) + &link(e)
        }
        Change::Discount(e) => {
            let mut s = format!("🔥 <b>İndirimde</b> — {}", header(e));
            if let Some((old, new)) = e.discount() {
                let pct = (old - new) as f64 * 100.0 / old as f64;
                s += &format!("💸 <s>{}</s> → <b>{}</b> (−%{:.0})
", fmt_tl(old), fmt_tl(new), pct);
            }
            s + &other_sites(e, state) + &link(e)
        }
        Change::New(e) => {
            let mut s = format!("🆕 <b>Yeni etkinlik</b> — {}", header(e));
            match e.min_price() {
                _ if e.sold_out => s += "🚫 Tükendi\n",
                Some(p) => s += &format!("💰 {}'den başlayan fiyatlarla\n", fmt_tl(p)),
                None => {}
            }
            s + &other_sites(e, state) + &link(e)
        }
        Change::BackInStock(e) => {
            let mut s = format!("🎟️ <b>Tekrar satışta</b> — {}", header(e));
            if let Some(p) = e.min_price() {
                s += &format!("💰 {}'den başlayan fiyatlarla\n", fmt_tl(p));
            }
            s + &other_sites(e, state) + &link(e)
        }
    }
}

/// /liste: yaklaşan etkinlikler, siteler arası eşleştirilip en ucuzu gösterilerek.
pub fn list(state: &State, today: NaiveDate, filter: Option<&str>) -> String {
    let mut events: Vec<&Event> = state
        .events
        .values()
        .map(|t| &t.event)
        .filter(|e| e.date.map(|d| d.date_naive() >= today).unwrap_or(true))
        .filter(|e| filter.map(|f| crate::model::fold(&e.title).contains(&crate::model::fold(f))).unwrap_or(true))
        .collect();
    events.sort_by_key(|e| (e.date, e.title.clone()));

    // Aynı etkinliği bir kez listele
    let mut groups: Vec<Vec<&Event>> = Vec::new();
    for e in events {
        match groups.iter_mut().find(|g| g.iter().any(|o| same_event(o, e) || (o.source == e.source && o.title == e.title && o.date == e.date))) {
            Some(g) => g.push(e),
            None => groups.push(vec![e]),
        }
    }
    if groups.is_empty() {
        return "Kayıtlı yaklaşan etkinlik yok.".into();
    }

    let mut s = format!("🎫 <b>Yaklaşan etkinlikler</b> ({})\n\n", groups.len());
    for g in groups {
        let e = g[0];
        let best = g.iter().filter(|x| !x.sold_out).filter_map(|x| x.min_price().map(|p| (p, *x))).min_by_key(|(p, _)| *p);
        let date = e.date.as_ref().map(fmt_date_short).unwrap_or_default();
        let price = match best {
            Some((p, x)) => format!("<a href=\"{}\">{}</a> ({})", esc(&x.url), fmt_tl(p), x.source),
            None if g.iter().all(|x| x.sold_out) => "tükendi".into(),
            None => format!("<a href=\"{}\">fiyat yok</a>", esc(&e.url)),
        };
        s += &format!("• {date} — {} — {price}\n", esc(&e.title));
    }
    s
}

/// /indirim: şu an sitede indirimli görünen yaklaşan etkinlikler, en büyük indirim önce.
pub fn discounts(state: &State, today: NaiveDate) -> String {
    let mut events: Vec<(&Event, i64, i64)> = state
        .events
        .values()
        .map(|t| &t.event)
        .filter(|e| e.date.map(|d| d.date_naive() >= today).unwrap_or(true))
        .filter_map(|e| e.discount().map(|(old, new)| (e, old, new)))
        .collect();
    if events.is_empty() {
        return "Şu an indirimli etkinlik yok.".into();
    }
    events.sort_by_key(|(_, old, new)| std::cmp::Reverse((old - new) * 1000 / old));

    let mut s = format!("🔥 <b>İndirimdekiler</b> ({})

", events.len());
    for (e, old, new) in events {
        let date = e.date.as_ref().map(fmt_date_short).unwrap_or_default();
        let pct = (old - new) as f64 * 100.0 / old as f64;
        s += &format!(
            "• {date} — {} — <s>{}</s> <a href=\"{}\">{}</a> (−%{:.0}, {})
",
            esc(&e.title),
            fmt_tl(old),
            esc(&e.url),
            fmt_tl(new),
            pct,
            e.source
        );
    }
    s
}

pub fn status(state: &State, sources: &[SourceId], percent: f64, min_tl: f64) -> String {
    let mut s = String::from("📊 <b>Durum</b>\n\n");
    let mut per_source: BTreeMap<SourceId, usize> = BTreeMap::new();
    for t in state.events.values() {
        *per_source.entry(t.event.source).or_default() += 1;
    }
    for src in sources {
        let h = state.health.get(src).cloned().unwrap_or_default();
        let icon = if h.consecutive_failures == 0 { "✅" } else { "⚠️" };
        s += &format!("{icon} {src}: son taramada {} etkinlik, takipte {}", h.last_count, per_source.get(src).unwrap_or(&0));
        if h.consecutive_failures > 0 {
            s += &format!(" — {} kez üst üste hata", h.consecutive_failures);
        }
        s += "\n";
    }
    s += &format!("\n🔔 Bildirim eşiği: %{percent} ya da {} düşüş", fmt_tl((min_tl * 100.0) as i64));
    s
}

pub fn source_down(src: SourceId, h: &Health) -> String {
    format!(
        "⚠️ <b>{src}</b> {} çalıştırmadır veri döndürmüyor.\nSon hata: <code>{}</code>",
        h.consecutive_failures,
        esc(h.last_error.as_deref().unwrap_or("-"))
    )
}

pub fn source_up(src: SourceId) -> String {
    format!("✅ <b>{src}</b> tekrar çalışıyor.")
}

pub const HELP: &str = "🤖 <b>Bilet Takip</b>\n\n\
Kayseri'deki etkinliklerin bilet fiyatlarını Bubilet, Biletinial ve Biletix'ten takip ediyorum. \
Bir etkinlik indirime girince, fiyat düşünce ya da tükenen bilet tekrar satışa çıkınca haber veririm.\n\n\
/indirim — şu an indirimdeki etkinlikler\n\
/liste — yaklaşan etkinlikler ve en ucuz fiyatlar\n\
/ara kelime — etkinlik ara (ör. /ara karsu)\n\
/tara — beklemeden hemen tara\n\
/durum — kaynakların durumu\n\
/esik 10 — bildirim eşiğini %10 yap\n\n\
<i>Siteleri 20 dakikada bir tarıyorum.</i>";
