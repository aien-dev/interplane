// Host stub for the platform layer. Written for this harness; not NightDriverStrip code.
// Provides only what src/socketserver.cpp and include/socketserver.h need from "globals.h".
#pragma once
#include <cstdint>
#include <cstddef>
#include <cstdio>
#include <cstdarg>
#include <memory>
#include <mutex>
#include <unistd.h>

#ifndef ENABLE_AUDIO
#define ENABLE_AUDIO 0
#endif
#define INCOMING_WIFI_ENABLED 1
#define USE_PSRAM 0
#ifndef NUM_LEDS
#define NUM_LEDS 144
#endif
#define NUM_BANDS 16
#define FLASH_VERSION 1
#define WIFI_COMMAND_PIXELDATA64 3
#define WIFI_COMMAND_PEAKDATA    4

struct CRGB { uint8_t r, g, b; };

template <typename T> using allocated_unique_ptr = std::unique_ptr<T>;
template <typename T> std::unique_ptr<T> make_unique_psram(size_t n) { return std::unique_ptr<T>(new typename std::remove_extent<T>::type[n]()); }
template <typename T> std::unique_ptr<T> make_unique_internal(size_t n) { return make_unique_psram<T>(n); }

extern int g_debugLevel;   // 0 quiet, 1 errors, 2 verbose
void host_log(int level, const char* tag, const char* fmt, ...) __attribute__((format(printf, 3, 4)));
#define debugV(...) host_log(2, "V", __VA_ARGS__)
#define debugI(...) host_log(1, "I", __VA_ARGS__)
#define debugW(...) host_log(1, "W", __VA_ARGS__)
#define debugE(...) host_log(1, "E", __VA_ARGS__)
inline void delay(unsigned ms) { usleep(ms * 1000u); }
