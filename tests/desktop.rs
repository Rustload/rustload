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
