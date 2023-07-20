use clap::Parser;
use futures::{FutureExt, SinkExt, StreamExt, TryStreamExt};
use revconn::{protocol::Message, util::handle_connection};
use std::{collections::HashMap, sync::Mutex};
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    sync::mpsc::Sender,
};
use tracing::{debug, info};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
// use tokio_serde_bincode::ReadBincode;
// use tokio_serde::{Deserializer, Framed};

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Name of the person to greet
    #[arg(short, long)]
    name: String,

    /// Number of times to greet
    #[arg(short, long, default_value_t = 1)]
    count: u8,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::Layer::new()
                .with_ansi(true)
                .with_file(true)
                .with_line_number(true)
                .with_level(true), //.json(),
        )
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()?;

    let listen_addr = "0.0.0.0:8000".to_string();

    let listener = TcpListener::bind(listen_addr).await?;

    while let Ok((inbound, _)) = listener.accept().await {
        let transfer = transfer(inbound).map(|r| {
            if let Err(e) = r {
                println!("Failed to transfer; error={}", e);
            }
        });

        tokio::spawn(transfer);
    }

    Ok(())
}

async fn transfer(mut inbound: TcpStream) -> anyhow::Result<()> {
    let mut conn_id = 0;
    // let (c2s_tx, mut c2s_rx) = tokio::sync::mpsc::channel::<Message>(32);
    let (s2c_tx, mut s2c_rx) = tokio::sync::mpsc::channel::<Message>(32);
    let conn_map: Mutex<HashMap<u32, Sender<Message>>> = Mutex::new(HashMap::new());

    let (ri, wi) = inbound.split();

    let mut ri = {
        // Delimit frames using a length header
        let length_delimited =
            tokio_util::codec::FramedRead::new(ri, tokio_util::codec::LengthDelimitedCodec::new());

        // Deserialize frames
        let deserialized = tokio_serde::SymmetricallyFramed::new(
            length_delimited,
            tokio_serde::formats::SymmetricalBincode::<Message>::default(),
        );
        deserialized
    };

    let mut wi = {
        // Delimit frames using a length header
        let length_delimited =
            tokio_util::codec::FramedWrite::new(wi, tokio_util::codec::LengthDelimitedCodec::new());

        // Serialize frames with JSON
        let serialized = tokio_serde::SymmetricallyFramed::new(
            length_delimited,
            tokio_serde::formats::SymmetricalBincode::<Message>::default(),
        );
        serialized
    };
    match ri.try_next().await?.ok_or(anyhow::anyhow!("not message"))? {
        Message::ClientHello {
            domain,
            path,
            auth_token,
        } => {
            wi.send(Message::ServerHello {
                domain: "hello".to_string(),
                path: "".to_string(),
            })
            .await?;
        }
        _ => Err(anyhow::anyhow!("invalid Message"))?,
    }

    let listener = TcpListener::bind("0.0.0.0:8002").await?;
    debug!("handshake complete waiting port=8002");

    loop {
        tokio::select! {
            conn = listener.accept() => {
                let (conn, sock) = conn?;
                conn_id += 1;
                debug!("new connection id={}, sock={:?}", conn_id, sock);
                let (e2s_tx, e2s_rx) = tokio::sync::mpsc::channel::<Message>(32);
                {
                    let mut m = conn_map.lock().unwrap();
                    m.insert(conn_id, e2s_tx);
                }
                wi.send(Message::NewConnection { id: conn_id }).await?;
                let s2c_tx = s2c_tx.clone();
                tokio::spawn(
                    handle_connection(conn_id, e2s_rx, s2c_tx, conn)
                );
            }
            // message = c2s_rx.recv() => {
            //     println!("rx1 completed first with {:?}", message);
            // }
            message = s2c_rx.recv() => {
                debug!("message from client: {:?}", message);
                let message = message.ok_or(anyhow::anyhow!("no message found"))?;
                wi.send(message).await?;
            }
            message = ri.try_next() => {
                debug!("message from external port: {:?}", message);
                let message = message?.ok_or(anyhow::anyhow!("no message found"))?;
                match message {
                    Message::Data{id, data} => {
                        let tx = match conn_map.lock().unwrap().get(&id) {
                            Some(tx) => tx.clone(),
                            None => Err(anyhow::anyhow!("key not found in conn map"))?,
                        };
                        tx.send(Message::Data {id, data}).await?;
                    },
                    Message::Shutdown {message} => {
                        break;
                    },
                    _ => break,
                }
            }
        }
    }

    // Spawn a task that prints all received messages to STDOUT
    // while let Some(msg) = deserialized.try_next().await.unwrap() {
    //     println!("GOT: {:?}", msg);
    // }

    // Send the value
    // serialized
    //     .send(Message::NewConnection {
    //         id: "hello".to_string(),
    //     })
    //     .await
    //     .unwrap();

    // let bytes = ri.read_u16().await?;

    // let mut v = vec![0u8; 4096];
    // ri.read_exact(&mut v[0..bytes as usize]).await?;

    // let ri = MessageStream::new(ri);
    // let o = ri.await?;

    Ok(())
}