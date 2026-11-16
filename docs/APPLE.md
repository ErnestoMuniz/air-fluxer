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
- **Configurações**: página "Voice & audio" no diálogo de configurações para escolher o microfone e a
  saída de som (System default ou um dispositivo da lista, com botão de atualizar). A escolha vale na
  hora, inclusive dentro de uma chamada.
- **Mensagens**: busca no servidor inteiro (ou no canal da DM) com `POST /search/messages`, resultados com
  canal/autor/data, e "pular para" carregando o trecho com `?around=` quando a mensagem está fora do
  histórico carregado; menu de contexto (Responder, Copiar texto, Editar e Apagar nas mensagens da própria
  conta), barra de resposta/edição com `message_reference`, e ack de leitura (`/read-states/ack`).
- **Login**: campo de instância (qualquer servidor Fluxer), além do `fluxer.com` padrão.
- **Configurações**: página de perfil responsiva; busca falsa e botão de usuário removidos; troca/remoção
  de avatar (PNG quadrado), fuso do perfil a partir do sistema, volume de entrada/saída, medidor do
  microfone e supressão de ruído; "Get Plutonium" abre a página da instância.
- **Membros**: filtro por nome/apelido.
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

- `cargo check` e `cargo test -p fluxer-app -p fluxer-client` (87 testes no app + 1 ignorado que abre os
  dispositivos, 29 no client).
- `--snap` headless nas cenas demo e `dm`.
- Janela real abrindo no Metal, com semáforos e menus Edit/Window do Rune; ⌘Q fecha.
- `list_devices()` lista microfones e saídas reais (HyperX, alto-falantes do MacBook, monitor) e
  um stream de playback abre e toca silêncio (`cargo test -p fluxer-app audio_devices -- --ignored`).

## Pendências

- **Microfone no bundle**: a primeira chamada pede permissão (TCC). Testar dentro do `Fluxer.app`
  assinado, não só no binário solto.
- **Distribuição gratuita**: a assinatura ad-hoc é a definitiva por enquanto (sem Developer ID). Quem
  baixar o `.app` precisa abrir com o botão direito → Abrir, ou rodar
  `xattr -dr com.apple.quarantine Fluxer.app`; o Gatekeeper não reconhece o binário na primeira vez.
- **Troca de dispositivo padrão**: o microfone segue o default do sistema (reabre em ~1 s quando ele
  muda); as saídas continuam abrindo o default no início de cada stream. AirPods/Bluetooth ainda
  merecem um teste de áudio real.
- **Volume do sistema** não é controlado; o app escala as amostras, como no Linux.
- **Câmera e compartilhamento de tela**: permissões e o caminho do LiveKit no macOS; hoje só dá para
  assistir ao stream de outras pessoas, não publicar o próprio.
- **Chamadas em DM** não existem; só canais de voz de servidor.
- **Badge no Dock**: falta (as notificações do sistema já saem para menção/DM).
- **Universal**: `UNIVERSAL=1 scripts/macos-bundle.sh` junta arm64 + x86_64 com lipo; o primeiro build
  x86_64 baixa o libwebrtc dele.
- **Integrações nativas**: o token agora vai para o Keychain do macOS (o JSON fica sem ele); faltam
  deep links, LaunchServices e auto-update.
- **Ícone**: gerado como tile arredondado (squircle) na cor da marca; só o logo pode mudar no futuro.
