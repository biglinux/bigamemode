# 🎮 BiGame-mode

**Modo de jogo para o BigLinux: perfis por jogo, Turbo, Gráficos com IA, benchmarks e diagnóstico — numa interface GTK4/libadwaita.**

BiGame-mode é a central de jogos do BigLinux. Ele não é mais um daemon de
performance: quem aplica as otimizações de sistema durante um jogo é o
[falcond](https://git.pika-os.com/general-packages/falcond). O BiGame-mode liga e
desliga o falcond (Turbo), escreve os perfis por jogo que ele lê, mostra o que
de fato está acontecendo, mede se uma mudança ajudou e cuida do que acontece
*dentro* do jogo — upscaling e geração de quadros — com backup e desfazer
completos.

Uma regra atravessa o projeto inteiro: **nada é oferecido que a máquina não
possa fazer, e nada é chamado de melhoria sem medição.**

- **Autor:** Rafael Ruscher — <rruscher@gmail.com>
- **Licença:** GPL-3.0-or-later
- **Repositório:** <https://github.com/ruscher/bigamemode>

---

## 📖 A História por Trás do Projeto

Eu, **Rafael Ruscher**, sempre fui apaixonado por jogos. Sou um grande entusiasta e, principalmente, um defensor ferrenho de jogos no Linux. Nos últimos anos, vimos o jogo virar: com as melhorias constantes e o apoio massivo da **Valve**, a compatibilidade hoje é quase total.

Fico extremamente feliz em poder jogar com amigos como o **Barnabé di Kartola**, e acompanhar a turma do **Alessandro** e do **Pacheco** do canal **System Infotech**. Eles jogam diariamente e, sempre que me sobra um tempinho, estou lá jogando com eles. Ver canais mostrando o **BigLinux** em ação me motiva profundamente.

Em respeito a essa comunidade e para garantir que todos tenham a melhor experiência possível, criei o **BiGame-mode**. O objetivo é aproveitar o máximo do hardware, trazendo os últimos recursos tecnológicos para alcançar o FPS máximo. Com a integração do `lsfg-vk` (Lossless Scaling) e o `falcond`, criamos uma solução completa de GameMode para o ecossistema BigLinux.

---

## 🚀 Funcionalidades

| Página | O que faz |
|---|---|
| **Início** | O **Turbo**, a chave principal. Desligado, o BiGame-mode não interfere em jogo nenhum; ligado, o falcond é habilitado e iniciado (confirmado pelo systemd e pelo próprio estado do falcond) e aplica o perfil de cada jogo. Mostra o jogo em execução, o perfil ativo e o estado dos Gráficos com IA. |
| **Detalhes** | Telemetria em tempo real: frequência, temperatura e uso de CPU e GPU, perfil de energia, escalonador sched-ext ativo e latência de rede. |
| **Perfis** | Jogos encontrados no Steam, Lutris e Heroic, cada um com seu perfil do falcond (modo de desempenho, escalonador sched-ext, modo do 3D V-Cache, inibição de repouso, scripts) e as opções do BiGame-mode (Gamescope, MangoHud, lsfg-vk). Inclui um **assistente passo a passo** que explica cada opção em linguagem simples. Quando um jogo desconhecido abre com o Turbo ligado, uma notificação oferece criar o perfil. |
| **Gráficos com IA** | No menu ⋮ de cada jogo: analisa os arquivos do jogo (API gráfica, upscalers que ele já traz — DLSS, XeSS, FSR — e suas versões, DLLs de proxy, anti-cheat), recomenda um plano e, só quando você clica em **Aplicar**, instala o OptiScaler com backup verificado. **Reparar** e **Restaurar os gráficos do jogo** devolvem cada arquivo original. |
| **Ajustes** | Configuração global do falcond: escalonador padrão, V-Cache, intervalo de varredura, opções do Gamescope detectadas da versão instalada. |
| **Vídeo** | Upscaling espacial padrão (Gamescope FSR, Wine FSR, vkBasalt) e geração de quadros com lsfg-vk, quando instalado. |
| **Benchmark** | Laboratório de medição: benchmarks embutidos de jogos (Shadow of the Tomb Raider, Rise of the Tomb Raider, Tomb Raider 2013, Cyberpunk 2077), SuperTuxKart e Unigine Superposition. Compara configurações A/B com várias execuções e só declara diferença quando ela supera a variação entre execuções e passa no teste t de Welch a 95 %. |
| **Diagnóstico** | Um relatório para suporte (hardware, drivers, serviços, perfis, jogos alterados pelos Gráficos com IA) e medições de rede. Somente leitura. |
| **Registros** | Tudo o que importa numa sessão de jogo, lido do journal numa chamada só: falcond, o helper, a interface, power-profiles-daemon, scx_loader, Polkit, Gamescope e os drivers de GPU. |
| **Configurações** | O que o BiGame-mode faz sozinho, e o botão **Devolver** que entrega o falcond de volta exatamente como estava antes. |

Também há um **ícone na bandeja** (StatusNotifierItem): fechar a janela deixa
o aplicativo em segundo plano, com a cor do ícone indicando o estado (azul:
ocioso, verde: ativo, amarelo: aviso, vermelho: erro).

### Gráficos com IA (OptiScaler)

- Detecta a API pelo que o executável importa (tabela de importação PE), não
  pelo nome de DLLs; cada conclusão vem com seu grau de certeza.
- Nunca instala nada em jogos com anti-cheat e nunca empilha dois upscalers ou
  dois geradores de quadros.
- Baixa o [OptiScaler](https://github.com/optiscaler/OptiScaler) (GPL-3.0) da
  release oficial, só por HTTPS, com SHA-256 fixado, e confere a listagem do
  arquivo antes de extrair. O BiGame-mode não redistribui binário gráfico de
  terceiros, nem baixa, instala ou substitui DLLs da NVIDIA.
- Cada instalação é uma transação: backup verificado → diário → troca atômica
  → verificação → registro (manifesto com hashes). Uma instalação interrompida
  é desfeita na próxima abertura. Remover segue o manifesto, arquivo por
  arquivo, pelo hash.
- Não precisa de root: pastas de jogo, cache e backups são do usuário.
- Medido em Shadow of the Tomb Raider (RX 9060 XT, 3440×1440): TAA 89,8 fps →
  XeSS do jogo 94,2 fps (+4,9 %) → FSR via OptiScaler 98,8 fps (+10,1 %), com
  1 % low inalterado ([docs/31](docs/31-DLSS-BENCHMARKS.md)).

---

## 📦 Instalação

### BigLinux / Manjaro / Arch Linux (recomendado)

**1. Ative o repositório BigCommunity (community-extra).** O `falcond` e o
`lsfg-vk` vêm dele, e ele não vem ativado numa instalação padrão do BigLinux.
Sem ele, a instalação para com *"falcond: alvo não encontrado"*.

```bash
# Chave que assina os pacotes do repositório
sudo pacman-key --keyserver hkps://keyserver.ubuntu.com \
    --recv-keys AECEEE84E52BBFAA9F1C9DF01EA0CEEEB09B44A3
sudo pacman-key --lsign-key AECEEE84E52BBFAA9F1C9DF01EA0CEEEB09B44A3

# Repositório, no fim do /etc/pacman.conf
sudo tee -a /etc/pacman.conf <<'CONF'

[community-extra]
SigLevel = PackageRequired
Server = https://repo.communitybig.org/extra/$arch
CONF

sudo pacman -Sy
```

**2. Ferramentas de compilação e o próprio BiGame-mode.**

```bash
sudo pacman -S --needed base-devel git
git clone https://github.com/ruscher/bigamemode.git
cd bigamemode
makepkg -si
```

O `makepkg` instala o que falta para compilar (Rust, gettext…), baixa o
código do GitHub, compila o workspace Rust em modo release, verifica o
catálogo de traduções, roda os testes e instala tudo.

**3. Opcional, mas recomendado:** os escalonadores sched-ext, para o falcond
trocar o escalonador de CPU durante o jogo:

```bash
sudo pacman -S scx-tools scx-scheds
```

Depois, abra **BiGame-mode** no menu de aplicativos. O helper privilegiado
(`bigame-daemon`) sobe sozinho pelo D-Bus na primeira vez que for necessário.

#### Dependências

| Pacote | Por quê |
|---|---|
| `gtk4`, `libadwaita`, `glib2` | interface gráfica |
| `dbus`, `polkit`, `systemd` | o helper root: serviço no barramento de sistema, cada método autorizado pelo Polkit, iniciado pelo systemd |
| `falcond` | aplica os perfis de jogo (CPU, escalonador, V-Cache, perfil de energia) |
| `power-profiles-daemon` | perfil de energia do sistema |
| `curl`, `libarchive` | baixar e extrair o OptiScaler (Gráficos com IA) |
| `hwdata`, `pciutils` | nome e fabricante da placa de vídeo |
| `iputils`, `iproute2` | latência de rede e fila da interface (Diagnóstico) |

Opcionais: `scx-tools` e `scx-scheds` (escalonadores sched-ext), `gamescope`,
`mangohud` (também usado para capturar frametimes nos benchmarks), `lsfg-vk`
(requer o seu próprio `Lossless.dll` do Lossless Scaling), `vkbasalt`,
`steam`, `lutris`, `heroic-games-launcher`, `nvidia-utils` (telemetria em
placas NVIDIA) e `supertuxkart` (benchmark nativo).

O BiGame-mode **não** usa o GameMode da Feral: ele e o falcond disputariam os
mesmos ajustes. Se o GameMode estiver instalado, o BiGame-mode aponta o
conflito, sem removê-lo.

#### O que o pacote instala

```text
/usr/bin/bigame-ui                     aplicativo (usuário comum)
/usr/bin/bigame-daemon                 helper privilegiado (root, via D-Bus)
/usr/bin/falcond-diag                  script de diagnóstico do falcond
/usr/lib/systemd/system/bigame-daemon.service
/usr/share/dbus-1/system.d/com.biglinux.BiGameMode.conf
/usr/share/dbus-1/system-services/com.biglinux.BiGameMode.service
/usr/share/polkit-1/actions/com.biglinux.BiGameMode.policy
/usr/share/applications/com.biglinux.BiGameMode.desktop
/usr/share/metainfo/com.biglinux.BiGameMode.metainfo.xml
/usr/share/icons/hicolor/scalable/apps/*.svg
/usr/share/locale/*/LC_MESSAGES/bigame-mode.mo
```

Ao atualizar, o helper é reiniciado. Ao remover, o falcond volta ao estado
em que estava antes do BiGame-mode assumi-lo. Arquivos que os Gráficos com
IA colocaram em jogos continuam lá: use **Restaurar os gráficos do jogo** antes
de desinstalar (os backups em `~/.local/state/bigame-mode/graphics` são
mantidos).

### A partir do código (desenvolvimento)

Requer Rust 1.85 ou mais novo (edição 2024), GTK 4.14+, libadwaita 1.6+ e
`glib-compile-resources` (glib2).

```bash
cd bigame-engine
cargo build --workspace          # debug
cargo test --workspace           # 500+ testes
cargo clippy --workspace --all-targets
./target/debug/bigame-ui
```

A interface roda sem o helper instalado, mas tudo que exige root (salvar
perfis, Turbo, configuração do falcond) fica indisponível até que o
`bigame-daemon` e seus arquivos de D-Bus, Polkit e systemd estejam instalados
— o caminho suportado para isso é o PKGBUILD.

---

## 🛠️ Arquitetura

```text
┌──────────────────────────┐   D-Bus (sistema)    ┌────────────────────────┐
│ bigame-ui  (usuário)     │ ───────────────────▶ │ bigame-daemon  (root)  │
│ GTK4 + libadwaita + tray │  cada método passa   │ valida cada argumento, │
│ Gráficos com IA (sem     │  pelo Polkit         │ escreve perfis e config│
│ root), benchmarks, logs  │                      │ do falcond, liga/deslig│
└────────────┬─────────────┘                      │ a o falcond (systemd)  │
             │ lê estado                          └───────────┬────────────┘
             ▼                                                ▼
   /var/lib/falcond/status  ◀──────────────  falcond ──▶ scx_loader, power-
   journal, sysfs, /proc                    (perfis)     profiles-daemon, sysfs
```

- **A interface nunca roda como root.** Tudo que precisa de privilégio passa
  por um helper pequeno com nove métodos privilegiados no D-Bus, cada um
  autorizado pelo Polkit e com os argumentos validados do lado do servidor.
- **O helper é confinado pelo systemd** (`ProtectSystem=strict`,
  `ProtectHome`, filtro de syscalls, só `AF_UNIX`), com escrita apenas em
  `/etc/falcond`, `/usr/share/falcond/profiles`, cpufreq/amdgpu no sysfs e
  `/var/lib/bigame-mode`.
- **Nenhum comando passa por shell.** Programas externos são chamados com
  vetor de argumentos; o systemd é controlado pela API D-Bus dele.
- **Cada componente é dono do que é seu.** O falcond cuida do desempenho do
  sistema; os Gráficos com IA cuidam dos arquivos do jogo; a Harmony Policy
  impede que duas tecnologias façam o mesmo trabalho em série (por exemplo,
  desliga o Wine FSR e a resolução do Gamescope num jogo com OptiScaler).

### Onde ficam os dados

| Caminho | Conteúdo |
|---|---|
| `/usr/share/falcond/profiles/user/<processo>.conf` | perfis de jogo do falcond (via helper) |
| `/etc/falcond/config.conf` | configuração global do falcond (via helper) |
| `/var/lib/bigame-mode/game-backend.json` | como o falcond estava antes de o BiGame-mode assumi-lo |
| `~/.config/bigame-mode/` | `settings.toml`, `video.toml`, `games/<processo>.toml` (opções por jogo) |
| `~/.cache/bigame-mode/graphics/optiscaler/<versão>/` | release do OptiScaler baixada e verificada |
| `~/.local/state/bigame-mode/graphics/<appid>/` | manifestos e backups dos Gráficos com IA |

---

## 🗂️ Estrutura do repositório

```text
bigamemode/
├── PKGBUILD, bigame-mode.install   empacotamento (Arch/BigLinux)
├── bigame-engine/                  workspace Rust
│   ├── bigame-core/                lógica sem interface: falcond, perfis, Turbo,
│   │   ├── src/graphics/           Gráficos com IA (pe, scan, rules, plan,
│   │   │                           optiscaler, manifest, transaction, runtime…)
│   │   ├── src/benchmark/          captura de frametimes, A/B, provedores
│   │   ├── src/booster/            plano → aplicar → verificar → restaurar
│   │   └── examples/               ferramentas de linha de comando (detect,
│   │                               graphics_scan, graphics_plan, bench_ab…)
│   ├── bigame-daemon/              helper root: main, polkit, validate, backend
│   ├── bigame-ui/                  aplicativo GTK4/libadwaita
│   │   ├── src/views/              páginas (home, profiles, ai_graphics…)
│   │   └── src/widgets/            componentes reutilizáveis
│   ├── benchmarks/                 dados brutos das medições publicadas
│   └── scripts/                    automação de benchmark (bench-game.sh…)
├── data/                           unit systemd, D-Bus, Polkit, .desktop,
│                                   metainfo, gresource
├── locale/                         traduções (.po, 29 idiomas), template .pot
│                                   e o extrator extract-strings.py
├── style/style.css                 tema, embutido no binário via gresource
├── usr/                            ícones, falcond-diag
├── tests/daemon-authorization.sh   teste de integração: o helper recusa sem Polkit
└── docs/                           auditorias, arquitetura, benchmarks e relatórios
```

`meson.build` e `com.biglinux.BiGameMode.json` (Flatpak) são restos de uma
versão anterior e não geram uma instalação funcional: o Meson não compila o
helper, e um Flatpak não pode instalar um serviço root com Polkit e systemd.

---

## 🧰 Tecnologias

### Linguagem e interface

- **Rust (edição 2024)** — todo o código. Workspace com três crates: `bigame-core`
  (biblioteca testável, sem interface), `bigame-daemon` e `bigame-ui`.
- **GTK 4 + libadwaita** (`gtk4-rs` 0.9, `libadwaita-rs` 0.7) — a interface,
  com navegação lateral `AdwNavigationSplitView`, diálogos adaptativos e o
  esquema de cores do desktop.
- **GResource** — CSS e ícones são compilados para dentro do binário
  (`glib-compile-resources` no `build.rs`).
- **ksni** — ícone na bandeja pelo protocolo StatusNotifierItem (KDE e
  outros desktops compatíveis).
- **Tokio** — runtime assíncrono do helper e das tarefas em segundo plano.
- **zbus** — D-Bus puro em Rust: fala com o helper, systemd, Polkit,
  power-profiles-daemon e scx_loader sem processos intermediários.
- **Serde, TOML, JSON** — perfis, configurações, manifestos e resultados.
- **sha2** — hashes SHA-256 do download do OptiScaler e de cada arquivo
  instalado ou salvo em backup.
- **tracing** — logs estruturados, lidos depois do journal pela página Registros.
- **gettext** (`gettext-rs`) — todas as frases da interface são traduzíveis;
  `locale/extract-strings.py` gera o `.pot` (o `xgettext` não entende Rust) e
  o build falha se ele estiver desatualizado.

### Sistema

- **falcond** — daemon de desempenho em Zig do PikaOS. Detecta o jogo pelo
  nome do processo e aplica o perfil: modo de desempenho, escalonador
  sched-ext, modo do V-Cache, perfil de energia. Ele restaura tudo quando o
  jogo fecha.
- **sched-ext / scx_loader** — escalonadores de CPU em BPF (LAVD, bpfland,
  rusty…) carregados sem trocar de kernel. O falcond pede a troca pelo
  `scx_loader` via D-Bus.
- **power-profiles-daemon** — perfis de energia `performance`, `balanced`,
  `power-saver`.
- **systemd** — o helper é um serviço D-Bus ativado sob demanda; o Turbo liga
  e desliga a unit do falcond pela API do systemd.
- **Polkit** — autoriza cada ação privilegiada (Turbo, perfis, configuração do
  falcond, CPU, GPU, V-Cache).
- **sysfs / procfs** — telemetria, detecção de hardware, jogo em execução
  (a árvore de processos do Proton é seguida até o executável real).
- **AMD 3D V-Cache** — modo `cache`/`frequency` do driver `amd_x3d_vcache`,
  em processadores Ryzen X3D.
- **journal** — uma leitura `journalctl -o json` junta os logs de todos os
  componentes de uma sessão de jogo.

### Jogos e gráficos

- **Steam, Lutris, Heroic** — biblioteca detectada nos arquivos de cada um.
  As opções de lançamento do Steam são editadas com o Steam fechado, com
  backup e releitura.
- **Proton / Wine, DXVK, VKD3D-Proton** — traduzem DirectX 9–11 e 12 para
  Vulkan. O OptiScaler entra como `dxgi.dll` na pasta do jogo e o Proton o
  carrega sem `WINEDLLOVERRIDES`.
- **Upscalers** — DLSS (NVIDIA), XeSS (Intel) e FSR (AMD) renderizam em
  resolução menor e reconstroem a imagem. O **OptiScaler** intercepta o
  upscaler que o jogo usa e o troca por outro — por exemplo, XeSS → FSR em
  placas AMD RDNA 4.
- **Gamescope** — micro-compositor da Valve: resolução interna, FSR espacial,
  limite de quadros, HDR. Os argumentos são validados contra o `--help` da
  versão instalada.
- **lsfg-vk** — camada Vulkan que traz a geração de quadros do Lossless
  Scaling ao Linux. Precisa do `Lossless.dll` do usuário.
- **MangoHud** — overlay de desempenho e fonte dos frametimes nos
  benchmarks.
- **vkBasalt** — pós-processamento Vulkan (nitidez CAS).
- **Wine FSR** — upscaling espacial do próprio Wine, em jogos em tela cheia.

### Empacotamento e testes

- **makepkg / PKGBUILD** — o único método de instalação suportado.
- **cargo test** — mais de 500 testes; os de arquivos rodam em pastas
  temporárias reais, nenhum toca um jogo de verdade.
- **clippy (pedantic)** — sem avisos.
- **curl e bsdtar (libarchive)** — chamados com vetor de argumentos para
  baixar e extrair o OptiScaler.
- **hwdata** — `pci.ids`, o banco de nomes de placas PCI.

---

## 📚 Documentação

A pasta [docs/](docs/) registra as auditorias, as decisões e cada medição,
com dados brutos em `bigame-engine/benchmarks/`. Destaques:

- [01 — Auditoria](docs/01-AUDIT.md) e [10 — Segurança](docs/10-SECURITY.md)
- [14 — Auditoria do Turbo](docs/14-TURBO-AUDIT.md) e [15 — Perfis dinâmicos](docs/15-DYNAMIC-PROFILES.md)
- [20 — Resultados de benchmark](docs/20-BENCHMARK-RESULTS.md)
- [26 — Licenças (Gráficos com IA)](docs/26-DLSS-LICENSE-AUDIT.md) e [28 — Arquitetura](docs/28-DLSS-ARCHITECTURE.md)
- [32 — Relatório final dos Gráficos com IA](docs/32-DLSS-FINAL-REPORT.md)

O aplicativo não depende de nenhum arquivo de `docs/` para funcionar.

---

## 🌍 Traduções

Os catálogos ficam em `locale/*.po`. Depois de mudar frases no código:

```bash
python3 locale/extract-strings.py            # atualiza bigame-mode.pot
msgmerge -U locale/pt_BR.po locale/bigame-mode.pot
```

---

## 🤝 Créditos

- **Desenvolvedor:** Rafael Ruscher (<rruscher@gmail.com>)
- **Agradecimentos especiais:**
  - Bruno Gonçalves
  - Barnabé di Kartola
  - Alessandro (System Infotech)
  - Pacheco (System Infotech)
  - A comunidade BigLinux
- **Projetos em que o BiGame-mode se apoia:** falcond (PikaOS), sched-ext,
  OptiScaler, Gamescope e Proton (Valve), lsfg-vk, MangoHud, GTK e GNOME.

---

## 📜 Licença

GPL-3.0-or-later. Veja [LICENSE](LICENSE).

OptiScaler é GPL-3.0 e é baixado da release oficial, sob demanda; DLSS, XeSS e
FSR pertencem a NVIDIA, Intel e AMD e seguem as licenças deles
([docs/26](docs/26-DLSS-LICENSE-AUDIT.md)).
