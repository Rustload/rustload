//! The web interface, compiled into the binary from `web/dist`.

use axum::{
    body::Body,
    http::{HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "web/dist/"]
#[allow_missing = true]
struct Assets;

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") || path == "api" {
        return (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({ "error": "not found", "code": "NOT_FOUND" })))
            .into_response();
    }
    match Assets::get(path).filter(|_| !path.is_empty()) {
        Some(file) => {
            // Vite fingerprints everything under `assets/`: cache it forever.
            let cache = if path.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
            respond(path, file.data.into_owned(), cache)
        }
        None => match Assets::get("index.html") {
            Some(index) => respond("index.html", index.data.into_owned(), "no-cache"),
            None => {
                (StatusCode::NOT_FOUND, "The web interface was not built (run `pnpm build` in web/).").into_response()
            }
        },
    }
}

fn respond(path: &str, data: Vec<u8>, cache: &'static str) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let mut response = Body::from(data).into_response();
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(mime.as_ref()) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}
