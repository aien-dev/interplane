# NightDriverStrip: a fresh FastLED resolve breaks the build (lane D3)

External proving-ground lane D3. Upstream: PlummersSoftwareLLC/NightDriverStrip, main `4d79c290`.
Nothing here was sent upstream. Fork branch: aien-dev/NightDriverStrip `fix/pin-fastled-3103` (`a837953e`). PR text: `PR-TEXT.md`.

**Outcome:** the originally suspected one-file source fix is not enough. Fix shipped on the fork is a one-line library pin to FastLED 3.10.3. Evidence tags: observed, reproduced, fixed, tested.

## Observed
- [observed] `platformio.ini:56` has `fastled/FastLED @ ^3.10.1`. A fresh resolve gives 3.10.5 (PlatformIO registry lists 3.10.1, 3.10.2, 3.10.3, 3.10.5; there is no 3.10.4; GitHub also has 3.10.6, released 2026-10-04, not on the registry).
- [observed] On unmodified main with 3.10.5, `m5demo`, `mesmerizer` and `mesmerizer_devkit_s3` all fail: `include/hashing.h:44:8: error: using typedef-name 'using CRGB = struct fl::CRGB' after 'struct'`.
- [observed] Upstream check at lane start: no commits on main after `4d79c290`; no open issue or PR mentioning FastLED 3.10, CRGB or hashing.h (searched issues and PRs for FastLED, CRGB, hashing.h, 3.10). Only open PR is #907 "Cache PlatformIO packages in CI", which is relevant (see Scope).

## Reproduction
    # worktree at 4d79c290, secrets.h as in CI, PlatformIO 6.2.0 (~/.local/share/pio-venv)
    nice -n 19 pio run -j4 -e m5demo        # resolves FastLED 3.10.5, fails
Raw rows: `results-matrix1.tsv`, `results-matrix2.tsv` (version actually installed is read from the installed `library.json`).

## Root Cause
- [observed] FastLED 3.10.3 `src/crgb.h:86` is `struct CRGB {`. FastLED 3.10.4, 3.10.5 and 3.10.6 have `struct CRGB` in `src/fl/gfx/crgb.h:38` (namespace `fl`) and `src/crgb.h:25` has `using CRGB = fl::CRGB;` with the comment "Backward compatibility: bring fl::CRGB into global namespace". So `struct CRGB;` in `include/hashing.h:44` (a forward declaration meant to avoid heavy headers) is now ill-formed.
- [observed] The forward declarations are redundant: `hashing.h` includes `globals.h`, whose line 107 includes `<FastLED.h>`.
- [observed] Removing them is not sufficient. With 3.10.5 the build then fails at 11 more sites (m5demo, `scons -k`): `call of overloaded 'memset|memcpy|memmove(CRGB*...)' is ambiguous` in `gfxbase.cpp` (203, 631, 661), `gfxbase_transforms.cpp` (71, 73, 117, 119), `ledbuffer.cpp:132`, `network.cpp:1130`, `ws281xgfx.cpp:233`, and `swap(fl::CRGB&, fl::CRGB&) is ambiguous` inside `std::rotate` from `include/effects/strip/fan_geometry.h:31,36`. [hypothesized, consistent with the messages] Because `CRGB` is now in `fl`, argument-dependent lookup also finds FastLED's `fl::swap` (`fl/stl/type_traits.h:877`) and `fl::memset` family. Other environments may have more sites; only m5demo was enumerated.

## Fix
- [fixed] Fork branch pins `fastled/FastLED @ 3.10.3` with a two-line comment. 3.10.3 is the newest registry release that predates the alias change. Rejected alternative: a source fix working with both 3.10.1 and 3.10.5. It would touch about 8 files (qualify as `::memset`, replace `std::rotate` with a hand loop), cannot be fully enumerated without building all ~60 envs, and goes beyond "smallest change". It can be a follow-up.

## Regression (build matrix)
`pio run -e <env>`, espressif32 55.3.37, xtensa gcc 14.2.0, libdeps removed before each version change.

| env | main + FastLED 3.10.1 | main + 3.10.5 | main + 3.10.1 after rm forward decls* | fork pin 3.10.3 |
|---|---|---|---|---|
| m5demo | pass | FAIL hashing.h:44 | n/a | pass |
| mesmerizer | pass | FAIL hashing.h:44 | n/a | pass |
| mesmerizer_devkit_s3 (ESP32-S3) | pass | FAIL hashing.h:44 | n/a | pass |

*The forward-declaration removal was tested with 3.10.1 and 3.10.5 (first matrix, all 3 envs): 3.10.1 passes, 3.10.5 fails at the swap/memset sites above (`results-matrix1.tsv`, rows "nd-fl-fix"). That commit was discarded and is not on the fork.
Also tried: pin 3.10.4: not on the registry (`UnknownPackageError`). 3.10.2 and 3.10.6 were not built.

## Scope
- Compile only, 3 of about 60 envs that CI builds (CI excludes none at this commit). Nothing flashed or run.
- CI (`.github/workflows/CI.yml`) installs PlatformIO fresh and resolves libraries fresh, so it will fail on any uncached run. PR #907 (package caching) would mask the break until the cache is refreshed.
- The pin hides, not fixes, the incompatibility with FastLED 3.10.4+. Upstream will need the source adaptation eventually.

## INTERPLANE Lesson
Dependency version ranges must be qualified, not assumed. `^3.10.1` was a claim about every future 3.x release; a minor release changed a core type to an alias and broke an untouched project. Qualify the range by building against its newest member, and keep a lockfile or exact pin where the build matters.
