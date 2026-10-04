#pragma once
#include "globals.h"
struct HostBufferManager { double AgeOfOldestBuffer() { return 0; } double AgeOfNewestBuffer() { return 0; } uint32_t BufferCount() { return 0; } uint32_t Depth() { return 0; } };
struct HostSystem { HostBufferManager* GetBufferManagers() { static HostBufferManager b[1]; return b; } };
extern HostSystem* g_ptrSystem;
extern std::mutex g_buffer_mutex;
