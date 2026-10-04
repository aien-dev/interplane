// Host MODEL of ESP-IDF heap capability semantics. Written for this harness, contains no project code.
// Sources (ESP-IDF v5.5.2, the version pinned by NightDriverStrip main via pioarduino 55.03.37):
//   caps bit values:      components/heap/include/esp_heap_caps.h:29-41
//   region capabilities:  components/heap/port/esp32/memory_layout.c:49-54 (DRAM, D/IRAM, IRAM rows)
//   free size / largest:  components/heap/heap_caps.c:277-286 (sum over matching heaps), :367-389 (max over matching heaps)
//   eligibility rule:     components/heap/heap_caps_base.c heap_caps_aligned_alloc_base: (get_all_caps(heap) & caps) == caps
// NOT modelled: TLSF size classes and per-block overhead (a real allocation can fail where this model succeeds,
// never the other way round for the cases below), block-owner bytes, EXEC special cases, PSRAM.
#pragma once
#include <cstddef>
#include <cstdint>
#include <vector>
#include <algorithm>

#define MALLOC_CAP_EXEC     (1u<<0)
#define MALLOC_CAP_32BIT    (1u<<1)
#define MALLOC_CAP_8BIT     (1u<<2)
#define MALLOC_CAP_DMA      (1u<<3)
#define MALLOC_CAP_INTERNAL (1u<<11)
#define MALLOC_CAP_DEFAULT  (1u<<12)

namespace heapmodel {
struct Region { const char* name; uint32_t caps[3]; std::vector<size_t> freeBlocks; };
inline std::vector<Region>& heaps() { static std::vector<Region> h; return h; }
inline uint32_t all_caps(const Region& r) { return r.caps[0] | r.caps[1] | r.caps[2]; }
// memory_layout.c:49-54
inline Region DRAM (std::vector<size_t> b) { return {"DRAM",   {MALLOC_CAP_8BIT|MALLOC_CAP_DEFAULT, MALLOC_CAP_INTERNAL|MALLOC_CAP_DMA|MALLOC_CAP_32BIT, 0}, b}; }
inline Region DIRAM(std::vector<size_t> b) { return {"D/IRAM", {0, MALLOC_CAP_DMA|MALLOC_CAP_8BIT|MALLOC_CAP_INTERNAL|MALLOC_CAP_DEFAULT, MALLOC_CAP_32BIT|MALLOC_CAP_EXEC}, b}; }
inline Region IRAM (std::vector<size_t> b) { return {"IRAM",   {MALLOC_CAP_INTERNAL|MALLOC_CAP_EXEC|MALLOC_CAP_32BIT, 0, 0}, b}; }
inline void reset() { heaps().clear(); }
inline size_t sum(const Region& r) { size_t s = 0; for (auto b : r.freeBlocks) s += b; return s; }
inline size_t largest(const Region& r) { size_t m = 0; for (auto b : r.freeBlocks) m = std::max(m, b); return m; }
// heap_caps_base.c eligibility, first fit over priorities then regions; returns true if a block was found
inline bool try_alloc(size_t size, uint32_t caps) {
    for (int prio = 0; prio < 3; prio++)
        for (auto& r : heaps())
            if ((r.caps[prio] & caps) != 0 && (all_caps(r) & caps) == caps)
                for (auto& b : r.freeBlocks)
                    if (b >= size) { b -= size; return true; }
    return false;
}
}

inline size_t heap_caps_get_free_size(uint32_t caps) {
    size_t t = 0;
    for (auto& r : heapmodel::heaps()) if ((heapmodel::all_caps(r) & caps) == caps) t += heapmodel::sum(r);
    return t;
}
inline size_t heap_caps_get_largest_free_block(uint32_t caps) {
    size_t m = 0;
    for (auto& r : heapmodel::heaps()) if ((heapmodel::all_caps(r) & caps) == caps) m = std::max(m, heapmodel::largest(r));
    return m;
}
inline void* heap_caps_malloc(size_t size, uint32_t caps) { return heapmodel::try_alloc(size, caps) ? (void*)1 : nullptr; }
