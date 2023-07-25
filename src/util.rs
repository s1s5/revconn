use base64::{engine::general_purpose, Engine as _};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tracing::debug;

use crate::protocol::Message;

async fn handle_connection_inner(
    conn_id: u32,
    mut rx: tokio::sync::mpsc::Receiver<Message>,
    s2c_tx: tokio::sync::mpsc::Sender<Message>,
    mut conn: TcpStream,
) -> anyhow::Result<()> {
    let (mut ri, mut wi) = conn.split();
    let mut buf = vec![0u8; 8192];

    // let mut frame_reader =
    //     tokio_util::codec::FramedRead::new(ri, tokio_util::codec::LengthDelimitedCodec::new());

    loop {
        tokio::select! {
            message = rx.recv() => {
                let message = message.ok_or(anyhow::anyhow!("no message"))?;
                debug!("message from receiver {:?}", message);
                match message {
                    Message::Data{id: _, data} => {
                        wi.write_all(&data).await?;
                    }
                    Message::CloseConnection{id: _} => {
                        break;
                    }
                    Message::Shutdown{message: _}=> {
                        break;
                    }
                    _ => {
                        Err(anyhow::anyhow!("Unexpected message"))?;
                    }
                }

            }
            num_bytes = ri.read(&mut buf) => {
                debug!("message from connection id={}, {:?}[bytes]", conn_id, num_bytes);
                let num_bytes = num_bytes?;
                if num_bytes == 0 {
                    break;
                }
                s2c_tx.send(Message::Data { id: conn_id, data: buf[0..num_bytes].iter().cloned().collect()}).await?;
            }
        }
    }
    Ok(())
}

pub async fn handle_connection(
    conn_id: u32,
    rx: tokio::sync::mpsc::Receiver<Message>,
    s2c_tx: tokio::sync::mpsc::Sender<Message>,
    conn: TcpStream,
) -> anyhow::Result<()> {
    let tx = s2c_tx.clone();
    let r = handle_connection_inner(conn_id, rx, s2c_tx, conn).await;
    tx.send(Message::CloseConnection { id: conn_id }).await?;
    r
}

pub fn get_key_and_nonce_from_env(key: &mut [u8; 32], nonce: &mut [u8; 12]) {
    // rPc0ATOUqsySPNoWtwa10+fSzLSNOq1vRJqz9qK0Aag=
    // uUXAxcAFLg+tLQsG
    let k = general_purpose::STANDARD
        .decode(std::env::var("ENCRYPT_KEY").unwrap())
        .unwrap();
    let n = general_purpose::STANDARD
        .decode(std::env::var("ENCRYPT_NONCE").unwrap())
        .unwrap();

    if k.len() < 32 {
        panic!("encrypt_key length must be greater than 32");
    }
    if n.len() < 12 {
        panic!("encrypt_key length must be greater than 32");
    }

    (0..32).into_iter().for_each(|i| key[i] = k[i]);
    (0..12).into_iter().for_each(|i| nonce[i] = n[i]);
}
