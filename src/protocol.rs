use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub enum Message {
    ClientHello {
        domain: String,
        path: Option<String>,
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

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Message::ClientHello { domain, path } => {
                write!(
                    f,
                    "Message::ClientHello domain={:?}, path={:?}",
                    domain, path
                )
            }
            Message::ServerHello { domain, path } => {
                write!(f, "Message::ServerHello domain={}, path={}", domain, path)
            }
            Message::NewConnection { id } => {
                write!(f, "Message::NewConnection id={}", id)
            }
            Message::Data { id, data } => {
                write!(f, "Message::Data id={}, bytes={}", id, data.len())
            }
            Message::CloseConnection { id } => {
                write!(f, "Message::CloseConnection id={}", id)
            }
            Message::Shutdown { message } => {
                write!(f, "Message::Shutdown message={:?}", message)
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ExternalMessage {
    NewConnection {
        conn_id: String,
        domain: String,
        path: String,
        port: u16,
    },
    ShutdownConnection {
        conn_id: String,
        domain: String,
        path: String,
        port: u16,
    },
}
