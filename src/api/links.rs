//! Link grabber: checks links before they are added.

use axum::{Json, extract::State};
use serde::Deserialize;
use serde_json::{Value, json};

use super::AppState;
use crate::{
    engine::grabber::{self, Link},
    error::{ApiError, ApiResult},
};

/// Links checked per request.
const MAX_LINKS: usize = 50;

#[derive(Deserialize)]
pub struct CheckRequest {
    urls: Vec<String>,
    #[serde(default)]
    scan_pages: bool,
    /// `false`: only say who would download each link, without contacting its site.
    #[serde(default = "default_true")]
    probe: bool,
}

fn default_true() -> bool {
    true
}

pub async fn check(State(state): State<AppState>, Json(body): Json<CheckRequest>) -> ApiResult<Json<Value>> {
    if body.urls.is_empty() {
        return Err(ApiError::bad_request("NO_LINKS", "no link found"));
    }
    if body.urls.len() > MAX_LINKS {
        return Err(ApiError::bad_request(
            "TOO_MANY_LINKS",
            format!("at most {MAX_LINKS} links can be checked at once"),
        ));
    }
    let links = body
        .urls
        .iter()
        .map(|url| {
            Link::parse(url).ok_or_else(|| {
                ApiError::bad_request("INVALID_LINK", format!("not an http(s) or magnet link: {}", url.trim()))
            })
        })
        .collect::<ApiResult<Vec<_>>>()?;
    let results = if body.probe {
        grabber::check_links(&state.engine, links, body.scan_pages).await?
    } else {
        grabber::route_links(&state.engine, links).await?
    };
    Ok(Json(json!({ "results": results })))
}
