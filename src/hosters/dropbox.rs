//! Dropbox shared links: `?dl=1` turns the preview page into the file (folders come as a ZIP).

use async_trait::async_trait;
use url::Url;

use super::{Hoster, HosterError, HosterLink, host_is};

pub struct Dropbox;

#[async_trait]
impl Hoster for Dropbox {
    fn name(&self) -> &'static str {
        "dropbox"
    }

    fn label(&self) -> &'static str {
        "Dropbox"
    }

    fn matches(&self, url: &Url) -> bool {
        host_is(url, &["dropbox.com"])
    }

    async fn resolve(&self, _http: &reqwest::Client, url: &Url) -> Result<HosterLink, HosterError> {
        Ok(HosterLink::direct(direct_url(url)))
    }
}

fn direct_url(url: &Url) -> String {
    let mut direct = url.clone();
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| key != "dl" && key != "raw")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    direct.query_pairs_mut().clear().extend_pairs(pairs).append_pair("dl", "1");
    direct.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forces_direct_download() {
        let url = Url::parse("https://www.dropbox.com/scl/fi/abc/file.zip?rlkey=xyz&dl=0").unwrap();
        assert_eq!(direct_url(&url), "https://www.dropbox.com/scl/fi/abc/file.zip?rlkey=xyz&dl=1");
    }
}
