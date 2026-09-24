> **Historical — superseded 2026-09-23.**
> This document describes the implementation as it stood before the audit and
> rebuild, and parts of it are no longer true (the Booster `AdwSwitchRow`, the
> old game detection and the old Gamescope builder no longer exist). It is kept
> for context, not as a reference.
> Current documentation: [README.md](README.md).

# Arquitetura do BiGameMode

Este documento resume as tecnologias, os componentes e o fluxo de execução do BiGameMode.

## Visão Geral

O projeto é dividido em 3 crates Rust:

- `bigame-ui`: interface GTK4/libadwaita
- `bigame-core`: lógica de negócio, integração com sistema, launch orchestration
- `bigame-daemon`: daemon root via D-Bus para operações privilegiadas

Fluxo de alto nível:

```text
UI (bigame-ui)
  -> core (bigame-core)
  -> D-Bus system bus
  -> bigame-daemon (root)
  -> falcond / sistema
  -> status em /tmp/falcond_status
  -> polling no Dashboard
```

## Tecnologias por Camada

### Linguagem e Runtime

- Rust 2024 edition
- Tokio (async runtime)
- zbus (D-Bus async + blocking)
- Serde + TOML
- anyhow
- tracing + tracing-subscriber

### Interface

- GTK4
- libadwaita
- gettext-rs (i18n)
- ksni (StatusNotifier/tray)

### Sistema Linux

- D-Bus (session + system)
- systemd (service units)
- polkit
- Power Profiles Daemon (`net.hadess.PowerProfiles`)
- GameMode (`com.feralinteractive.GameMode`)
- sysfs/procfs para telemetria e runtime checks

### Ecossistema gamer integrado

- falcond
- lsfg-vk
- gamescope
- mangohud (opcional)
- sched-ext/scx
- AMD 3D V-Cache controls

### Build e distribuição

- Cargo
- Meson
- Flatpak (`org.gnome.Platform`)
- PKGBUILD (Arch/BigLinux)
- gettext toolchain (`xgettext`, `msgmerge`, `msgfmt`)

## Componentes

### `bigame-ui`

Responsável por:

- Dashboard de status/telemetria
- Wizard de perfil
- Aba de vídeo avançado (upscaling/framegen)
- Diagnósticos em runtime
- Ações de launch via UI

### `bigame-core`

Responsável por:

- leitura/escrita de perfis
- leitura/escrita de config global de vídeo (`video.toml`)
- `LaunchPlan` (gamescope/env vars/framegen)
- helpers de conflito (ex.: lsfg-vk vs OptiScaler/AFMF)
- parsing de status de `falcond`
- integração D-Bus com PowerProfiles/GameMode/daemon

### `bigame-daemon`

Responsável por métodos privilegiados via D-Bus:

- `SaveProfile`
- `DeleteProfile`
- `ApplyFalcondConfig`
- `SetCpuGovernor`
- `SetVcacheMode`

## Fluxo `Launch (Turbo)`

```text
1) Usuário clica "Launch (Turbo)" no Dashboard
2) UI resolve comando de launch (Steam URI ou executável direto)
3) UI carrega perfil + video_config
4) core::launcher::LaunchPlan::build(...)
   - valida Turbo ativo (PowerProfiles = performance)
   - monta args de gamescope (quando aplicável)
   - injeta env vars (Wine FSR/vkBasalt/AFMF etc.)
   - faz staging de OptiScaler (quando habilitado)
5) spawn do processo
6) falcond detecta processo e aplica perfil
7) falcond atualiza /tmp/falcond_status
8) Dashboard atualiza status visual por polling
```

## Fluxo de Perfil (Wizard)

```text
1) Dashboard/Profiles abre wizard
2) Usuário preenche passos
3) Save chama bigame_core::profiles::save(...)
4) core persiste perfil via D-Bus no daemon root
5) sync lsfg-vk é best-effort (não bloqueia save do perfil)
6) UI fecha wizard, exibe toast, e atualiza lista de perfis
```

## Observabilidade e Debug

Logs relevantes:

- `dashboard create-profile wizard opened`
- `wizard save requested`
- `wizard save succeeded`
- `dashboard launch requested`
- `dashboard launch succeeded`
- `dashboard launch failed`
- `dashboard runtime status changed`

Comando recomendado:

```bash
cd bigame-engine
BIGAME_LOCALEDIR=../locale RUST_LOG=info cargo run -p bigame-ui
```

Diagnóstico rápido dentro da UI:

- Dashboard -> Video Runtime Status -> `Run Diagnostics`
- opcional: salvar relatório (`~/bigame-diagnostics.log`)

## Limitações conhecidas

- Em alguns jogos Wine/Proton, matching por nome de processo pode cair em executáveis genéricos se o perfil for mal nomeado.
- Para Steam, o launch pode exigir URI (`steam://rungameid/...`) em vez de executável direto.
- Compatibilidade de `lsfg-vk` depende da versão do arquivo de configuração instalado no sistema.

## Roadmap sugerido

- Migrar detecção de jogo ativo para identificação por árvore de processo/PPID em vez de nome simples.
- Expor no wizard um campo "processo detectado recentemente" para seleção guiada.
- Adicionar testes de integração para launch path Steam/Lutris/Heroic.
