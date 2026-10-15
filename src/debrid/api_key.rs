//! Credentials of accounts that authenticate with an API key, stored as `{"api_key": "..."}`.
//! Real-Debrid accounts connected with a code store OAuth tokens instead (see `realdebrid`).

use serde::Serialize;
use serde_json::Value;

use super::DebridError;

#[derive(Serialize)]
struct Credentials<'a> {
    api_key: &'a str,
}

pub fn encode(api_key: &str) -> String {
    serde_json::to_string(&Credentials { api_key: api_key.trim() }).expect("serializable")
}

pub fn decode(raw: &str) -> Result<String, DebridError> {
    let value: Value = serde_json::from_str(raw).map_err(|_| DebridError::Auth("invalid credentials".into()))?;
    value["api_key"].as_str().map(str::to_owned).ok_or_else(|| DebridError::Auth("missing API key".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_the_key() {
        let stored = encode(" key\"1 ");
        // The format of existing databases.
        assert_eq!(stored, r#"{"api_key":"key\"1"}"#);
        assert_eq!(decode(&stored).unwrap(), "key\"1");
        assert!(matches!(decode("not json"), Err(DebridError::Auth(m)) if m == "invalid credentials"));
        assert!(matches!(decode(r#"{"username":"me"}"#), Err(DebridError::Auth(m)) if m == "missing API key"));
    }
}
