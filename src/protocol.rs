use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub enum Message {
    ClientHello {
        domain: Option<String>,
        path: Option<String>,
        auth_token: String,
    },
    ServerHello {
        domain: String,
        path: String,
    },
    NewConnection {
        id: u32,
    },
    Data {
        id: u32,
        data: Vec<u8>,
    },
    CloseConnection {
        id: u32,
    },
    Shutdown {
        message: Option<String>,
    },
}
