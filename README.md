# rux

A native client for the Fluxer chat service, written in Rust on [Rune](https://github.com/anomalyco/rune),
a GPU UI framework. Sign-in (with e-mail authorization and two-factor), servers and direct messages, live
messages and history, replies, reactions, mentions, embeds, GIFs and video clips, uploads, the GIF, sticker and
emoji pickers, profiles, and voice calls (LiveKit, CoreAudio on macOS and PulseAudio on Linux, RNNoise noise
suppression, screen shares).

```
app/                the application (`fluxer-app`)
crates/fluxer-client/   Fluxer's REST API and gateway: no UI, no Rune
docs/               working notes
```

## Building

The app takes Rune from a local clone at `ref/rune` (`ref/` is not tracked). Last built against revision
`ea6ddf8` of its `dev` branch:

```sh
mkdir -p ref && git clone https://github.com/anomalyco/rune ref/rune
git -C ref/rune checkout ea6ddf8
```

You also need a Rust toolchain (see `rust-toolchain.toml`) and `clang` (the C++ glue of libwebrtc is built with it,
see `.cargo/config.toml`). On Linux, also the PulseAudio client library (`libpulse`). macOS uses the system's
CoreAudio and needs no extra packages; the first voice call asks for microphone access.

```sh
cargo run --release -p fluxer-app           # the app
FLUXER_DEMO=1 cargo run --release -p fluxer-app   # with made-up data, no account
cargo test -p fluxer-app -p fluxer-client
```

### macOS app

```sh
scripts/macos-bundle.sh                     # → target/release/Fluxer.app
open target/release/Fluxer.app
```

The bundle carries the microphone and camera usage strings and is ad-hoc signed, which is enough to run it
on this Mac. Shipping it to other people also needs a Developer ID signature and notarisation. See
`docs/APPLE.md` for what the macOS port does and what is still missing.

## Looking at it without a window

`-- --snap out.png [--scene NAME]` draws one frame to a PNG. Scenes: `login`, `ip`, `code`, `call`, `call-watch`,
`call-ping`, `dm`, `scroll`, `settings`, `settings-guild`, `settings-picker`, `settings-longbio`, `extras`, `quiet`,
`farjump`, `mentions`.

## Where it keeps things

| | |
|---|---|
| session, last place, preferences, audio settings | `~/Library/Application Support/fluxer-rune/` on macOS, `~/.config/fluxer-rune/` on Linux (`FLUXER_RUX_CONFIG_DIR`) |
| downloaded pictures (cut down to 256 MB at launch) | `~/Library/Caches/fluxer-rune/media/` on macOS, `~/.cache/fluxer-rune/media/` on Linux (`FLUXER_RUX_CACHE_DIR`) |

## Memory and CPU

Pictures are the big spender. Animated GIF, WebP and APNG are not decoded up front: `app/src/anim.rs` keeps the
encoded bytes and decodes one frame at a time, only while the picture is on screen and the window is focused.
The loaded pictures are trimmed (`Net::trim`): animations that nothing asks for any more are let go after a few
seconds, the rest stay within a budget.
