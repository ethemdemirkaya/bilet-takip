//! Aynı etkinliği farklı sitelerde eşleştirme: aynı gün + benzer isim.

use crate::model::{fold, tr_offset, Event};

const STOPWORDS: [&str; 8] = ["konseri", "konser", "concert", "tiyatro", "oyunu", "live", "in", "ve"];

fn normalize(title: &str) -> String {
    let f: String = fold(title).chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    f.split_whitespace().filter(|w| !STOPWORDS.contains(w)).collect::<Vec<_>>().join(" ")
}

pub fn same_event(a: &Event, b: &Event) -> bool {
    if a.source == b.source {
        return false;
    }
    let (Some(da), Some(db)) = (a.date, b.date) else { return false };
    if da.with_timezone(&tr_offset()).date_naive() != db.with_timezone(&tr_offset()).date_naive() {
        return false;
    }
    let (na, nb) = (normalize(&a.title), normalize(&b.title));
    if na.is_empty() || nb.is_empty() {
        return false;
    }
    na == nb
        || (na.len() >= 4 && nb.len() >= 4 && (na.contains(&nb) || nb.contains(&na)))
        || strsim::jaro_winkler(&na, &nb) >= 0.9
        || word_overlap(&na, &nb) >= 0.8
}

/// Kelime kümelerinin Jaccard benzerliği (kelime sırası farklı başlıklar için).
fn word_overlap(a: &str, b: &str) -> f64 {
    let wa: std::collections::BTreeSet<&str> = a.split(' ').collect();
    let wb: std::collections::BTreeSet<&str> = b.split(' ').collect();
    let inter = wa.intersection(&wb).count() as f64;
    let union = wa.union(&wb).count() as f64;
    if union == 0.0 { 0.0 } else { inter / union }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SourceId;
    use chrono::DateTime;

    fn ev(source: SourceId, title: &str, date: &str) -> Event {
        Event {
            source,
            id: title.into(),
            title: title.into(),
            venue: String::new(),
            date: DateTime::parse_from_rfc3339(date).ok(),
            category: String::new(),
            url: String::new(),
            tiers: Default::default(),
            sold_out: false,
            list_prices: Default::default(),
        }
    }

    #[test]
    fn matches_across_sites() {
        let a = ev(SourceId::Bubilet, "Poizi Konseri", "2026-10-09T18:00:00+00:00");
        let b = ev(SourceId::Biletix, "Poizi", "2026-10-09T18:00:00Z");
        assert!(same_event(&a, &b));
        let c = ev(SourceId::Biletinial, "Salih Bademci - Sesler", "2026-10-24T20:30:00+03:00");
        let d = ev(SourceId::Bubilet, "Sesler - Salih Bademci", "2026-10-24T17:30:00+00:00");
        assert!(same_event(&c, &d));
        assert!(!same_event(&c, &ev(SourceId::Bubilet, "Poizi", "2026-10-24T17:30:00+00:00")));
    }

    #[test]
    fn different_day_no_match() {
        let a = ev(SourceId::Bubilet, "Grup Abdal", "2026-10-09T16:00:00Z");
        let b = ev(SourceId::Biletix, "Grup Abdal", "2026-10-16T17:30:00Z");
        assert!(!same_event(&a, &b));
    }
}
