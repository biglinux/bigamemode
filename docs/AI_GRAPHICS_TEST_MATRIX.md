# AI Graphics — test matrix

What was exercised, where, and with which evidence. "unit" is a test in
`cargo test --workspace` (551 in bigame-core at the time of writing);
"desktop" is the reference desktop (RX 9060 XT, Vega iGPU, Proton
Experimental 11.0); "laptop" is the lab laptop (GTX 1050 Ti + Intel HD 630,
NVIDIA 580) measured in the previous mission; "—" means not exercised.

## Hardware and platform

| Case | Unit test | Desktop | Laptop |
|---|---|---|---|
| AMD RDNA 4 (RX 9060 XT) | `gpu_names_come_from_the_pci_database_and_give_the_rdna_generation`, `sottr_on_rdna4_gets_fsr4_from_its_xess_with_the_files_listed`, `the_reference_desktop_is_told_exactly_what_is_missing` | SOTTR (OptiScaler +10.1 %), Cyberpunk (FSR 4 through Proton, OptiScaler −6.4 %) | — |
| AMD RDNA 3 | `the_external_amd_backend_needs_rdna_3_or_4_dx12_and_an_ffx_path` (RDNA 3 accepted) | — | — |
| AMD before RDNA (Vega) | `on_older_amd_the_game_keeps_its_own_upscaler_and_nothing_changes` | the iGPU seen, idle, never chosen | — |
| NVIDIA GTX | `a_gtx_is_never_told_to_use_dlss`, `dlss_needs_an_rtx_card_and_frame_generation_needs_ada_or_later` | — | SOTTR +13.4 % (OptiScaler FSR 3.1 from XeSS), Xid on OptiFG |
| NVIDIA RTX with native DLSS | `nvidia_rtx_with_native_dlss_installs_nothing`, `a_measurement_against_xess_does_not_overrule_native_dlss_on_rtx` | — | — |
| Intel Arc | `optiscaler_runs_on_every_vendor_but_only_for_64_bit_windows_games`, `only_nvidia_cards_run_dlss` | — | — |
| Hybrid (two GPUs) | `with_two_gpus_the_plan_says_which_one_it_is_for_until_the_game_runs` | Vega + RX 9060 XT: DRM fdinfo picks the RX | HD 630 + GTX: fdinfo picked the GTX |

## Game shape

| Case | Unit test | Machine |
|---|---|---|
| DirectX 12 detected from imports / files | `imports_are_detected_not_facts`, `a_game_like_sottr_is_read_for_what_it_ships` | SOTTR, Cyberpunk (`amd_fidelityfx_dx12.dll`, `ffx_backend_dx12_x64.dll`) |
| DirectX 11 / renderer picked at run time | `a_game_that_picks_its_renderer_at_run_time_is_only_likely_dx12` | SOTTR DX11 arm (laptop) |
| Vulkan | `the_external_amd_backend_needs_rdna_3_or_4_dx12_and_an_ffx_path` (Vulkan rejected for the neural backend) | — |
| Running game settles the API | `the_running_game_makes_the_api_a_fact`, `the_running_process_name_settles_which_executable_is_the_game` | both games, `/proc/<pid>/maps` |
| Native Linux game | `a_native_linux_game_gets_a_plain_explanation_and_no_files` | — |
| 32-bit game | `a_32_bit_game_keeps_native` | — |
| Anti-cheat | `anti_cheat_is_found_by_folder_file_and_executable`, `anti_cheat_blocks_injection_but_still_points_at_the_games_own_upscaler`, `anti_cheat_blocks_it_whatever_is_installed`, `injection_into_a_protected_game_is_blocked`, `the_game_list_can_block_or_prefer_but_never_unblock` | — |
| Proxy DLL taken (ReShade) | `a_dxgi_owned_by_reshade_stops_the_plan_instead_of_overwriting_it`, `proxy_owners_are_read_from_contents_and_only_beside_the_executable_count` | Cyberpunk's `dbghelp.dll` (Microsoft) listed, not a conflict |
| OptiScaler installed / loaded / active / failed | `installed_loaded_active_and_failed_are_read_not_assumed`, manifest tests | Cyberpunk: installed, active (`Fsr4Update: true`), restored; SOTTR earlier |
| Game ships the FidelityFX API (FSR 3.1 path) | `a_game_whose_own_fsr_runs_fsr4_through_proton_gets_no_optiscaler` | Cyberpunk: provider mapped with the variable, never without |
| Game without FSR path | `sottr_on_rdna4_gets_fsr4_from_its_xess_with_the_files_listed` | SOTTR |
| External neural backend absent / present | `the_reference_desktop_is_told_exactly_what_is_missing`, `external` module tests (installed, loaded, active from a log banner, failed from log errors) | absent: "unavailable: AMD HIP runtime, Neural-rendering model" |
| Old manifests without `backend` / `managed` | `manifest` tests (defaults read as OptiScaler, managed) | the existing SOTTR manifest still verified |

## Rules and plan

| Case | Unit test |
|---|---|
| Two upscalers or two frame generators never in series | `two_upscalers_or_two_frame_generators_never_pass`, `gamescope_and_wine_fsr_are_turned_off_for_the_game_never_stacked` |
| Every rule has a basis, no duplicates | `every_rule_has_a_reason_and_no_pair_is_listed_twice`, `an_unestablished_pair_is_unknown_not_supported`, `problems_are_reported_worst_first_and_harmless_pairs_left_out` |
| Frame generation never automatic | `frame_generation_is_never_automatic_and_is_experimental_when_chosen` |
| Measurements drive Recommended | `a_gain_measured_on_this_machine_makes_optiscaler_the_recommendation`, `a_gain_whose_floor_could_not_be_compared_is_shown_but_not_chosen`, `far_below_60_in_every_measurement_the_plan_points_to_frame_generation` |
| FSR 4 launch option round trip | `the_variable_goes_in_front_and_comes_out_again_leaving_the_rest` |
| Backend ids and management flag | `ids_round_trip_and_only_optiscaler_is_managed` |

## Regression checks (this branch, 2026-09-26)

| Check | Result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 warnings |
| `cargo test --workspace` | 551 + 17 + 18 passed, 0 failed |
| `tests/daemon-authorization.sh` | passed: every privileged request refused with Polkit unreachable |
| `locale/extract-strings.py --check`, `msgfmt pt_BR.po` | up to date; 1111 messages translated (the four new modules added to `POTFILES.in`) |
| Steam launch options restored (both accounts), `UserSettings.json` restored, Cyberpunk's `amd_fidelityfx_dx12.dll` original hash back | verified after the session |

## Not exercised

RDNA 3 hardware, RTX hardware, Intel Arc hardware, a Vulkan game, a native
Linux game, an anti-cheat game and a 32-bit game on a real machine (unit
fixtures only); the AMD neural component under Proton (no HIP runtime, no
model); presented-frame counts for OptiScaler's frame generation on AMD.
