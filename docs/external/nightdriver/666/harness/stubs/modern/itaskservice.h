// Host stub: minimal ITaskService surface used by SocketServer. Not NightDriverStrip code.
#pragma once
#include "globals.h"
#include <atomic>
struct IService { virtual ~IService() = default; virtual const char* Name() const = 0; };
class ITaskService : public IService
{
  public:
    bool Start() { return true; }
    void Stop() { _shutdownRequested = true; }
  protected:
    struct TaskConfig { const char* taskName; size_t stackSize; int priority; int core; uint32_t shutdownTimeoutMs; };
    virtual TaskConfig GetTaskConfig() const = 0;
    virtual void Run() = 0;
    virtual void OnBeforeWaitForStop() {}
    bool ShouldShutdown() const { return _shutdownRequested.load(); }
    std::atomic<bool> _shutdownRequested{false};
};
