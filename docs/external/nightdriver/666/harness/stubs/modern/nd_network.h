#pragma once
#include "globals.h"
namespace nd_network { bool IsWiFiConnected(); void SetSocketBlockingEnabled(int fd, bool blocking); int GetWiFiRSSI(); }
