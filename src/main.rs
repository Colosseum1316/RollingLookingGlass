use clap::ArgAction;
use clap::Parser;
use flashlight::create_varint;
use rolling_glass::{is_known_protocol_number, ProtocolNum};
use serde_json::json;
use std::error::Error;
use std::net::SocketAddr;
use std::slice;
use time::macros::format_description;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::level_filters::LevelFilter;
use tracing::{debug, info, warn};
use tracing_subscriber::fmt::time::OffsetTime;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Parser)]
#[command(long_about = None, disable_help_flag = true)]
struct RollingLookingGlassArguments {
    #[arg(short, long = "help", action = ArgAction::Help, help = "Print this help information")]
    _help: Option<bool>,

    #[arg(
        short,
        long = "address",
        help = "Listening address",
        default_value = "127.0.0.1:25565"
    )]
    address: String,

    #[arg(short, long = "brand", default_value = "Void")]
    brand: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = RollingLookingGlassArguments::parse();
    tracing_subscriber::registry()
        .with(fmt::layer().with_timer(OffsetTime::new(time::UtcOffset::current_local_offset()?, format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3][offset_hour sign:mandatory]:[offset_minute]"))).with_target(false))
        .with(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .init();

    debug!("Debug logging enabled");
    let listener = TcpListener::bind(&args.address).await?;
    info!("Listening on {}", &args.address);
    info!("Brand name: {}", &args.brand);

    let brand: &'static str = Box::leak(args.brand.into_boxed_str());
    loop {
        let (socket, addr) = listener.accept().await?;
        tokio::spawn(async move {
            info!("New connection from {}", &addr);
            if let Err(e) = handle_packets(socket, &addr, brand).await {
                warn!("{} error: {}", &addr, e);
            }
            info!("Connection from {} is closed", &addr);
        });
    }
}

// https://minecraft.wiki/w/Java_Edition_protocol/Packets#Handshake
static HANDSHAKE_MAX_LENGTH: usize = 263usize;
static PING_REQUEST_LENGTH: usize = 9usize;

async fn handle_packets(
    mut socket: TcpStream,
    addr: &SocketAddr,
    brand: &str,
) -> Result<(), Box<dyn Error>> {
    let resize = read_varint(&mut socket).await?;
    let handshake_length = resize;
    if handshake_length > HANDSHAKE_MAX_LENGTH {
        debug!("{} sent a handshake packet that's too large", &addr);
        return Err(Box::from("Handshake packet too large"));
    }

    let mut byte: u8 = 255u8;

    socket.read_exact(slice::from_mut(&mut byte)).await?;
    if byte != 0u8 {
        debug!(
            "{} sent a handshake packet that has incorrect packet id",
            &addr
        );
        return Err(Box::from("Unknown packet id"));
    }

    let rev = read_varint(&mut socket).await?;
    let protocol_number = rev;
    if !is_known_protocol_number(protocol_number as ProtocolNum) {
        debug!(
            "{} sent a handshake packet that has unknown protocol number",
            &addr
        );
        return Err(Box::from("Unknown protocol number"));
    }
    debug!("Read protocol number {} from {}", protocol_number, &addr);

    // Read server address. It's not used.
    let resize = read_varint(&mut socket).await?;
    if resize > 255 {
        debug!("{} sent a server address that's too long", &addr);
        return Err(Box::from("Server address too long"));
    }
    socket.read_exact(&mut vec![0u8; resize]).await?;
    debug!("Read server address from {}", &addr);

    // Read server port number. It's not used.
    socket.read_u16().await?;
    debug!("Read server port number from {}", &addr);

    // Read intent number. It must be either 1 or 2.
    socket.read_exact(slice::from_mut(&mut byte)).await?;
    if byte != 1u8 && byte != 2u8 {
        return Err(Box::from(
            "The intent number must be either 1 (Status) or 2 (Login).",
        ));
    }
    debug!("Read intent number {} from {}", byte, &addr);

    if byte == 1u8 {
        let resize = read_varint(&mut socket).await?;
        if resize != 1 {
            return Err(Box::from("Not a Status Request"));
        }
        // Status Request
        socket.read_exact(slice::from_mut(&mut byte)).await?;
        if byte != 0u8 {
            debug!(
                "{} sent a Status Request packet that has incorrect packet id",
                &addr
            );
            return Err(Box::from("Unknown packet id"));
        }
        debug!("Read Status Request from {}", &addr);

        // Status Response
        let payload = json!({
            "version": json!({
                "name": brand,
                "protocol": protocol_number
            }),
            "players": json!({
                "max": 0,
                "online": 0,
                "sample": []
            })
        })
        .to_string();
        let strlen = payload.len();
        let strlen_varint = create_varint(strlen as i32);
        let packet_len = 1 + strlen_varint.len() + strlen;
        let packet_len_varint = create_varint(packet_len as i32);
        debug!("Writing Status Response to {}", &addr);
        socket.write_all(&packet_len_varint).await?;
        socket.write_u8(0x00).await?;
        socket.write_all(&strlen_varint).await?;
        socket.write_all(payload.as_bytes()).await?;

        debug!("Waiting for Ping Request from {}", &addr);
        // Ping Request
        let resize = read_varint(&mut socket).await?;
        let ping_request_length = resize;
        if ping_request_length == PING_REQUEST_LENGTH {
            socket.read_exact(slice::from_mut(&mut byte)).await?;
            if byte != 1u8 {
                debug!(
                    "{} sent a Ping Request packet that has incorrect packet id",
                    &addr
                );
                return Err(Box::from("Unknown packet id"));
            }

            debug!("Writing Ping Response to {}", &addr);
            let ping_long = socket.read_i64().await?;
            // Pong Response
            socket.write_u8(PING_REQUEST_LENGTH as u8).await?;
            socket.write_all(slice::from_mut(&mut byte)).await?;
            socket.write_i64(ping_long).await?;
        } else {
            return Err(Box::from("Not a Ping Request"));
        }
    } else {
        read_varint(&mut socket).await?;

        socket.read_exact(slice::from_mut(&mut byte)).await?;
        if byte != 0u8 {
            debug!(
                "{} sent a Login Start packet that has incorrect packet id",
                &addr
            );
            return Err(Box::from("Unknown packet id"));
        }

        let resize = read_varint(&mut socket).await?;
        let ign_length = resize;
        if ign_length == 0 || ign_length > 16 {
            debug!("{} sent an illegal username which is too long", &addr);
            return Err(Box::from("Username too long"));
        }
        // Immediately send Disconnect (Login), the rest of the buffer is ignored.
        let payload = json!({
            "text": format!("Your IP address is {}", &addr.ip()),
        })
        .to_string();
        let strlen = payload.len();
        let strlen_varint = create_varint(strlen as i32);
        let packet_len = 1 + strlen_varint.len() + strlen;
        let packet_len_varint = create_varint(packet_len as i32);
        debug!("Writing Disconnect (Login) packet to {}", &addr);
        socket.write_all(&packet_len_varint).await?;
        socket.write_u8(0x00).await?;
        socket.write_all(&strlen_varint).await?;
        socket.write_all(payload.as_bytes()).await?;
        socket.shutdown().await?;
    }

    Ok(())
}

async fn read_varint(stream: &mut TcpStream) -> Result<usize, Box<dyn Error>> {
    let mut byte = 0x00;
    let mut res = 0i32;
    for i in 0.. {
        if i > 5 {
            return Err(Box::from("Not a valid varint"));
        }
        let buf = slice::from_mut(&mut byte);
        stream.read_exact(buf).await?;
        if buf.is_empty() {
            break;
        }
        res |= ((buf[0] as i32) & 0x7Fi32) << (7 * i);
        if ((buf[0] as i32) & 0x80i32) == 0 {
            break;
        }
    }
    if res <= 0 {
        return Err(Box::from("Varint not bigger than 0"));
    }
    Ok(res as usize)
}
