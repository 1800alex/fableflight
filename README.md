
<div align="center">

# FableFlight

### An Open-World Flight Simulator Built in Pure Rust

Explore the skies with real-world weather, live air traffic, and an immersive open-world flying experience.

*Created with Claude Fable 5.2, Opus 5.5 and other AI's*

<br>

[![Discord](https://img.shields.io/badge/Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white)](https://discord.gg/9akYzMRBc6)
[![Rust](https://img.shields.io/badge/Rust-CE422B?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org/)
<br>
</div>
<div align="center">
  <video
    src="https://github.com/user-attachments/assets/4f8833da-13c0-4d20-a50b-c0acf2d88df1"
    width="800"
    controls>
  </video>
</div>

## Download
Download the [latest release](https://github.com/Rolandjg/fableflight/releases/latest)

## Compiling

Install [Rust](https://www.rust-lang.org/tools/install), then run:

```sh
cargo run --release
```

Or build with `cargo build --release`.

## Features

- **Aircraft:** Cessna 172RG, Robin DR400, Boeing 747, F/A-18, F-4, F-22, Concorde, SR-71, Starship, and downloadable [FlightGear aircraft](https://wiki.flightgear.org/FGAddon).
- **Flight systems:** fuel, mass and balance, electrical failures, clickable cockpits, radio navigation, and autopilot.
- **World:** real METAR weather, satellite/road minimap, live ADS-B traffic, and multiplayer.
- **Controls:** remappable keyboard, joystick, throttle, and pedals. Configure them in **Settings → Bindings**.
- **Arcade mode:** optional boosts, missiles, and in-flight respawns. Enable in **Settings → General** or pass `--arcade`.
- **AI copilot:** optional MCP integration; see [docs/mcp.md](docs/mcp.md).

The UI supports English and French. Settings and key bindings are saved automatically.

## Multiplayer

Connect from the **Multiplayer** settings. To host a relay:

```sh
cd relay
docker compose up -d
```
Allow UDP port `7480` through your firewall.

Read [docs/relay.md](docs/relay.md) for more info.

## Contributing

Contributions are welcome, see [AGENTS.md](AGENTS.md) for guidance on coding agent standards.

