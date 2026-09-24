> **Historical — superseded 2026-09-23.**
> This document describes the implementation as it stood before the audit and
> rebuild, and parts of it are no longer true (the Booster `AdwSwitchRow`, the
> old game detection and the old Gamescope builder no longer exist). It is kept
> for context, not as a reference.
> Current documentation: [README.md](README.md).

# BiGameMode - Chat Knowledge Base

Purpose: durable context for future chat sessions.
Scope: architecture, runtime behavior, build/test, packaging, diagnostics, and recent fixes.
Language: PT-BR (ASCII-friendly).

## 1) O que e o app
BiGameMode e uma suite de otimizacao gamer para BigLinux.
Ele orquestra:
- falcond (daemon de performance)
- gamescope (compositor gamer)
- frame generation (lsfg-vk, AFMF, OptiScaler)
- ajustes de governor/power profile
- telemetria e diagnostico em UI GTK4/libadwaita

## 2) Estrutura do projeto
Workspace principal: `bigame-engine/`
Crates:
- `bigame-core/`: logica de negocio, launcher, integracoes sistema, config
- `bigame-ui/`: UI GTK4/libadwaita, dashboard, wizard, views
- `bigame-daemon/`: operacoes privilegiadas via D-Bus

Outros diretorios:
- `data/`: unit files, desktop, policy, metainfo, recursos
- `locale/`: traducao `.po/.mo`
- `docs/`: documentacao arquitetural
- `PKGBUILD`: empacotamento Arch/BigLinux

## 3) Fluxo funcional principal
### Launch (Turbo)
1. Usuario clica `Launch (Turbo)` no Dashboard.
2. UI resolve comando de launch (Steam `-applaunch` ou executavel direto).
3. UI carrega perfil + video config.
4. `bigame_core::launcher::LaunchPlan` monta comando/env.
5. Processo e spawnado.
6. falcond detecta processo e aplica perfil.
7. Dashboard atualiza status runtime em polling.

### Wizard/perfis
1. Usuario cria/edita perfil.
2. Persistencia via core + daemon D-Bus.
3. UI atualiza lista/estado e exibe feedback.

## 4) Tecnologias
- Rust (workspace)
- GTK4 + libadwaita
- zbus (D-Bus)
- tokio
- serde + toml
- tracing
- gettext-rs
- systemd/polkit

## 5) Build e validacao rapida
No root `bigame-engine/`:
- `cargo build --release`
- `cargo test --release`
- `cargo check -p bigame-ui`
- `cargo test -p bigame-core launcher::tests::`

Observacao:
- `clippy -D warnings` pode falhar por warnings preexistentes em partes nao tocadas.

## 6) Diagnostico e logs
### Logs relevantes da UI
- `dashboard launch requested`
- `dashboard launch succeeded`
- `dashboard launch failed`
- `LSFG-VK active for current game context`

### Diagnostico in-app
Dashboard -> Video Runtime Status -> `Run Diagnostics`

### Indicadores comuns de runtime (nao necessariamente erro fatal)
- `WARNING: radv is not a conformant Vulkan implementation`
- `LD_PRELOAD ... gameoverlayrenderer.so ... wrong ELF class`

## 7) Integracoes/Dependencias externas
- falcond
- lsfg-vk
- gamescope
- Steam/Lutris/Heroic detection
- power profiles daemon

## 8) Status conhecido de implementacao
Pontos fortes:
- app funcional, UI robusta, telemetria, perfis, launch path, daemon.

Pontos pendentes:
- reduzir caminhos shell privilegiados remanescentes
- melhorar camada de erros amigaveis ao usuario
- expandir CI e alguns estados UX (status page/banner)

## 9) Fix recente importante (Steam + LSFG conflito)
Contexto do bug:
- Em launch Steam (`steam -applaunch <appid>`), o launcher aplicava politica de conflito usando `executable`.
- Para Steam, `executable` era `steam`, nao o nome real do jogo.
- Resultado: tentativa de desativar LSFG para `steam` (errado), mantendo LSFG ativo no perfil real do jogo.
- Sintoma: jogo podia fechar logo apos abrir; log mostrava `LSFG-VK active` e depois stop rapido.

Root cause:
- Identificador logico do jogo estava acoplado ao binario de launch.

Correcao aplicada:
- Novo metodo em launcher:
  - `LaunchPlan::build_with_args_for_game(executable, executable_args, logical_game, video, gs_override)`
- Politica de harmonia e checagem de conflito agora usam `logical_game`.
- Dashboard atualizado para passar `exe_for_launch` como `logical_game` no path Steam.

Arquivos alterados nessa correcao:
- `bigame-engine/bigame-core/src/launcher.rs`
- `bigame-engine/bigame-ui/src/views/dashboard.rs`

Validacao executada:
- `cargo test -p bigame-core launcher::tests::` -> OK
- `cargo check -p bigame-ui` -> OK

## 10) Config de Copilot (ambiente do usuario) - nota operacional
Para evitar erro 400 de reasoning effort com Claude Opus 4.7:
- Em `~/.config/Code - Insiders/User/settings.json` manter:
  - `github.copilot.chat.responsesApiReasoningEffort = "medium"`
  - `github.copilot.chat.anthropic.thinking.effort = "medium"`

## 11) Proxima abordagem recomendada quando jogo nao abre
1. Confirmar se falha tambem fora do BiGameMode (Steam direto).
2. Coletar log apos `dashboard launch succeeded`.
3. Verificar se `LSFG-VK active` aparece no momento do launch.
4. Validar perfil LSFG por jogo (`active_in` + multiplier > 1).
5. Rodar Runtime Diagnostics no Dashboard e anexar resultado.

---
Este arquivo e intencionalmente pratico para retomada rapida por chat/agent.
