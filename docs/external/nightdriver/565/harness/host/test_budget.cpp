// Logic-level test (MODEL of documented ESP-IDF semantics, not a device run, not the real allocator).
// Real project code under test: include/heapbudget.h (FreeInternalBytes8Bit) from the fork branch.
// Copied from src/systemcontainer.cpp at the same branch (arithmetic only, not compiled from source):
//   memtouse = free > RESERVE ? free - RESERVE : 0; cBuffers = memtouse / memtoalloc; throw if < MIN; clamp to MAX.
#include <cstdio>
#include <cstdlib>
#include "heapbudget.h"
using namespace heapmodel;

static const uint32_t RESERVE = 150000, MIN_BUF = 3, MAX_BUF = 24;
static int fails = 0;
#define CHECK(c, ...) do { if (!(c)) { fails++; printf("FAIL %s:%d ", __FILE__, __LINE__); printf(__VA_ARGS__); printf("\n"); } } while (0)

static uint32_t buffers_for(uint32_t freeMem, uint32_t setBytes, bool* admitted) {
    uint32_t memtouse = freeMem > RESERVE ? freeMem - RESERVE : 0;
    uint32_t c = memtouse / setBytes;
    *admitted = c >= MIN_BUF;
    return c > MAX_BUF ? MAX_BUF : c;
}
static uint32_t old_free() { return (uint32_t)heap_caps_get_free_size(MALLOC_CAP_INTERNAL); }  // Esp.cpp:164 getFreeHeap

static void layout(size_t dram, size_t iram) { reset(); heaps().push_back(DRAM({dram})); heaps().push_back(IRAM({iram})); }

int main() {
    // Case 1: documented classic-ESP32 region mix (mem_alloc.rst:91-93 shows 14+111 KiB D/IRAM and 90 KiB IRAM). Numbers illustrative.
    layout(160000, 90000);
    CHECK(old_free() == 250000, "old=%u", old_free());
    CHECK(FreeInternalBytes8Bit() == 160000, "new=%u", FreeInternalBytes8Bit());
    const uint32_t setBytes = 9000;
    bool okOld, okNew;
    uint32_t nOld = buffers_for(old_free(), setBytes, &okOld);
    uint32_t nNew = buffers_for(FreeInternalBytes8Bit(), setBytes, &okNew);
    printf("case1 old: admit=%d buffers=%u | new: admit=%d buffers=%u\n", okOld, nOld, okNew, nNew);
    uint32_t left8 = (uint32_t)heap_caps_get_free_size(MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT) - (okOld ? nOld * setBytes : 0);
    CHECK(okOld && left8 < RESERVE, "old query should over-admit (8-bit left %u vs reserve %u)", left8, RESERVE);
    CHECK(!okNew, "new query should refuse cleanly");

    // Case 2: invariant sweep. If admitted, 8-bit free after taking the buffers must be >= RESERVE.
    uint32_t seed = 12345, oldViol = 0, newViol = 0, n = 0;
    for (int i = 0; i < 20000; i++) {
        seed = seed * 1664525u + 1013904223u;
        size_t dram = 100000 + (seed >> 8) % 200000;
        seed = seed * 1664525u + 1013904223u;
        size_t iram = (seed >> 8) % 100000;
        seed = seed * 1664525u + 1013904223u;
        uint32_t sb = 600 + (seed >> 8) % 12000;
        layout(dram, iram); n++;
        bool a, b;
        uint32_t co = buffers_for(old_free(), sb, &a), cn = buffers_for(FreeInternalBytes8Bit(), sb, &b);
        if (a && dram < RESERVE + (size_t)co * sb) oldViol++;
        if (b && dram < RESERVE + (size_t)cn * sb) newViol++;
    }
    printf("case2 sweep n=%u old_invariant_violations=%u new_invariant_violations=%u\n", n, oldViol, newViol);
    CHECK(oldViol > 0, "expected old query to violate the invariant somewhere");
    CHECK(newViol == 0, "new query violated the invariant %u times", newViol);

    // Case 3: the issue's literal numbers as a MODEL shape (hypothesis only: the issue gives no caps or call site).
    // 50K INTERNAL free, 34K INTERNAL largest, but the 34K block is in IRAM and DRAM has only small fragments.
    reset(); heaps().push_back(DRAM({3000, 3000, 3000, 3000, 2000, 2000})); heaps().push_back(IRAM({34000}));
    CHECK(old_free() == 50000, "free=%u", old_free());
    CHECK(heap_caps_get_largest_free_block(MALLOC_CAP_INTERNAL) == 34000, "largest internal");
    CHECK(heap_caps_get_largest_free_block(MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT) == 3000, "largest 8bit");
    printf("case3 internal free/largest = %zu/%zu, 8bit free/largest = %zu/%zu\n",
           heap_caps_get_free_size(MALLOC_CAP_INTERNAL), heap_caps_get_largest_free_block(MALLOC_CAP_INTERNAL),
           heap_caps_get_free_size(MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT), heap_caps_get_largest_free_block(MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT));

    CHECK(heap_caps_malloc(4096, MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT) == nullptr, "4K 8-bit alloc must fail in this state");
    CHECK(heap_caps_malloc(4096, MALLOC_CAP_INTERNAL | MALLOC_CAP_32BIT) != nullptr, "4K 32-bit alloc succeeds from IRAM");
    printf(fails ? "RESULT FAIL (%d)\n" : "RESULT PASS\n", fails);
    return fails ? 1 : 0;
}
