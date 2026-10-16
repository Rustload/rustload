//! Who will download an http(s) link, in the order `resolve.rs` tries: accounts supporting the host
//! (its own premium account first, then debrid services), then a built-in hoster plugin, then the
//! link itself. Shared by the availability
//! checker and the link grabber. Media sites never get here: yt-dlp takes them first
//! (`engine::media::routes`).

use super::Engine;
use crate::{debrid::AccountHandle, hosters::Hoster};

pub enum Handler<'a> {
    /// The best account supporting the host (the next ones are tried if it fails).
    Debrid(AccountHandle),
    /// A built-in hoster plugin.
    Hoster(&'a dyn Hoster),
    /// The link itself.
    Direct,
}

impl<'a> Handler<'a> {
    /// The handler of a new link. Files of a debrid transfer are pinned to their account instead.
    pub async fn select(engine: &'a Engine, url: &str) -> Self {
        if engine.settings.get().debrid.enabled
            && let Some(account) = engine.debrid.candidates(url).await.into_iter().next()
        {
            return Self::Debrid(account);
        }
        Self::builtin(engine, url)
    }

    /// The handler when no debrid account is involved.
    pub fn builtin(engine: &'a Engine, url: &str) -> Self {
        engine.hosters.find(url).map_or(Self::Direct, Self::Hoster)
    }
}
