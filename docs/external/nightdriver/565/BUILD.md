# NightDriverStrip #565 fix: ESP32 compile check

Revisions: upstream main `4d79c290` (baseline) and fork branch `fix/565-buffer-budget-8bit` (`8e3d3515`, one commit on top of `4d79c290`).
Scope: compile and link only. Nothing was flashed or run on a device. No upstream action was taken.

## Upstream CI
`.github/workflows/CI.yml` runs `pio run -e <env>` for every env from `tools/show_envs.py` minus `config/ci_exclude.json`. At `4d79c290` the exclude list is `[]`, so CI builds all environments (about 60), including `m5demo` and `mesmerizer`. It runs on push, pull_request and workflow_dispatch, and installs the latest PlatformIO.

## Fork CI
Actions permissions on aien-dev/NightDriverStrip: `{"enabled":true,"allowed_actions":"all"}`.
`gh run list -R aien-dev/NightDriverStrip --branch fix/565-buffer-budget-8bit` returned no runs, and `gh run list -R aien-dev/NightDriverStrip -L 3` also returned none. The workflow has not run on the fork. The cause was not investigated.

## Toolchain (linux-aarch64, host `uname -m` = aarch64)
- PlatformIO Core 6.2.0, in an isolated venv at `~/.local/share/pio-venv` (not inside any repo)
- Platform espressif32 55.3.37 (pioarduino), framework-arduinoespressif32 3.3.7
- toolchain-xtensa-esp-elf 14.2.0+20251107 (`xtensa-esp-elf-gcc` esp-14.2.0_20251107). It installed and ran natively on aarch64, so no toolchain was unavailable.

## Finding: upstream main does not build with a fresh library resolve
`lib_deps` has `fastled/FastLED @ ^3.10.1`. A fresh resolve picked FastLED 3.10.5, and both `m5demo` and `mesmerizer` failed on unmodified upstream main (and on the fix branch) at the same line:

    include/hashing.h:44:8: error: using typedef-name 'using CRGB = struct fl::CRGB' after 'struct'
    *** [.pio/build/m5demo/src/debug_cli.cpp.o] Error 1

This is independent of the fix. Only 3.10.5 (failing) and 3.10.1 (passing) were tried. To get a baseline, a local, uncommitted edit pinned `fastled/FastLED @ 3.10.1` in `platformio.ini` in both worktrees. That edit is not part of the fix branch.

## Commands
    python3 -m venv ~/.local/share/pio-venv && ~/.local/share/pio-venv/bin/pip install platformio
    git worktree add --detach ../nd565-main 4d79c290      # baseline; fix branch in ../nd565-fix
    # in each worktree:
    grep -v "^#define cszSSID" include/secrets.example.h > include/secrets.h
    echo '#define cszSSID ""' >> include/secrets.h        # same as CI
    sed -i 's|FastLED               @ ^3.10.1|FastLED               @ 3.10.1|' platformio.ini   # local pin, not committed
    nice -n 19 pio run -j4 -e m5demo
    nice -n 19 pio run -j4 -e mesmerizer

## Results (FastLED 3.10.1)
| env | board class | main 4d79c290 | fix 8e3d3515 |
|---|---|---|---|
| m5demo | classic ESP32, no PSRAM (changed `#else` branch is compiled) | SUCCESS | SUCCESS |
| mesmerizer | ESP-WROVER, `USE_PSRAM=1` via `psram_lx6_flags` (changed branch is preprocessed out) | SUCCESS | SUCCESS |

Size (`xtensa-esp32-elf-size` text/data/bss, and firmware.bin bytes):
| env | main | fix | delta |
|---|---|---|---|
| m5demo | 1572464 / 644260 / 37105; bin 2193552; RAM 64680 B, Flash 2193143 B | identical | 0 |
| mesmerizer | 1766432 / 891121 / 53980; bin 2634384; RAM 80596 B, Flash 2633972 B | identical | 0 |

For `m5demo`, `systemcontainer.cpp.o` and `firmware.bin` differ by hash between the two revisions (the code did change) while section sizes are equal. Both revisions compile without errors. No warnings were reviewed beyond the build exit code.

## Not covered
Other envs (CI builds about 60), S3/octal-PSRAM envs, any runtime behaviour. No code change to the fix was needed.
