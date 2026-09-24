# 🎮 BiGame-mode

**Modo de jogo para o BigLinux: Turbo, perfis por jogo, Gráficos com IA,
benchmarks e diagnóstico, numa interface GTK4/libadwaita.**

- **Autor:** Rafael Ruscher — <rruscher@gmail.com>
- **Licença:** GPL-3.0-or-later
- **Repositório:** <https://github.com/ruscher/bigamemode>

## O que é

O BiGame-mode é a central de jogos do BigLinux. Com um botão — o **Turbo** — os
jogos passam a rodar com o perfil de desempenho certo, aplicado e desfeito
automaticamente pelo [falcond](https://git.pika-os.com/general-packages/falcond).
O BiGame-mode mostra o que de fato está em vigor, mede se uma mudança ajudou e
cuida do que acontece *dentro* do jogo — upscaling e geração de quadros — com
backup e desfazer completos.

Uma regra atravessa o projeto: **nada é oferecido que a máquina não possa
fazer, e nada é chamado de melhoria sem medição.**

## Principais recursos

| Página | O que faz |
|---|---|
| **Início** | O **Turbo**, a chave principal. Desligado, o BiGame-mode não interfere em jogo nenhum; ligado, o falcond aplica o perfil de cada jogo. Mostra o jogo em execução, o perfil ativo e o estado dos Gráficos com IA. |
| **Detalhes** | Telemetria em tempo real: frequência, temperatura e uso de CPU e GPU, memória, disco, perfil de energia, escalonador e latência de rede. |
| **Perfis** | Jogos do Steam, Lutris, Heroic e do menu de aplicativos (jogos nativos, como o SuperTuxKart do pacman), cada um com seu perfil: modo de desempenho, escalonador sched-ext, modo do 3D V-Cache, inibição de repouso, Gamescope, MangoHud e lsfg-vk. Um **assistente** explica cada opção em linguagem simples, e quando um jogo desconhecido abre com o Turbo ligado, uma notificação oferece criar o perfil. |
| **Gráficos com IA** | No menu ⋮ de cada jogo: analisa o jogo (API gráfica, DLSS/XeSS/FSR que ele já traz, DLLs de proxy, anti-cheat), recomenda um plano e, só quando você clica em **Aplicar**, instala o OptiScaler com backup verificado. **Reparar** e **Restaurar os gráficos do jogo** devolvem cada arquivo original. |
| **Ajustes** | Configuração global do falcond e as opções do Gamescope detectadas da versão instalada. |
| **Vídeo** | Upscaling espacial padrão (Gamescope FSR/NIS, Wine FSR, vkBasalt) e geração de quadros com lsfg-vk. |
| **Benchmark** | Quais medições são possíveis nesta máquina e o que já foi medido. Para jogos que abrem diretamente, **Medir a diferença** (no menu do jogo) compara com e sem otimizações, em várias execuções alternadas. |
| **Diagnóstico** | Saúde do sistema com a correção de cada problema, um relatório para suporte e medições de rede. Somente leitura. |
| **Registros** | Tudo o que importa numa sessão de jogo, do journal: falcond, BiGame-mode, power-profiles-daemon, scx_loader, Gamescope e os drivers de GPU. |
| **Configurações** | O que o BiGame-mode faz sozinho, e **Devolver**, que entrega o falcond exatamente como estava antes. |

Fechar a janela deixa o aplicativo na **bandeja** (azul: ocioso, verde: jogo
otimizado, amarelo: aviso).

## Como funciona

- **O falcond cuida do desempenho do sistema.** Ele reconhece o jogo pelo nome
  do processo e aplica o perfil — perfil de energia, escalonador sched-ext,
  modo do V-Cache, inibição de repouso —, restaurando tudo quando o jogo fecha.
  O BiGame-mode liga e desliga o falcond (Turbo) e escreve os perfis que ele lê.
- **Cada ajuste tem um único dono.** O BiGame-mode não aplica por conta própria
  o que o falcond ou o power-profiles-daemon já aplicam, e o GameMode da Feral
  não é usado (os dois disputariam os mesmos ajustes).
- **Duas tecnologias com a mesma função não rodam em série.** Num jogo com
  OptiScaler, o Wine FSR e o upscaling do Gamescope ficam desligados naquela
  execução, e o lsfg-vk também quando o OptiScaler gera os quadros.
- **Gráficos com IA** detectam o que o jogo realmente usa (pela tabela de
  importação do executável, não pelo nome de DLLs), nunca tocam jogos com
  anti-cheat e baixam o [OptiScaler](https://github.com/optiscaler/OptiScaler)
  da release oficial, por HTTPS e com SHA-256 fixado. Cada instalação é uma
  transação — backup verificado, diário, troca atômica, verificação — e uma
  instalação interrompida é desfeita na próxima abertura. O BiGame-mode não
  redistribui binários de terceiros nem baixa ou substitui DLLs da NVIDIA.

## Instalação

### BigLinux / Manjaro / Arch Linux

**1. Ative o repositório BigCommunity (community-extra).** O `falcond` e o
`lsfg-vk` vêm dele, e ele não vem ativado numa instalação padrão do BigLinux.

```bash
sudo pacman-key --keyserver hkps://keyserver.ubuntu.com \
    --recv-keys AECEEE84E52BBFAA9F1C9DF01EA0CEEEB09B44A3
sudo pacman-key --lsign-key AECEEE84E52BBFAA9F1C9DF01EA0CEEEB09B44A3

sudo tee -a /etc/pacman.conf <<'CONF'

[community-extra]
SigLevel = PackageRequired
Server = https://repo.communitybig.org/extra/$arch
CONF

sudo pacman -Sy
```

**2. Compile e instale o BiGame-mode.**

```bash
sudo pacman -S --needed base-devel git
git clone https://github.com/ruscher/bigamemode.git
cd bigamemode
makepkg -si
```

O `makepkg` instala o que falta para compilar, compila, confere as traduções,
roda os testes e instala o pacote.

**3. Recomendado:** os escalonadores sched-ext, para o falcond poder trocar o
escalonador de CPU durante o jogo.

```bash
sudo pacman -S scx-tools scx-scheds
```

Depois, abra **BiGame-mode** no menu de aplicativos.

Ao atualizar, o helper é reiniciado. Ao remover, o falcond volta ao estado em
que estava antes do BiGame-mode. Arquivos que os Gráficos com IA colocaram em
jogos continuam lá até **Restaurar os gráficos do jogo** (os backups ficam em
`~/.local/state/bigame-mode/graphics`).

## Dependências

| Pacote | Por quê |
|---|---|
| `gtk4`, `libadwaita`, `glib2`, `hicolor-icon-theme` | interface |
| `dbus`, `polkit`, `systemd` | o helper root: serviço no barramento de sistema, iniciado pelo systemd, cada ação autorizada pelo Polkit |
| `falcond`, `power-profiles-daemon` | desempenho por jogo e perfil de energia |
| `curl`, `libarchive` | baixar e extrair o OptiScaler |
| `hwdata`, `pciutils` | identificar a placa de vídeo |
| `iputils`, `iproute2` | latência e fila de rede |

Opcionais: `scx-tools` e `scx-scheds`, `gamescope`, `mangohud` (também captura
os frametimes das medições), `lsfg-vk` (com o seu `Lossless.dll` do Lossless
Scaling), `vkbasalt`, `steam`, `lutris`, `heroic-games-launcher`,
`nvidia-utils` (telemetria em placas NVIDIA) e `supertuxkart`.

O pacote instala `bigame-ui` (o aplicativo, como usuário comum), `bigame-daemon`
(o helper root) com sua unit do systemd, arquivos de D-Bus e política do
Polkit, o `.desktop`, o metainfo, os ícones e as traduções.

## Arquitetura resumida

```text
┌─────────────────────────┐   D-Bus (sistema)   ┌─────────────────────────┐
│ bigame-ui    (usuário)  │ ──────────────────▶ │ bigame-daemon   (root)  │
│ GTK4 + libadwaita, tray │  cada chamada passa │ valida cada argumento;  │
│ Gráficos com IA,        │  pelo Polkit        │ perfis e config do      │
│ medições, registros     │                     │ falcond, Turbo, sysfs   │
└───────────┬─────────────┘                     └───────────┬─────────────┘
            │ lê o estado                                   ▼
            └──────────────────────────────▶ falcond ──▶ scx_loader,
                                             (perfis)    power-profiles-daemon
```

- A interface nunca roda como root. O helper tem nove métodos privilegiados,
  cada um autorizado pelo Polkit e validado do lado root, e roda confinado
  pelo systemd. Nenhum comando passa por shell.
- O código é um workspace Rust em `bigame-engine/`: `bigame-core` (toda a
  lógica, sem interface), `bigame-daemon` e `bigame-ui`.

Detalhes em [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) e
[docs/SECURITY.md](docs/SECURITY.md).

## Compatibilidade

- **Sistema:** BigLinux e derivados do Manjaro/Arch, com systemd e o falcond do
  repositório BigCommunity.
- **Área de trabalho:** testado no KDE Plasma (Wayland). No GNOME, o ícone da
  bandeja depende de uma extensão AppIndicator.
- **Jogos:** Steam (incluindo Proton), Lutris, Heroic e jogos nativos do menu de
  aplicativos.
- **Testado em:** AMD Ryzen 7 5700G com Radeon RX 9060 XT (RDNA 4), e numa
  máquina virtual com BigLinux padrão (instalação do zero, sem aceleração 3D).
- **Detectado, mas não testado em hardware real:** GPUs NVIDIA e Intel, sistemas
  híbridos (Intel/AMD + NVIDIA), CPUs híbridas, 3D V-Cache, notebooks na
  bateria, VRR e HDR. Nessas máquinas o BiGame-mode oferece só o que detectar
  como suportado.

## Benchmarks

Medido com o benchmark do próprio Shadow of the Tomb Raider (3440×1440, três
execuções alternadas por configuração, diferença exigida acima da variação e
no teste t de Welch a 95 %):

| Configuração | FPS médio |
|---|---|
| TAA nativo do jogo | 89,8 |
| XeSS Quality do próprio jogo | 94,2 (+4,9 %) |
| **FSR via OptiScaler (Gráficos com IA)** | **98,8 (+10,1 %)** |

Na mesma máquina, fixar a GPU no nível de energia `high` deixou os jogos
7,5–8,3 % **mais lentos**, e perfil de energia, governador e escalonadores
sched-ext não mudaram nada — por isso o BiGame-mode não os força. Método, todos
os resultados e os dados brutos: [docs/BENCHMARKS.md](docs/BENCHMARKS.md).

## Desenvolvimento

Requer Rust 1.85 ou mais novo, GTK 4.14+, libadwaita 1.6+ e
`glib-compile-resources`.

```bash
cd bigame-engine
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets     # sem avisos (lints pedantic)
./target/debug/bigame-ui
./target/debug/bigame-ui --diagnostics     # relatório de suporte no terminal
```

- Sem o pacote instalado, a interface roda, mas o que exige root (perfis,
  Turbo, configuração do falcond) fica indisponível.
- `../tests/daemon-authorization.sh` confere que o helper recusa todas as
  ações privilegiadas quando o Polkit não está disponível.
- `bigame-core/examples/` traz ferramentas de linha de comando: detecção
  (`detect`, `library`, `running`, `health`), Gráficos com IA (`graphics_scan`,
  `graphics_plan`, `graphics_apply`, `graphics_status`, `optiscaler_fetch`,
  `pe_dump`), Turbo e Booster (`turbo`, `booster_run`, `measure`) e relatórios
  de benchmark (`bench_native_report`, `bench_report`).
  Rode com `cargo run -p bigame-core --example <nome>`.
- `bigame-engine/scripts/` automatiza sessões de benchmark (`bench-game.sh`,
  `bench-lab.sh`); os dados publicados ficam em `bigame-engine/benchmarks/`.

**Traduções:** os catálogos ficam em `locale/*.po`. Depois de mudar textos no
código:

```bash
python3 locale/extract-strings.py        # atualiza locale/bigame-mode.pot
msgmerge -U --no-wrap locale/pt_BR.po locale/bigame-mode.pot
```

O build falha se o template estiver desatualizado.

## Autor

**Rafael Ruscher** — <rruscher@gmail.com>

Eu, **Rafael Ruscher**, sempre fui apaixonado por jogos. Sou um grande entusiasta e, principalmente, um defensor ferrenho de jogos no Linux. Nos últimos anos, vimos o jogo virar: com as melhorias constantes e o apoio massivo da **Valve**, a compatibilidade hoje é quase total.

Fico extremamente feliz em poder jogar com amigos como o **Barnabé di Kartola**, e acompanhar a turma do **Alessandro** e do **Pacheco** do canal **System Infotech**. Eles jogam diariamente e, sempre que me sobra um tempinho, estou lá jogando com eles. Ver canais mostrando o **BigLinux** em ação me motiva profundamente.

Em respeito a essa comunidade e para garantir que todos tenham a melhor experiência possível, criei o **BiGame-mode**. O objetivo é aproveitar o máximo do hardware, trazendo os últimos recursos tecnológicos para alcançar o FPS máximo. Com a integração do `lsfg-vk` (Lossless Scaling) e o `falcond`, criamos uma solução completa de GameMode para o ecossistema BigLinux.

**Agradecimentos:** Bruno Gonçalves, Barnabé di Kartola, Alessandro e Pacheco
(System Infotech) e a comunidade BigLinux.

## Projetos utilizados

O BiGame-mode se apoia em projetos de terceiros, cada um com seus autores e
licenças:

- **Sistema:** [falcond](https://git.pika-os.com/general-packages/falcond)
  (PikaOS), [sched-ext](https://github.com/sched-ext/scx) e `scx_loader`,
  power-profiles-daemon, systemd, D-Bus, Polkit.
- **Jogos e gráficos:** [OptiScaler](https://github.com/optiscaler/OptiScaler),
  Gamescope e Proton (Valve), DXVK, VKD3D-Proton,
  [lsfg-vk](https://github.com/PancakeTAS/lsfg-vk), MangoHud, vkBasalt. DLSS,
  XeSS e FSR pertencem a NVIDIA, Intel e AMD e seguem as licenças delas.
- **Aplicativo:** Rust, GTK e libadwaita (GNOME), gtk4-rs, zbus, Tokio, Serde,
  ksni, gettext.

## Licença

GPL-3.0-or-later. Veja [LICENSE](LICENSE).
