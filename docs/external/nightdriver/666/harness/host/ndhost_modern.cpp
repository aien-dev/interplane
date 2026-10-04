// Host driver: runs the real SocketServer::Run() on Linux loopback. Written for this harness.
// Every line on stdout is machine-parsed by the Rust harness:
//   LOG <tag> <text>        firmware debug output
//   PKT cmd=<u> len=<n> fnv=<hex>   a packet handed to ProcessIncomingData
#include "globals.h"
#include "socketserver.h"
#include "values.h"
#include "systemcontainer.h"
#include "nd_network.h"
#include <cstdlib>
#include <fcntl.h>
#include <cstring>
#include <thread>

int g_debugLevel = 1;
HostValues g_Values;
HostSystem g_hostSystem;
HostSystem* g_ptrSystem = &g_hostSystem;
std::mutex g_buffer_mutex;

void host_log(int level, const char* tag, const char* fmt, ...)
{
    if (level > g_debugLevel) return;
    char buf[512];
    va_list ap; va_start(ap, fmt); vsnprintf(buf, sizeof buf, fmt, ap); va_end(ap);
    size_t n = strlen(buf); while (n && (buf[n-1] == '\n')) buf[--n] = 0;
    printf("LOG %s %s\n", tag, buf);
}

namespace nd_network {
bool IsWiFiConnected() { return true; }
void SetSocketBlockingEnabled(int fd, bool blocking)
{
    int fl = fcntl(fd, F_GETFL, 0);
    fcntl(fd, F_SETFL, blocking ? (fl & ~O_NONBLOCK) : (fl | O_NONBLOCK));
}
int GetWiFiRSSI() { return -50; }
}

extern "C" {
void uzlib_uncompress_init(struct uzlib_uncomp*, void*, unsigned) {}
int uzlib_zlib_parse_header(struct uzlib_uncomp*) { return -1; }   // compressed path not exercised
int uzlib_uncompress_chksum(struct uzlib_uncomp*) { return -1; }
}

bool ProcessIncomingData(allocated_unique_ptr<uint8_t[]>& p, size_t len)
{
    uint32_t h = 2166136261u;
    for (size_t i = 0; i < len; i++) { h ^= p[i]; h *= 16777619u; }
    uint16_t cmd = p[0] | (p[1] << 8);
    printf("PKT cmd=%u len=%zu fnv=%08x\n", cmd, len, h);
    return true;
}

struct HostServer : SocketServer
{
    using SocketServer::SocketServer;
    using SocketServer::Run;
};

int main(int argc, char** argv)
{
    if (argc < 2) { fprintf(stderr, "usage: ndhost <port> [debug-level]\n"); return 2; }
    if (argc > 2) g_debugLevel = atoi(argv[2]);
    setvbuf(stdout, nullptr, _IOLBF, 0);
    HostServer s(atoi(argv[1]), NUM_LEDS);
    s.Run();     // the real accept/read/dispatch loop; runs until the harness kills this process
    return 0;
}
