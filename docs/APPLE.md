# Versão Apple (macOS)

Estado da branch `apple-version`: o app compila, roda e desenha no macOS (testado em macOS 26,
arm64). O áudio do sistema é o CoreAudio; o Linux continua no PulseAudio. A chamada de voz real
(LiveKit) ainda não foi exercitada de ponta a ponta numa conta.

## O que mudou

- **Áudio** (`app/src/audio/`): `mod.rs` guarda o que é comum (preferências, nível, quem fala);
  `pulse.rs` é o backend Linux que já existia; `macos.rs` é o CoreAudio via `cpal`. O resto do app
  continua falando 48 kHz mono i16; um dispositivo que só oferece outra taxa é reamostrado
  linearmente (voz, não música). Listagem de microfones e saídas, `drain` para o som não ser
  cortado e limite de fila para um consumidor travado não crescer sem fim.
- **Caminhos nativos** (`store.rs`, `net.rs`): `~/Library/Application Support/fluxer-rune/` e
  `~/Library/Caches/fluxer-rune/media/`; `FLUXER_RUX_CONFIG_DIR`/`FLUXER_RUX_CACHE_DIR` continuam
  valendo.
- **Janela** (`main.rs`, `ui.rs`): conteúdo ocupa a janela toda (sem barra de título opaca, que
  ficaria clara no tema sempre escuro do app); a rail desce 40 pt para os semáforos do macOS não
  caírem em cima do ícone de DMs. A faixa do título do AppKit continua arrastando a janela.
- **Sobre** (`ui/settings.rs`): "macOS 26.2 (arm64)" via `sw_vers`, lido uma vez.
- **Bundle** (`packaging/macos/`, `scripts/macos-bundle.sh`): `Fluxer.app` com `Info.plist`
  (microfone, câmera, versão), ícone gerado do `app/assets/logo.svg` e assinatura ad-hoc com
  hardened runtime + entitlements de áudio/câmera.
- **Testes**: o gateway falso ignora conexões que não são handshake WebSocket (scanners locais
  derrubavam o teste de reconexão); o teste de custo de frame mede CPU da thread, não parede,
  então outros testes rodando em paralelo não o fazem falhar.
- **WebRTC/macOS**: as categorias Objective-C do libwebrtc (por exemplo
  `+[NSString stringForStdString:]`) vivem numa biblioteca estática. Sem `-ObjC` o linker as
  descarta e criar a `PeerConnectionFactory` aborta com SIGABRT na hora de entrar numa call
  (`+[NSString stringForStdString:]: unrecognized selector`). `app/build.rs` passa a flag no
  macOS; há um teste que cria a factory contra um endpoint morto para não regredir.

## Verificado

- `cargo check` e `cargo test -p fluxer-app -p fluxer-client` (81 testes no app + 1 ignorado que abre os
  dispositivos, 29 no client).
- `--snap` headless nas cenas demo e `dm`.
- Janela real abrindo no Metal, com semáforos e menus Edit/Window do Rune; ⌘Q fecha.
- `list_devices()` lista microfones e saídas reais (HyperX, alto-falantes do MacBook, monitor) e
  um stream de playback abre e toca silêncio (`cargo test -p fluxer-app audio_devices -- --ignored`).

## Pendências

- **Microfone no bundle**: a primeira chamada pede permissão (TCC). Testar dentro do `Fluxer.app`
  assinado, não só no binário solto.
- **Distribuição**: Developer ID + notarização + `stapler`; hoje é assinatura ad-hoc.
- **Troca de dispositivo padrão** em tempo real: o `cpal` não segue o default do sistema sozinho;
  hoje só ao reabrir o stream (entrar na chamada/menu). AirPods e Bluetooth merecem um teste.
- **Volume do sistema** não é controlado; o app escala as amostras, como no Linux.
- **Câmera e compartilhamento de tela**: permissões e o caminho do LiveKit no macOS.
- **Universal**: só `aarch64-apple-darwin` foi construído; falta o `x86_64` (e o libwebrtc dele).
- **Integrações nativas**: keychain para a sessão, notificações, deep links, LaunchServices.
- **Ícone**: hoje é o círculo do logo; o estilo do macOS costuma ser um "squircle" com margem.
