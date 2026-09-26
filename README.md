# 🎮 BiGame-mode

**Modo de jogo para o BigLinux: Turbo, perfis por jogo, Gráficos com IA e
uma página que mostra, com evidência, o que está mesmo em vigor — numa
interface GTK4/libadwaita.**

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

| Recurso | O que faz |
|---|---|
| **Início** | O **Turbo**, a chave principal. Desligado, o BiGame-mode não interfere em jogo nenhum; ligado, o falcond aplica o perfil de cada jogo. Mostra o jogo em execução, o perfil ativo e o estado dos Gráficos com IA. |
| **Perfis** | Jogos do Steam, Lutris, Heroic e do menu de aplicativos (jogos nativos, como o SuperTuxKart do pacman, e Flatpaks), só os que estão mesmo instalados, cada um com seu perfil: modo de desempenho, escalonador sched-ext, modo do 3D V-Cache, inibição de repouso, Gamescope, MangoHud (gravado onde o lançador do jogo o lê: Steam, Heroic ou Lutris) e lsfg-vk. No menu ⋮ de cada jogo: **Iniciar (Turbo)**, **Criar com Assistente** (um perfil guiado, cada opção explicada), **Gráficos com IA**, **Medir a diferença**, **Restaurar os gráficos do jogo**. Quando um jogo desconhecido abre com o Turbo ligado, uma notificação oferece criar o perfil. |
| **Gráficos com IA** | No menu ⋮ de cada jogo: analisa o jogo (API gráfica, DLSS/XeSS/FSR que ele já traz, DLLs de proxy, anti-cheat, a GPU em que ele renderiza), recomenda um plano e, só quando você clica em **Aplicar**, instala o OptiScaler com backup verificado ou, num jogo que já traz o FSR 3.1 da AMD, escreve a única opção de execução com que o Proton o eleva ao FSR 4 — sem tocar em arquivo nenhum. Onde a lista de jogos sabe onde o jogo guarda a chave do seu upscaler (Shadow of the Tomb Raider: o XeSS no registro), **Aplicar** também a liga e **Restaurar** a devolve. **Reparar** e **Restaurar os gráficos do jogo** devolvem cada arquivo original. **Diagnosticar** diz por que algo não funciona. Renderização neural em AMD (DLSS-NR-on-AMD) é detectada e explicada, nunca baixada: a licença não permite. |
| **Ajustes** | Tudo o que é aplicado aos jogos, em grupos progressivos: desempenho do sistema (falcond: modo de desempenho, escalonador sched-ext, 3D V-Cache), exibição e Gamescope, upscaling e nitidez (Wine FSR, vkBasalt), geração de quadros (lsfg-vk), overlay e o avançado. O que a máquina não pode fazer aparece como **não suportado** ou **dependência ausente** com o comando que resolve, nunca como um controle quebrado; dois upscalers ligados ao mesmo tempo são apontados, com a saída num clique. |
| **Detalhes** | O que a máquina está fazendo pelo jogo, com a evidência. Uma visão geral (pronto para jogar, Turbo, falcond, perfil, energia, escalonador, GPU, Gamescope, upscaling, geração de quadros), telemetria em tempo real, um cartão por placa de vídeo (carga, clock, VRAM, temperatura, energia, qual renderiza o jogo), o desempenho (Turbo, falcond e o perfil que aplicou, perfil de energia, escalonador, V-Cache) e o pipeline de vídeo (Gamescope, Wine FSR, vkBasalt, geração de quadros, MangoHud, Gráficos com IA) — cada item diz se está **ativo**, **aguardando**, **configurado mas não detectado**, **desligado**, **sem dependência** ou **não suportado**, e ao abrir a linha, o que significa, a evidência e a correção. **Problemas** reúne o que precisa de atenção, classificado (corrigível, precisa de você, hardware, informação), com comandos para copiar. Rede, carga em segundo plano, opções de lançamento da Steam quebradas e o relatório para suporte ficam aqui. |
| **Registros** | Tudo o que importa numa sessão de jogo, do journal: falcond, BiGame-mode, power-profiles-daemon, scx_loader, Gamescope e os drivers de GPU. |
| **Configurações** | A aparência — tema **Padrão** ou **Gamer**, claro, escuro ou o do sistema; uma instalação nova abre em Gamer escuro —, o que o BiGame-mode faz sozinho, e **Devolver**, que entrega o falcond exatamente como estava antes. |

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
roda os testes e instala o pacote. O PKGBUILD compila a branch `main` do
GitHub, não as mudanças locais do clone.

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
| `lsfg-vk` | geração de quadros (Lossless Scaling) por jogo; só gera quadros com o seu próprio `Lossless.dll`, que nunca vem no pacote |

Opcionais: `scx-tools` e `scx-scheds`, `gamescope`, `mangohud` (também captura
os frametimes das medições), `vkbasalt`, `steam`, `lutris`,
`heroic-games-launcher`, `nvidia-utils` (telemetria em placas NVIDIA, pela
biblioteca NVML) e `supertuxkart`.

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
- **Testado em:** AMD Ryzen 7 5700G com Radeon RX 9060 XT (RDNA 4) e a Radeon
  Vega integrada; um notebook híbrido com Intel HD 630 e GeForce GTX 1050 Ti
  (driver NVIDIA 580); e uma máquina virtual com BigLinux padrão (instalação do
  zero, sem aceleração 3D).
- **Detectado, mas não testado em hardware real:** RDNA 3, RTX, Intel Arc,
  CPUs híbridas, 3D V-Cache, notebooks na bateria, X11, VRR e HDR. Nessas
  máquinas o BiGame-mode oferece só o que detectar como suportado.

## Benchmarks

Medido com o benchmark do próprio Shadow of the Tomb Raider (3440×1440, três
execuções alternadas por configuração, diferença exigida acima da variação e
no teste t de Welch a 95 %):

| Configuração | FPS médio |
|---|---|
| TAA nativo do jogo | 89,8 |
| XeSS Quality do próprio jogo | 94,2 (+4,9 %) |
| **FSR via OptiScaler (Gráficos com IA)** | **98,8 (+10,1 %)** |

Já no Cyberpunk 2077, que traz o FSR 3.1 da AMD, o FSR 4 pelo Proton (uma
opção de execução, nenhum arquivo) rendeu o mesmo que o FSR 3.1 (38,4 → 38,2,
sem diferença, duas execuções por configuração) e o OptiScaler ficou 6,4 % **mais lento** — por isso ali o
recomendado é o FSR do próprio jogo.

Na mesma máquina, fixar a GPU no nível de energia `high` deixou os jogos
7,5–8,3 % **mais lentos**, e perfil de energia, governador e escalonadores
sched-ext não mudaram nada — por isso o BiGame-mode não os força. A geração de
quadros do lsfg-vk custou 42 % dos quadros renderizados em x2 (88,9 → 51,8) e
55 % em x3, e por isso nunca é ligada sozinha. Método e todos os resultados:
[docs/BENCHMARKS.md](docs/BENCHMARKS.md); os dados brutos de cada sessão ficam
em `bigame-engine/benchmarks/`.

## Desenvolvimento

Requer Rust 1.85 ou mais novo, GTK 4.14+, libadwaita 1.7+ e
`glib-compile-resources`.

```bash
cd bigame-engine
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings   # lints pedantic
./target/debug/bigame-ui
./target/debug/bigame-ui --diagnostics     # relatório de suporte no terminal
```

- Sem o pacote instalado, a interface roda, mas o que exige root (perfis,
  Turbo, configuração do falcond) fica indisponível.
- `../tests/daemon-authorization.sh` confere que o helper recusa todas as
  ações privilegiadas quando o Polkit não está disponível.
- **Medir a diferença** (menu ⋮ de um jogo que abre diretamente) compara com
  e sem otimizações, em várias execuções alternadas; a engine de benchmark
  (`bigame-core/src/benchmark/`, `scripts/bench-*.sh`) não tem página própria.
- `bigame-core/examples/` traz ferramentas de linha de comando: detecção
  (`detect`, `library`, `running`, `health`), Gráficos com IA (`graphics_scan`,
  `graphics_plan`, `graphics_apply`, `graphics_status`, `graphics_capabilities`,
  `graphics_diagnose`, `graphics_native`, `optiscaler_fetch`, `pe_dump`), Turbo e Booster (`turbo`, `booster_run`, `measure`) e relatórios
  de benchmark (`bench_native_report`, `bench_report`).
  Rode com `cargo run -p bigame-core --example <nome>`.
- `bigame-engine/scripts/` automatiza sessões de benchmark (`bench-game.sh`,
  `bench-lab.sh`); os dados publicados ficam em `bigame-engine/benchmarks/`.

**Traduções:** os catálogos ficam em `locale/*.po`. Depois de mudar textos no
código:

```bash
python3 locale/extract-strings.py        # atualiza locale/bigame-mode.pot
for po in locale/*.po; do
    msgmerge -U --no-wrap --no-fuzzy-matching "$po" locale/bigame-mode.pot
done
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
