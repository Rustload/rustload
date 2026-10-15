//! Username/password accounts of hosters. The provider's session token is kept with the stored
//! credentials, so that restarts do not log in again, and renewed when the provider reports it
//! expired.
//!
//! Nothing here implements `Debug`: credentials must never end up in logs.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::DebridError;

#[derive(Clone, Serialize, Deserialize)]
struct Credentials {
    username: String,
    password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token: Option<String>,
}

impl Credentials {
    fn parse(raw: &str) -> Result<Self, DebridError> {
        serde_json::from_str(raw).map_err(|_| DebridError::Auth("invalid credentials".into()))
    }

    fn to_json(&self) -> String {
        serde_json::to_string(self).expect("serializable")
    }
}

pub fn credentials(username: &str, password: &str) -> String {
    Credentials { username: username.trim().to_owned(), password: password.to_owned(), token: None }.to_json()
}

/// New credentials with another username and/or password; the session token is dropped.
pub fn update(raw: &str, username: Option<&str>, password: Option<&str>) -> Result<String, DebridError> {
    let current = Credentials::parse(raw)?;
    Ok(credentials(username.unwrap_or(&current.username), password.unwrap_or(&current.password)))
}

pub fn username(raw: &str) -> Option<String> {
    Credentials::parse(raw).ok().map(|c| c.username).filter(|u| !u.is_empty())
}

/// Error of an authenticated call: `Expired` asks the caller to log in again and retry.
pub enum CallError {
    Expired,
    Failed(DebridError),
}

impl From<DebridError> for CallError {
    fn from(error: DebridError) -> Self {
        Self::Failed(error)
    }
}

impl From<reqwest::Error> for CallError {
    fn from(error: reqwest::Error) -> Self {
        Self::Failed(error.into())
    }
}

impl From<CallError> for DebridError {
    fn from(error: CallError) -> Self {
        match error {
            CallError::Expired => DebridError::Auth("the session was refused right after logging in".into()),
            CallError::Failed(error) => error,
        }
    }
}

pub struct Session {
    credentials: Mutex<Credentials>,
    /// Credentials to persist after a login, see `DebridService::take_updated_credentials`.
    updated: Mutex<Option<String>>,
}

impl Session {
    pub fn from_credentials(raw: &str) -> Result<Self, DebridError> {
        let credentials = Credentials::parse(raw)?;
        if credentials.username.is_empty() || credentials.password.is_empty() {
            return Err(DebridError::Auth("missing username or password".into()));
        }
        Ok(Self { credentials: Mutex::new(credentials), updated: Mutex::new(None) })
    }

    /// Username and password, to log in.
    pub fn login(&self) -> (String, String) {
        let credentials = self.credentials.lock().expect("poisoned");
        (credentials.username.clone(), credentials.password.clone())
    }

    pub fn username(&self) -> String {
        self.credentials.lock().expect("poisoned").username.clone()
    }

    fn token(&self) -> Option<String> {
        self.credentials.lock().expect("poisoned").token.clone()
    }

    /// Stores a new session token, or forgets an expired one.
    fn set_token(&self, token: Option<String>) {
        let mut credentials = self.credentials.lock().expect("poisoned");
        credentials.token = token;
        *self.updated.lock().expect("poisoned") = Some(credentials.to_json());
    }

    /// Sends an authenticated request with the session token, logging in first when there is
    /// none, and once more if the provider reports the session expired. `login` returns a new
    /// token; `request` sends the request with the token it is given.
    pub async fn call<T, L, R>(&self, login: impl Fn() -> L, request: impl Fn(String) -> R) -> Result<T, DebridError>
    where
        L: Future<Output = Result<String, DebridError>>,
        R: Future<Output = Result<T, CallError>>,
    {
        let token = self.token_or_login(&login).await?;
        match request(token).await {
            Err(CallError::Expired) => {
                self.set_token(None);
                let token = self.token_or_login(&login).await?;
                Ok(request(token).await?)
            }
            result => Ok(result?),
        }
    }

    async fn token_or_login<L>(&self, login: impl Fn() -> L) -> Result<String, DebridError>
    where
        L: Future<Output = Result<String, DebridError>>,
    {
        if let Some(token) = self.token() {
            return Ok(token);
        }
        let token = login().await?;
        self.set_token(Some(token.clone()));
        Ok(token)
    }

    pub fn take_updated(&self) -> Option<String> {
        self.updated.lock().expect("poisoned").take()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    #[test]
    fn keeps_the_token_with_the_credentials() {
        let session = Session::from_credentials(&credentials("me@example.com", "pass word ")).unwrap();
        assert_eq!(session.login(), ("me@example.com".to_owned(), "pass word ".to_owned()));
        assert_eq!(session.token(), None);
        assert!(session.take_updated().is_none());

        session.set_token(Some("tok".into()));
        let stored = session.take_updated().unwrap();
        assert!(session.take_updated().is_none());
        let restored = Session::from_credentials(&stored).unwrap();
        assert_eq!(restored.token().as_deref(), Some("tok"));
        assert_eq!(restored.username(), "me@example.com");
    }

    #[test]
    fn updates_drop_the_token() {
        let session = Session::from_credentials(&credentials("old", "secret")).unwrap();
        session.set_token(Some("tok".into()));
        let stored = session.take_updated().unwrap();

        let renamed = update(&stored, Some("new"), None).unwrap();
        let session = Session::from_credentials(&renamed).unwrap();
        assert_eq!(session.login(), ("new".to_owned(), "secret".to_owned()));
        assert_eq!(session.token(), None);
        assert_eq!(username(&renamed).as_deref(), Some("new"));
    }

    #[tokio::test]
    async fn logs_in_again_once_when_the_session_expired() {
        let session = Session::from_credentials(&credentials("me", "secret")).unwrap();
        session.set_token(Some("old".into()));
        let logins = AtomicU32::new(0);
        let login = || async {
            logins.fetch_add(1, Ordering::Relaxed);
            Ok("new".to_owned())
        };

        let expiring = |token: String| async move { if token == "old" { Err(CallError::Expired) } else { Ok(token) } };
        assert_eq!(session.call(login, expiring).await.ok().as_deref(), Some("new"));
        assert_eq!(logins.load(Ordering::Relaxed), 1);
        assert_eq!(
            Session::from_credentials(&session.take_updated().unwrap()).unwrap().token().as_deref(),
            Some("new")
        );

        // A session refused right after logging in is an authentication error, not a loop.
        let refused = session.call(login, |_| async { Err::<(), _>(CallError::Expired) }).await;
        assert!(matches!(refused, Err(DebridError::Auth(_))));
        assert_eq!(logins.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn rejects_incomplete_credentials() {
        assert!(Session::from_credentials(&credentials("me", "")).is_err());
        assert!(Session::from_credentials("{\"api_key\":\"x\"}").is_err());
        assert_eq!(username("not json"), None);
    }
}
