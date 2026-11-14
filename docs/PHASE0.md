# Fluxer → Rust + Rune: Fase 0 (levantamento)

Repos clonados (shallow) em `ref/fluxer` e `ref/rune`.

## 1. Tamanho do cliente (`fluxer_app/src`)
~1470 `.tsx` + ~1920 `.ts`, 47 features. Maiores (arquivos .ts/.tsx):

| Feature | Arquivos | Notas |
|---|---|---|
| voice | 442 | LiveKit (`pkgs/livekit-client`), maior risco |
| channel | 422 | lista de mensagens, header, layout |
| app | 366 | shell, bootstrap, router |
| user | 308 | perfil, configurações |
| ui | 279 | componentes base (avatar, slider, menu, overlay...) |
| auth | 250 | login, registro, MFA |
| messaging | 233 | composer, envio, reações |
| guild | 147 | servidores, canais |
| platform | 122 | integração desktop/web |
| expressions / premium / lexical / theme_settings | 74 / 72 / 63 / 53 | emoji/GIF, planos, editor rico (Lexical) |

Menores: threads, moderation, search, forum, notification, invite, discovery, emoji, relationship, permissions, read_state, gateway (43, protocolo WS: `events`, `snapshot`, `transport`, `types`).

Já em Rust no repo: `fluxer_app/rust/libfluxcore` (zstd) e `libfluxwebp` — reaproveitáveis direto.

## 2. Mapa de componentes: Rune-kit ↔ Fluxer

| Necessidade Fluxer | Rune (`rune-kit`) | Status |
|---|---|---|
| Botão, Switch, Checkbox, Radio, Segmented | `button`/`actions`, `switch`, `checkbox`, `radio`, `segmented` | ✅ |
| Select, Combobox, Menu, Popover, Dialog, Toast | `select`, `combobox`, `menu`, `popover`, `dialog`, `toast`, `overlay` | ✅ |
| Input de texto, Field/forms | `text_input`, `field` | ✅ (validar IME) |
| Accordion, Collapsible, Card, Badge, Table, List, Tree | idem | ✅ |
| Scroll, painéis, sidebar, split | `scroll_bar`, `shell/{sidebar,pane,split}` | ✅ |
| Markdown (mensagens), syntax highlight | `markdown/*`, `syntax` | ✅ parcial (spoilers, menções, emoji custom faltam) |
| Atalhos | `keybind`, `shortcut_recorder` | ✅ |
| Command palette (quick switcher) | `palette` | ✅ |
| Avatar + status + badge de menção | — | 🔧 criar (`fluxer-ui`) |
| Slider (volume), Pagination, Spinner, InlineEdit | — | 🔧 criar |
| Lista de mensagens virtualizada, scroll ancorado | Rune tem índice de altura (sum-tree) no core | 🔧 criar sobre o core |
| Composer rico (menções, emoji, anexos) — hoje Lexical | `text_input` + rope no core | 🔧 maior item de UI |
| Imagens/GIF/vídeo/embeds, WebP | `pixels`, `image` no core; `libfluxwebp` | 🔧 pipeline de mídia |
| Seletor de emoji/GIF, drag-and-drop, bottom sheet (mobile) | — | 🔧 criar / adiar mobile |
| Voz/vídeo | — | 🔧 fora do Rune (LiveKit Rust SDK + `cpal`) |
| Tema do Fluxer | `Kit` + theme crate (modelo `themes/opencode`) | 🔧 `themes/fluxer` a partir do CSS |

## 3. Riscos validados no código do Rune
- Pré-release: pinar `rev` de `dev`. PLAN.md do Rune lista como riscos principais **texto/IME** e escopo.
- IME e a11y: Rune implementa protocolos nativos + AccessKit; testar em Linux (Wayland) cedo.
- Shell hoje via winit; tray, deep links e notificações nativas ficam por nossa conta.

## 4. Spike (próximo passo, ~1 semana)
Binário `fluxer-app` mínimo:
1. Login (REST) → token.
2. Conectar ao gateway WS e receber `READY`.
3. Lista de guilds/canais (`sidebar`) + lista de mensagens virtualizada (1k+ itens) + composer simples.
4. Render headless em PNG para teste visual.

**Critério de aprovação:** scroll a 60+ fps com 10k mensagens, digitação com IME (pt-BR/CJK) funcionando, texto legível comparado ao app atual.

## 5. Estrutura de workspace proposta
```
rux/
  Cargo.toml            # workspace; rune = { git, rev = "<dev rev>" }
  crates/
    fluxer-proto/       # modelos API/gateway (derivar dos tipos TS)
    fluxer-client/      # REST + gateway WS + cache + auth
    fluxer-ui/          # componentes próprios sobre rune-kit
    fluxer-voice/       # LiveKit/áudio (fase 4)
  themes/fluxer/        # tokens do Fluxer como dados
  app/                  # binário
```
