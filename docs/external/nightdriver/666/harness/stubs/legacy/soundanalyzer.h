#pragma once
#include "globals.h"
struct HostAnalyzer { bool Enabled() const { return ENABLE_AUDIO != 0; } };
extern HostAnalyzer g_Analyzer;
