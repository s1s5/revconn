use clap::Parser;
use futures::{FutureExt, SinkExt, TryStreamExt};
use revconn::{
    encstream::{DecStream, EncStream},
    protocol::{ExternalMessage, Message},
    util::{get_key_and_nonce_from_env, handle_connection},
};
use std::{collections::HashMap, sync::Mutex};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc::Sender,
};
use tracing::{debug, error, info};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long)]
    bind: Option<String>,

    #[arg(long)]
    callback: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
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

    let mut key = [0x42; 32];
    let mut nonce = [0x24; 12];
    get_key_and_nonce_from_env(&mut key, &mut nonce);

    let listen_addr = args.bind.unwrap_or("0.0.0.0:8000".to_string());
    let listener = TcpListener::bind(listen_addr).await?;

    while let Ok((inbound, _)) = listener.accept().await {
        let transfer =
            transfer(inbound, key.clone(), nonce.clone(), args.callback.clone()).map(|r| {
                if let Err(e) = r {
                    println!("Failed to transfer; error={}", e);
                }
            });

        tokio::spawn(transfer);
    }

    Ok(())
}

struct OnShutdown {
    callback: String,
    conn_id: String,
    domain: String,
    path: String,
    port: u16,
}

impl Drop for OnShutdown {
    fn drop(&mut self) {
        let callback = self.callback.clone();
        let conn_id = self.conn_id.clone();
        let domain = self.domain.clone();
        let path = self.path.clone();
        let port = self.port;
        tokio::spawn(async move {
            let client = reqwest::Client::new();
            match client
                .post(callback)
                .body(
                    serde_json::to_string(&ExternalMessage::ShutdownConnection {
                        conn_id,
                        domain,
                        path,
                        port,
                    })
                    .unwrap(),
                )
                .send()
                .await
            {
                Ok(_) => {}
                Err(err) => error!("Error sending message to callback {:?}", err),
            };
        });
    }
}

async fn transfer(
    mut inbound: TcpStream,
    key: [u8; 32],
    nonce: [u8; 12],
    callback: Option<String>,
) -> anyhow::Result<()> {
    let mut conn_id = 0;
    // let (c2s_tx, mut c2s_rx) = tokio::sync::mpsc::channel::<Message>(32);
    let (s2c_tx, mut s2c_rx) = tokio::sync::mpsc::channel::<Message>(32);
    let conn_map: Mutex<HashMap<u32, Sender<Message>>> = Mutex::new(HashMap::new());

    let (ri, wi) = inbound.split();
    let ri = DecStream::new(ri, &key, &nonce);
    let wi = EncStream::new(wi, &key, &nonce);

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

    let client = reqwest::Client::new();
    let (domain, path) = match ri.try_next().await?.ok_or(anyhow::anyhow!("not message"))? {
        Message::ClientHello { domain, path } => {
            let path = path.unwrap_or("".to_string());

            (domain, path)
        }
        _ => Err(anyhow::anyhow!("invalid Message"))?,
    };

    let listener = TcpListener::bind("0.0.0.0:0").await?;
    let conn_uid = format!("conn-{}", hex::encode(uuid::Uuid::new_v4().as_bytes()));
    debug!(
        "handshake complete waiting uid={}, port={}",
        conn_uid,
        listener.local_addr()?.port()
    );

    let _on_shutdown = if let Some(callback) = callback {
        match client
            .post(&callback)
            .body(
                serde_json::to_string(&ExternalMessage::NewConnection {
                    conn_id: conn_uid.clone(),
                    domain: domain.clone(),
                    path: path.clone(),
                    port: listener.local_addr()?.port(),
                })
                .unwrap(),
            )
            .send()
            .await
        {
            Ok(_) => Some(OnShutdown {
                callback: callback.clone(),
                conn_id: conn_uid.clone(),
                domain: domain.clone(),
                path: path.clone(),
                port: listener.local_addr()?.port(),
            }),
            Err(err) => {
                error!("Error sending message to callback {:?}", err);
                None
            }
        }
    } else {
        None
    };
    wi.send(Message::ServerHello { domain, path }).await?;

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
                        info!("shutdown {:?}", message);
                        break;
                    },
                    _ => break,
                }
            }
        }
    }

    Ok(())
}
