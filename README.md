# rustplus

A Rust client for the Rust+ companion app protocol. This library provides a native, asynchronous interface for interacting with Rust game servers.

## Features

- **Asynchronous API:** Built on `tokio` and `tokio-tungstenite`.
- **Full Protocol Coverage:** Support for server info, map markers, team chat, and entity control.
- **Camera Support:** Native decoding of camera rays into PNG frames using the `image` crate.
- **Automatic Rate Limiting:** Transparently manages token buckets for IP and Player ID limits.
- **Strongly Typed:** Automatically generated Protobuf bindings via `prost`.

## Usage

```rust
use rustplus::RustPlusClient;

#[tokio::main]
async fn main() -> rustplus::Result<()> {
    let mut client = RustPlusClient::new(
        "127.0.0.1", // Server IP
        28082,       // App Port
        7656119...,  // Steam ID
        -12345678,   // Player Token
        false,       // Use Facepunch Proxy
    );

    client.connect().await?;

    let info = client.get_info().await?;
    if let Some(resp) = info.response {
        if let Some(get_info) = resp.get_info {
            println!("Server Name: {}", get_info.name);
        }
    }

    client.send_team_message("Hello from Rust!").await?;

    Ok(())
}
```

## Camera Usage

```rust
let mut camera = client.get_camera("DOME1");
camera.subscribe().await?;

let mut frames = camera.subscribe_frames();
while let Ok(png_bytes) = frames.recv().await {
    // png_bytes contains the rendered frame
}
```

## Configuration

The client respects the following default server limits:
- IP Limit: 50 tokens, 15/sec replenishment.
- Player Limit: 25 tokens, 3/sec replenishment.
