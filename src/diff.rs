use crate::config::Thresholds;
use crate::model::{Event, SourceId};
use crate::store::{State, Tracked};
use chrono::NaiveDate;

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    New(Event),
    /// (kategori, eski fiyat, yeni fiyat)
    PriceDrop(Event, Vec<(String, i64, i64)>),
    /// Site etkinliğe indirim koydu (üstü çizili fiyat göründü).
    Discount(Event),
    BackInStock(Event),
}

/// Bir kaynağın güncel etkinliklerini durumla karşılaştırır, durumu günceller ve değişiklikleri döner.
///
/// Fiyat düşüşü, her kategori için tutulan referans fiyata göre ölçülür: fiyat artarsa referans
/// yükselir, anlamlı bir düşüş bildirilince referans yeni fiyat olur. Böylece aynı fiyat için tekrar
/// bildirim gitmez, küçük küçük düşüşler birikince yine yakalanır.
pub fn apply(state: &mut State, source: SourceId, current: Vec<Event>, th: &Thresholds, today: NaiveDate) -> Vec<Change> {
    let first_scan = !state.initialized_sources.contains(&source);
    let mut changes = Vec::new();

    for ev in current {
        let key = ev.key();
        let Some(t) = state.events.get_mut(&key) else {
            if !first_scan {
                changes.push(if ev.discount().is_some() { Change::Discount(ev.clone()) } else { Change::New(ev.clone()) });
            }
            state.events.insert(
                key,
                Tracked { reference: ev.tiers.clone(), event: ev, first_seen: today, last_seen: today },
            );
            continue;
        };

        let mut drops = Vec::new();
        for (tier, &price) in &ev.tiers {
            match t.reference.get(tier).copied() {
                Some(reference) if th.is_significant(reference, price) => {
                    drops.push((tier.clone(), reference, price));
                    t.reference.insert(tier.clone(), price);
                }
                Some(reference) if price > reference => {
                    t.reference.insert(tier.clone(), price);
                }
                Some(_) => {}
                None => {
                    t.reference.insert(tier.clone(), price);
                }
            }
        }

        if ev.discount().is_some() && t.event.discount().is_none() {
            changes.push(Change::Discount(ev.clone()));
        } else if t.event.sold_out && !ev.sold_out {
            changes.push(Change::BackInStock(ev.clone()));
        } else if !drops.is_empty() {
            changes.push(Change::PriceDrop(ev.clone(), drops));
        }

        t.event = ev;
        t.last_seen = today;
    }

    state.initialized_sources.insert(source);
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn th() -> Thresholds {
        Thresholds { min_drop_percent: 5.0, min_drop_tl: 50.0 }
    }

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 25).unwrap()
    }

    fn ev(id: &str, prices: &[(&str, i64)], sold_out: bool) -> Event {
        Event {
            source: SourceId::Bubilet,
            id: id.into(),
            title: format!("Etkinlik {id}"),
            venue: "EKM".into(),
            date: None,
            category: "Konser".into(),
            url: format!("https://x/{id}"),
            tiers: prices.iter().map(|(n, p)| (n.to_string(), *p)).collect::<BTreeMap<_, _>>(),
            sold_out,
            list_price: None,
        }
    }

    fn discounted(id: &str, list: i64, now: i64) -> Event {
        Event { list_price: Some(list), ..ev(id, &[("En uygun bilet", now)], false) }
    }

    fn run(state: &mut State, events: Vec<Event>) -> Vec<Change> {
        apply(state, SourceId::Bubilet, events, &th(), day())
    }

    #[test]
    fn first_scan_is_silent_then_new_events_notify() {
        let mut s = State::default();
        assert!(run(&mut s, vec![ev("a", &[("Tam", 100_000)], false)]).is_empty());
        let c = run(&mut s, vec![ev("a", &[("Tam", 100_000)], false), ev("b", &[("Tam", 50_000)], false)]);
        assert_eq!(c.len(), 1);
        assert!(matches!(&c[0], Change::New(e) if e.id == "b"));
    }

    #[test]
    fn price_drop_notifies_once() {
        let mut s = State::default();
        run(&mut s, vec![ev("a", &[("Tam", 75_000)], false)]);
        let c = run(&mut s, vec![ev("a", &[("Tam", 37_500)], false)]);
        assert_eq!(c, vec![Change::PriceDrop(ev("a", &[("Tam", 37_500)], false), vec![("Tam".into(), 75_000, 37_500)])]);
        assert!(run(&mut s, vec![ev("a", &[("Tam", 37_500)], false)]).is_empty(), "aynı fiyat tekrar bildirilmemeli");
    }

    #[test]
    fn small_changes_ignored_but_accumulate() {
        let mut s = State::default();
        run(&mut s, vec![ev("a", &[("Tam", 100_000)], false)]);
        assert!(run(&mut s, vec![ev("a", &[("Tam", 97_000)], false)]).is_empty()); // %3, 30 TL
        let c = run(&mut s, vec![ev("a", &[("Tam", 94_000)], false)]); // toplam %6
        assert!(matches!(&c[0], Change::PriceDrop(_, d) if d[0] == ("Tam".into(), 100_000, 94_000)));
    }

    #[test]
    fn rise_then_drop_notifies_again() {
        let mut s = State::default();
        run(&mut s, vec![ev("a", &[("Tam", 50_000)], false)]);
        assert!(run(&mut s, vec![ev("a", &[("Tam", 80_000)], false)]).is_empty());
        let c = run(&mut s, vec![ev("a", &[("Tam", 50_000)], false)]);
        assert!(matches!(&c[0], Change::PriceDrop(_, d) if d[0] == ("Tam".into(), 80_000, 50_000)));
    }

    #[test]
    fn discount_notifies_once_instead_of_price_drop() {
        let mut s = State::default();
        run(&mut s, vec![ev("a", &[("En uygun bilet", 100_000)], false)]);
        let c = run(&mut s, vec![discounted("a", 100_000, 70_000)]);
        assert_eq!(c, vec![Change::Discount(discounted("a", 100_000, 70_000))]);
        assert!(run(&mut s, vec![discounted("a", 100_000, 70_000)]).is_empty(), "aynı indirim tekrar bildirilmemeli");
    }

    #[test]
    fn new_event_already_discounted() {
        let mut s = State::default();
        run(&mut s, vec![ev("a", &[("Tam", 100_000)], false)]);
        let c = run(&mut s, vec![ev("a", &[("Tam", 100_000)], false), discounted("b", 90_000, 45_000)]);
        assert!(matches!(&c[..], [Change::Discount(e)] if e.id == "b"));
    }

    #[test]
    fn back_in_stock() {
        let mut s = State::default();
        run(&mut s, vec![ev("a", &[], true)]);
        let c = run(&mut s, vec![ev("a", &[("Tam", 50_000)], false)]);
        assert!(matches!(&c[0], Change::BackInStock(_)));
    }
}
