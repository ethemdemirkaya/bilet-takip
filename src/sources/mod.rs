pub mod biletinial;
pub mod biletix;
pub mod bubilet;

use crate::config::Config;
use crate::http::Http;
use crate::model::{Event, SourceId};
use anyhow::Result;

/// Config'de açık olan kaynaklar.
pub fn enabled(cfg: &Config) -> Vec<SourceId> {
    let mut v = Vec::new();
    if cfg.sources.bubilet {
        v.push(SourceId::Bubilet);
    }
    if cfg.sources.biletinial {
        v.push(SourceId::Biletinial);
    }
    if cfg.sources.biletix {
        v.push(SourceId::Biletix);
    }
    v
}

pub fn fetch(source: SourceId, http: &Http, cfg: &Config) -> Result<Vec<Event>> {
    match source {
        SourceId::Bubilet => bubilet::fetch(http, cfg),
        SourceId::Biletinial => biletinial::fetch(http, cfg),
        SourceId::Biletix => biletix::fetch(http, cfg),
    }
}
