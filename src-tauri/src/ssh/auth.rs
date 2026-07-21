use serde::{Deserialize, Serialize};

/// Authentication method for SSH connections.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AuthMethod {
    #[serde(rename = "password")]
    Password { password: String },

    #[serde(rename = "key")]
    PrivateKey {
        key_path: String,
        passphrase: Option<String>,
    },

    #[serde(rename = "agent")]
    Agent,
}
