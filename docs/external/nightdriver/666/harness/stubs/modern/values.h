#pragma once
#include "globals.h"
struct HostValues { struct { double CurrentTime() const { return 0; } } AppTime; double Brite = 0; double FPS = 0; double Watts = 0; };
extern HostValues g_Values;
