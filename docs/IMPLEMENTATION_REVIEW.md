> **Historical — superseded 2026-09-23.**
> This document describes the implementation as it stood before the audit and
> rebuild, and parts of it are no longer true (the Booster `AdwSwitchRow`, the
> old game detection and the old Gamescope builder no longer exist). It is kept
> for context, not as a reference.
> Current documentation: [README.md](README.md).

# BiGameMode - Revisao Validada da Implementacao (23.04.2026)

Este arquivo substitui a revisao especulativa anterior por uma auditoria baseada no codigo real do repositorio.

## Metodologia
- Leitura direta dos modulos principais de UI, core, daemon, empacotamento e i18n.
- Verificacao executada no workspace Rust:
	- `cargo build --release` -> OK
	- `cargo test --release` -> OK (`58` testes passando em `bigame-core`)
	- `cargo clippy --all-targets --all-features -- -D warnings` -> FALHA por warnings preexistentes de documentacao/estilo em Rust
- Verificacao do fluxo Meson nao executada por falta do binario `meson` no ambiente desta auditoria.

## Resumo Executivo

O projeto esta funcional em runtime e ja implementa a maior parte do que importa para uso real: dashboard com telemetria, UI GTK4/libadwaita com navegacao lateral, daemon root via D-Bus, gerenciamento de perfis, deteccao de jogos instalados, tray icon e PKGBUILD com build/test.

O que estava incorreto na documentacao anterior:
- O despacho por D-Bus nao e total. Operacoes privilegiadas importantes usam D-Bus, mas ainda existem acoes shell na UI e no core.
- A parte de empacotamento/i18n estava superestimada. Havia metadata desatualizada em `locale/POTFILES.in` e no `meson.build` da raiz.
- Os itens de UX mais sofisticados sugeridos no review original ainda nao foram todos implementados.

## 1. UX/UI e Design

### Implementado
- Dashboard com telemetria em tempo real:
	- CPU, GPU, temperatura, RAM, disco e latencia.
	- Polling de `1Hz` com `glib::spawn_future_local` + `gio::spawn_blocking`.
- Layout principal com `AdwNavigationSplitView`.
- Views separadas para Dashboard, Profiles, Tuning, Logs e Settings.
- `ToastOverlay` para feedback nao modal.
- `AlertDialog` para confirmacoes e erros.
- Integracao com system tray via `ksni`.
- `hide on close` funcional.
- Animacoes CSS e badges de status.

### Parcial
- Tratamento de falhas de servico existe, mas a UX ainda depende de dialog/modal e botao de alerta.
- A acao de reparo ainda dispara comando shell privilegiado, nao um fluxo totalmente encapsulado pelo daemon.

### Nao implementado
- `adw::StatusPage` para empty states/erros guiados.
- `adw::Banner` para mensagens contextuais menos agressivas.

## 2. Arquitetura Rust / Backend

### Implementado
- `bigame-core`, `bigame-daemon` e `bigame-ui` estao separados em workspace Cargo.
- Escrita de configuracao global via daemon root em D-Bus.
- Escrita e remocao de perfis via daemon root em D-Bus.
- Toggle de power profile via D-Bus.
- Servico de status do falcond reexposto em D-Bus de sessao.

### Parcial
- O review anterior dizia que o despacho era totalmente por D-Bus e sem shell. Isso nao e verdade.
- Ainda existem comandos externos relevantes:
	- reparo via `pkexec sh -c ...` na UI;
	- `sudo pkill -HUP falcond` ao deletar perfil;
	- `pgrep`, `grep`, `timeout`, `ping`, `dmesg`, `lspci` e outros para diagnostico/telemetria.
- O tratamento de erro continua baseado em `anyhow`; nao existe uma camada estruturada do tipo `BiGameError`/`thiserror` para separar erro tecnico de mensagem amigavel.

### Nao implementado
- Normalizacao de erros com `thiserror` + mensagens de UX amigaveis.

## 3. Funcionalidades

### Implementado
- Telemetria em tempo real com sparklines.
- Deteccao de jogos instalados em:
	- Steam
	- Lutris
	- Heroic
- Gestao de perfis por jogo.
- Integracao com `gamescope`, `sched-ext`, `vcache` e `lsfg-vk` em diferentes niveis da UI/core.

### Parcial
- O projeto detecta jogos instalados e mostra status do falcond, mas isso nao equivale a auto-start por processo/launcher.

### Nao implementado
- Auto-start por jogo baseado em processo/`bwrap`/launcher.
- Sincronia na nuvem com BigLinux ID.

## 4. Estrutura, Empacotamento e i18n

### Implementado
- `PKGBUILD` funcional com:
	- build do workspace Rust;
	- `cargo test --release` em `check()`;
	- instalacao de binarios, desktop file, policy, servicos, icones e licenca;
	- compilacao de `.po` para `.mo`.
- `gettext-rs` presente na UI.
- Arquivos `.po` existentes para muitos idiomas.

### Corrigido nesta revisao
- `locale/POTFILES.in` estava apontando para caminhos antigos (`crates/...`).
- `meson.build` da raiz estava desalinhado com o layout atual do repo.

### Ainda pendente de validacao externa
- O fluxo Meson nao foi executado nesta auditoria porque `meson` nao esta instalado no ambiente.
- `cargo clippy -D warnings` continua falhando por warnings preexistentes no codigo Rust.

### Nao implementado
- CI com GitHub Actions (`.github/workflows/`).

## 5. O que realmente falta terminar

Prioridade alta:
1. Substituir caminhos shell privilegiados remanescentes por chamadas D-Bus/coordenacao no daemon.
2. Implementar camada estruturada de erros para nao vazar mensagens tecnicas cruas ao usuario.
3. Adicionar CI automatizado para build/test/clippy.

Prioridade media:
1. Implementar `StatusPage` para servico offline/permissao ausente.
2. Implementar `Banner` para mensagens contextuais.
3. Validar o fluxo Meson apos instalar `meson` no ambiente.

Prioridade baixa / futura:
1. Auto-start por jogo/processo.
2. Cloud sync / BigLinux ID.

## Conclusao

O projeto nao esta "todo implementado" em relacao ao review original. A base principal ja existe e esta funcional, mas ainda faltam terminar principalmente:
- UX de erro mais madura;
- eliminacao de shell privilegiado remanescente;
- tratamento estruturado de erros;
- CI;
- automacao de ativacao por jogo.

Em outras palavras: o aplicativo esta utilizavel e compila/testa bem, mas a documentacao anterior superestimava o nivel de acabamento da arquitetura e do empacotamento.
# BiGameMode - Revisão e Sugestões de Melhoria (UX/UI, Código, Funcionalidades)

Durante a exploração do código da interface em GTK4 (`bigame-ui`) e o núcleo de orquestração (`bigame-core`), montei o seguinte relatório técnico sugerindo melhorias substanciais ao aplicativo em múltiplos eixos:

## 1. UX/UI e Design (Libadwaita/GTK4)
- **Estados Vazios (Empty States):** Como o BiGameMode roda em `systemctl` (via o daemon `falcond`), se ele detectar que o serviço ou permissões do PolicyKit falharam, a UI atual exibe erros brutos. Melhoraria imensamente a UX ao adotar o componente `adw::StatusPage` para guiar a correção interativa da falta de permissão ou falta de pacotes.
- **Microinterações:** Durante a ativação de um modo denso de performance, usar botões que exibem "loading" temporários ajudaria em respostas mais fluídas se a comunicação D-Bus ou backend demorasse 300-500ms.
- **Feedback Constante (Branding):** Em relatórios de "Sistema Não Baseado em BigLinux/TigerOS", seria viável utilizar `adw::Banner` (toast messages dinâmicos em vez de Dialogs agressivos modais).
- **Ícones Simbólicos Contextuais:** Refinar os `.ui` (ou layouts programáticos) para usar classes `.suggested-action` e `.destructive-action` em todos os perfis predefinidos que aplicam escalonamento alto de CPU.

## 2. Refatoração de Código e Arquitetura (Rust)
- **Desacoplamento Assíncrono (async/await):** Dentro do `bigame-ui`, operações do GTK não devem ser bloqueadas. Identifiquei algumas áreas que esperam respostas síncronas bloqueando Main Thread. Utilizar o `glib::MainContext::spawn_local` junto das primitivas de `tokio` melhorará a responsividade quando o DBUS do `falcond` é acionado para aplicar os perfis (especialmente se I/O pesado ou systemd estão envolvidos na resposta).
- **Despacho Total via DBus:** Ao invés do binário gráfico subornar o shell com `sudo -n /usr/bin/mkdir` na hora de salvar propriedades, a interface `bigame-ui` deveria enviar um objeto JSON pelo D-Bus para o `falcond` (Que roda como root pelo systemctl, correto?). Isso evita depender de re-regras engessadas em sudoers e delega inteiramente a operação restrita ao servidor de orquestração.
- **Tratamento de Erros:** Vários erros do sistema reportam strings com `anyhow::Result` formatados até a tela do usuário. Criar uma tratativa própria `enum BiGameError` associada ao `thiserror` no `bigame-core` separaria as entranhas logadas do que é amigavelmente injetado no `adw::Toast` para o usuário na interface GTK.

## 3. Funcionalidades Potenciais
- **Gráficos de Tempo-Real / Benchmark:** Injetar uma view lateral interativa exibindo estatísticas ao vivo do sistema (uso de CPU, GPU, Temperaturas). O usuário saberá ativamente que o modo Gamer realmente aplicou melhorias em tempo real!
- **Auto-Start por Jogo (Perfis por Processos):** Criar integração entre a checagem de execução e a captura de chamadas `bwrap` (Steam, Lutris, Heroic), detectando automaticamente a liberação de um jogo e aplicando o perfil Gamer no modo automático.
- **Sincronia na Nuvem com BigLinux ID:** Integração para o usuário manter backups dos seus perfis salvos, recuperando suas customizações perfeitamente ao reinstalar o sistema.

## 4. Estrutura e Empacotamento
- **CI de Automação:** Criar uma branch de GitHub Actions `.github/workflows/rust.yml` que valide a syntax através do `cargo clippy --pedantic` e efetue binds em pacotes para verificar brechas e code debt a cada PR.
