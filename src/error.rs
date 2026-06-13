#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Protobuf decode error: {0}")]
    Decode(#[from] prost::DecodeError),

    #[error("Protobuf encode error: {0}")]
    Encode(#[from] prost::EncodeError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("API error: {0}")]
    Api(String),

    #[error("Request timeout")]
    Timeout,

    #[error("Connection closed")]
    Disconnected,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
