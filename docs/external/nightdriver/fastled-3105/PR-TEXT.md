# Upstream PR text (NOT submitted; Drake's button)

Branch: aien-dev/NightDriverStrip `fix/pin-fastled-3103` (one commit `a837953e` on top of upstream main `4d79c290`). Target: `main`.

## Title
Pin FastLED to 3.10.3: 3.10.5 breaks the build

## Description
`platformio.ini` asks for `fastled/FastLED @ ^3.10.1`. A fresh library resolve now picks FastLED 3.10.5, and every environment I tried fails to compile:

    include/hashing.h:44:8: error: using typedef-name 'using CRGB = struct fl::CRGB' after 'struct'

Cause: since FastLED 3.10.4, `CRGB` is `using CRGB = fl::CRGB;` at global scope (`src/crgb.h` in 3.10.4/3.10.5; in 3.10.3 it is still `struct CRGB`). `hashing.h` forward-declares `struct CRGB;`, which is no longer legal.

I tried the obvious source fix first (delete the two forward declarations, since `globals.h` already includes `FastLED.h`). That clears the first error, but the build then fails in about ten more places, because `CRGB` now lives in namespace `fl` and argument-dependent lookup also finds FastLED's own `fl::memset`, `fl::memcpy`, `fl::memmove` and `fl::swap` templates: `call of overloaded 'memset(CRGB*&, int, unsigned int)' is ambiguous` (`gfxbase.cpp`, `gfxbase_transforms.cpp`, `ledbuffer.cpp`, `network.cpp`, `ws281xgfx.cpp`) and `call of overloaded 'swap(fl::CRGB&, fl::CRGB&)' is ambiguous` (inside `std::rotate` in `fan_geometry.h`). That is a larger change than a build fix should be, so this PR only pins the library to 3.10.3, the newest release on the PlatformIO registry that builds (3.10.4 is not on the registry). Adapting to `fl::CRGB` can be a separate change when you want it.

Builds (`pio run -e <env>`, PlatformIO 6.2.0, espressif32 55.3.37, xtensa gcc 14.2.0), fresh library install:

| env | main, FastLED 3.10.1 | main, FastLED 3.10.5 (what `^3.10.1` resolves to today) | this PR (3.10.3) |
|---|---|---|---|
| m5demo | pass | FAIL (hashing.h:44) | pass |
| mesmerizer | pass | FAIL (hashing.h:44) | pass |
| mesmerizer_devkit_s3 | pass | FAIL (hashing.h:44) | pass |

CI note: `.github/workflows/CI.yml` installs PlatformIO fresh and runs `pio run -e <env>` for every environment, so CI will hit this on any run without a warm package cache (PR #907 proposes caching PlatformIO packages, which would hide it until the cache is refreshed). Only 3 of about 60 environments were built here; nothing was flashed or run.

AI assistance: this change and the investigation were done with the help of an AI coding assistant (Claude); I reviewed it and take responsibility for it.

## Contributing requirements
* [ ] I read the contribution guidelines in CONTRIBUTING.md.
* [ ] I understand the BlinkenPerBit metric, and maximized it in this PR. (one line of config, nothing added to the firmware)
* [ ] I selected `main` as the target branch.
* [ ] All code herein is subjected to the license terms in COPYING.txt.
