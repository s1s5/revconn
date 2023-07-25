use clap::Parser;
use futures::{SinkExt, TryStreamExt};
use revconn::encstream::{DecStream, EncStream};
use revconn::protocol::Message;
use revconn::util::{get_key_and_nonce_from_env, handle_connection};
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::net::TcpStream;
use tokio::sync::mpsc::Sender;
use tracing::{debug, info};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long)]
    server: String,

    #[arg(long)]
    backend: String,

    #[arg(long)]
    domain: String,

    #[arg(long)]
    path: Option<String>,
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

    let mut conn = TcpStream::connect(args.server).await?;

    let (ri, wi) = conn.split();
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

    wi.send(Message::ClientHello {
        domain: args.domain,
        path: args.path,
    })
    .await?;
    debug!("send hello to server");

    match ri.try_next().await?.ok_or(anyhow::anyhow!("not message"))? {
        Message::ServerHello { domain, path } => {
            println!("{}, {}", domain, path);
        }
        _ => Err(anyhow::anyhow!("fail handshaking"))?,
    }
    debug!("get hello from server");

    let (gtx, mut grx) = tokio::sync::mpsc::channel::<Message>(32);
    let conn_map: Mutex<HashMap<u32, Sender<Message>>> = Mutex::new(HashMap::new());

    loop {
        tokio::select! {
            message = ri.try_next() => {
                let message = match message? {
                    Some(message) => message,
                    None => Err(anyhow::anyhow!("connection closed"))?,
                };
                debug!("get message from server, {:?}", message);

                match message {
                    Message::NewConnection { id } => {
                        let (tx, rx) = tokio::sync::mpsc::channel::<Message>(32);
                        {
                            let mut m = conn_map.lock().unwrap();
                            m.insert(id, tx);
                        }

                        let conn = TcpStream::connect(args.backend.clone()).await?;
                        let gtx = gtx.clone();
                        tokio::spawn(handle_connection(id, rx, gtx, conn));
                    }
                    Message::Data { id, data } => {
                        let tx = match conn_map.lock().unwrap().get(&id) {
                            Some(tx) => tx.clone(),
                            None => Err(anyhow::anyhow!("key not found in conn map"))?,
                        };
                        tx.send(Message::Data {id, data}).await?;
                    }
                    Message::CloseConnection { id } => {
                        let mut m = conn_map.lock().unwrap();
                        m.remove(&id);
                    }
                    Message::Shutdown { message } => {
                        info!("shutdown {:?}", message);
                        break;
                    }
                    _ => {
                        Err(anyhow::anyhow!("unknown message type from server"))?;
                    }
                }
            }
            message = grx.recv() => {
                let message = message.ok_or(anyhow::anyhow!("no message found"))?;
                debug!("get message from proxy {:?}", message);
                match message {
                    Message::Data{id, data} => {
                        wi.send(Message::Data { id, data }).await?;
                    }
                    Message::CloseConnection{id} => {
                        let mut m = conn_map.lock().unwrap();
                        m.remove(&id);
                    }
                    _ => {
                        Err(anyhow::anyhow!("unknown message type from proxy"))?;
                    }
                }
            }
        }
    }

    Ok(())
}
