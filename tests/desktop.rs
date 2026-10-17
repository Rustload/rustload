//! The desktop app's server: local only, opened logged in with single-use addresses.

use rustload::{Config, Server};

#[tokio::test]
async fn opens_logged_in_once_per_address() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config::desktop(dir.path().join("config"), dir.path().join("downloads"), 0, "tester");
    let server = Server::start(config).await.unwrap();
    let handle = server.handle();
    assert!(handle.address().ip().is_loopback());

    let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let url = handle.login_url("/?add=magnet%3A%3Fxt%3Durn%3Abtih%3Aabc").unwrap();
    let response = http.get(&url).send().await.unwrap();
    assert_eq!(response.status(), 303);
    assert_eq!(response.headers()["location"], "/?add=magnet%3A%3Fxt%3Durn%3Abtih%3Aabc");
    let cookie = response.headers()["set-cookie"].to_str().unwrap().split(';').next().unwrap().to_owned();

    let status: serde_json::Value = http
        .get(format!("http://{}/api/auth/status", handle.address()))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["user"]["username"], "tester");
    assert_eq!(status["desktop"], true);

    // Single use, and only paths of this site.
    assert_eq!(http.get(&url).send().await.unwrap().status(), 401);
    let elsewhere = handle.login_url("//example.com/").unwrap();
    assert_eq!(http.get(&elsewhere).send().await.unwrap().headers()["location"], "/");

    server.stop().await.unwrap();
}

#[tokio::test]
async fn opening_the_window_again_replaces_its_session() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config::desktop(dir.path().join("config"), dir.path().join("downloads"), 0, "tester");
    let server = Server::start(config).await.unwrap();
    let handle = server.handle();
    let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let session = |response: &reqwest::Response| {
        response.headers()["set-cookie"].to_str().unwrap().split(';').next().unwrap().to_owned()
    };
    let user = |cookie: String| {
        let http = http.clone();
        let url = format!("http://{}/api/auth/status", handle.address());
        async move {
            let status: serde_json::Value =
                http.get(url).header("cookie", cookie).send().await.unwrap().json().await.unwrap();
            status["user"]["username"].as_str().map(str::to_owned)
        }
    };

    let first = session(&http.get(handle.login_url("/").unwrap()).send().await.unwrap());
    let reopened = http.get(handle.login_url("/").unwrap()).header("cookie", &first).send().await.unwrap();
    assert_eq!(reopened.status(), 303);
    let second = session(&reopened);
    assert_ne!(first, second);
    assert_eq!(user(second).await.as_deref(), Some("tester"));
    assert_eq!(user(first).await, None);

    server.stop().await.unwrap();
}

#[tokio::test]
async fn servers_issue_no_login_addresses() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::desktop(dir.path().join("config"), dir.path().join("downloads"), 0, "tester");
    config.desktop = false;
    let server = Server::start(config).await.unwrap();
    let handle = server.handle();
    assert!(handle.login_url("/").is_err());
    let response = reqwest::get(format!("http://{}/api/auth/handoff?code=x", handle.address())).await.unwrap();
    assert_eq!(response.status(), 404);
    server.stop().await.unwrap();
}

const EXTENSION: &str = "chrome-extension://abcdefghijklmnop";

async fn pair(handle: &rustload::Handle, origin: Option<&str>) -> reqwest::Response {
    let mut request = reqwest::Client::new()
        .post(format!("http://{}/api/desktop/pair", handle.address()))
        .json(&serde_json::json!({ "browser": "Chrome" }));
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    request.send().await.unwrap()
}

#[tokio::test]
async fn pairs_the_extension_once_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config::desktop(dir.path().join("config"), dir.path().join("downloads"), 0, "tester");
    let server = Server::start(config).await.unwrap();
    let handle = server.handle();
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    {
        let asked = asked.clone();
        handle.set_pairing_approver(move |request| {
            asked.lock().unwrap().push((request.origin, request.browser));
            async { true }
        });
    }

    let response = pair(&handle, Some(EXTENSION)).await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["access-control-allow-origin"], EXTENSION);
    let paired: serde_json::Value = response.json().await.unwrap();
    assert_eq!(paired["url"], format!("http://127.0.0.1:{}/", handle.address().port()));
    assert_eq!(*asked.lock().unwrap(), [(EXTENSION.to_owned(), "Chrome".to_owned())]);

    // The key works like one copied from the settings.
    let system = reqwest::Client::new()
        .get(format!("http://{}/api/system", handle.address()))
        .header("x-api-key", paired["api_key"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(system.status(), 200);

    // The browser's preflight for the JSON body.
    let preflight = reqwest::Client::new()
        .request(reqwest::Method::OPTIONS, format!("http://{}/api/desktop/pair", handle.address()))
        .header("origin", EXTENSION)
        .header("access-control-request-method", "POST")
        .send()
        .await
        .unwrap();
    assert_eq!(preflight.status(), 204);
    assert_eq!(preflight.headers()["access-control-allow-origin"], EXTENSION);

    server.stop().await.unwrap();
}

#[tokio::test]
async fn refuses_pairing_when_declined_or_not_an_extension() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config::desktop(dir.path().join("config"), dir.path().join("downloads"), 0, "tester");
    let server = Server::start(config).await.unwrap();
    let handle = server.handle();

    // The app has not said how to ask yet.
    assert_eq!(pair(&handle, Some(EXTENSION)).await.status(), 503);

    handle.set_pairing_approver(|_| async { false });
    let refused = pair(&handle, Some(EXTENSION)).await;
    assert_eq!(refused.status(), 403);
    assert_eq!(refused.headers()["access-control-allow-origin"], EXTENSION);
    assert_eq!(refused.json::<serde_json::Value>().await.unwrap()["code"], "PAIRING_REFUSED");

    for origin in [None, Some("https://example.com"), Some("http://127.0.0.1:8800")] {
        let response = pair(&handle, origin).await;
        assert_eq!(response.status(), 403, "{origin:?}");
        assert!(response.headers().get("access-control-allow-origin").is_none());
        assert_eq!(response.json::<serde_json::Value>().await.unwrap()["code"], "NOT_AN_EXTENSION");
    }
    server.stop().await.unwrap();
}

#[tokio::test]
async fn servers_do_not_pair() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::desktop(dir.path().join("config"), dir.path().join("downloads"), 0, "tester");
    config.desktop = false;
    let server = Server::start(config).await.unwrap();
    let handle = server.handle();
    handle.set_pairing_approver(|_| async { true });
    assert_eq!(pair(&handle, Some(EXTENSION)).await.status(), 404);
    server.stop().await.unwrap();
}
