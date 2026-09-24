> **Historical — superseded 2026-09-23.**
> This document describes the implementation as it stood before the audit and
> rebuild, and parts of it are no longer true (the Booster `AdwSwitchRow`, the
> old game detection and the old Gamescope builder no longer exist). It is kept
> for context, not as a reference.
> Current documentation: [README.md](README.md).

# Prompt: Adicionar Upscaling Espacial e Novas Tecnologias de Frame Generation ao BiGame-mode

## Contexto Atual do Projeto
- **Nome:** BiGame-mode
- **Linguagem Principal:** Rust (Interface GTK4/Libadwaita)
- **Dependências já integradas:** `falcond` (orquestrador de performance) e `lsfg-vk` (Lossless Scaling via Vulkan)
- **Objetivo:** Central de otimização para jogos no Linux (BigLinux)

## Tarefa Principal
Implementar na interface gráfica e na lógica de orquestração do BiGame-mode o suporte para as seguintes tecnologias:

### 1. Para Aumento de Resolução (Upscaling Espacial)

| Tecnologia | Tipo de Integração | Comportamento Esperado |
|------------|--------------------|------------------------|
| **Gamescope** (FSR 1.0, NIS, Integer Scaling) | Gerenciar via `gamescope` | Adicionar opções na GUI para selecionar o filtro e a resolução base. O comando deve ser gerado dinamicamente: `gamescope -w [largura_base] -h [altura_base] -W [largura_alvo] -H [altura_alvo] -U --fsr-sharpness [0-20]` (ou `--nis` / `--integer-scaling`). |
| **WINE_FULLSCREEN_FSR** | Variável de ambiente | Opção de toggle para adicionar `WINE_FULLSCREEN_FSR=1` e `WINE_FULLSCREEN_FSR_MODE=ultra` (ou outras qualidades) nas variáveis de ambiente do jogo (Proton/Wine). |
| **vkBasalt / ReShade** | Injeção de shaders | Checkbox para ativar `vkBasalt` (se instalado) e menu para escolher o arquivo de configuração (`vkBasalt.conf`). A integração deve definir `ENABLE_VKBASALT=1`. |

### 2. Para Geração de Quadros Artificiais (Frame Generation)

| Tecnologia | Tipo de Integração | Comportamento Esperado |
|------------|--------------------|------------------------|
| **OptiScaler + dlssg-to-fsr3** (FSR 3 FG) | Overlay / Injeção Vulkan | Adicionar um seletor de "Camada de Frame Generation" que, quando ativado, fará o pré-carregamento da `dxgi.dll`/`nvngx.dll` do OptiScaler na pasta do jogo. Deve também permitir configurar o modo (FSR3, XeSS, etc.) e o indicador de status (OSD). |
| **AFMF** (AMD Fluid Motion Frames) | Driver (Mesa RADV / amdgpu-pro) | Implementar um alerta informativo (tooltip) sobre a limitação no Linux: "O AFMF total requer amdgpu-pro. No driver livre (RADV), a implementação é experimental." Para usuários avançados, permitir habilitar variáveis experimentais como `RADV_PERFTEST=afmf`. |

## Requisitos de Implementação (Para o Copilot)

1.  **Modelos de Dados (Rust):** Crie ou estenda `structs` em `src/models/` para armazenar as novas configurações (ex: `UpscalingConfig`, `FrameGenConfig`).

2.  **Interface Gráfica (GTK4/Libadwaita):**
    *   Adicione uma nova aba/widget em `src/ui/dashboard.rs` ou crie um diálogo "Configurações Avançadas de Vídeo".
    *   Use `AdwExpanderRow` para agrupar "Upscaling Espacial" e "Frame Generation".
    *   Para cada tecnologia, use `GtkSwitch`, `GtkDropDown` (para modos) e `GtkScale` (para parâmetros como nitidez).

3.  **Orquestração de Comandos:** Modifique o módulo `bigame-engine` (src/engine/) para que, ao iniciar um jogo, ele:
    *   Monte o comando `gamescope` se a opção estiver ativa.
    *   Injete as variáveis de ambiente (`WINE_FULLSCREEN_FSR`, `ENABLE_VKBASALT`).
    *   Execute scripts auxiliares para copiar arquivos `.dll` do OptiScaler para o prefixo do jogo (se aplicável).

4.  **Integração com `falcond` e `lsfg-vk`:**
    *   As novas tecnologias devem coexistir com o `lsfg-vk` atual. Crie uma lógica de prioridade ou aviso para evitar conflitos (ex: não ativar `lsfg-vk` e `OptiScaler` ao mesmo tempo, pois ambos geram quadros).

5.  **Traduções e Tooltips:** Adicione strings explicativas (em português e inglês) para cada nova opção, exemplificando:
    *   "Gamescope: Melhor qualidade de upscaling, mas usa mais VRAM."
    *   "OptiScaler: Geração de quadros via FSR 3 para jogos com DLSS3."

## Exemplo de Código (Esqueleto para o Copilot gerar)

```rust
// Em src/models/settings.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpscalingSettings {
    pub gamescope_enabled: bool,
    pub gamescope_filter: GamescopeFilter, // Fsr, Nis, Integer
    pub gamescope_sharpness: f32,
    pub wine_fsr_enabled: bool,
    pub vkbasalt_enabled: bool,
}

// Em src/engine/launcher.rs
fn build_gamescope_command(config: &UpscalingSettings, game_res: (u32, u32)) -> Option<String> {
    // Lógica para retornar o prefixo "gamescope ..." ou None
}
