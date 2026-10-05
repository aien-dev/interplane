"""Smart-home host for a maintainer check of CORE.md "Adding an adapter" and docs/WRITING-AN-ADAPTER.md."""

from interplane import lenshift
from interplane.crossaxis import MappingTable, Rule
from interplane.crossveil import CapabilityDescriptor, Catalog, Decision, Limits, Pipeline, ToolRef, make_result

RUNTIME = "home"
POLICY = "home.policy"
TRUSTED_FLOORS = ("user_supplied", "trusted_runtime")
NAMES = ("read_sensors", "set_light", "send_alert", "unlock_door")


def params(required, optional=()):
    return {"type": "object", "properties": {k: {"type": "string"} for k in (*required, *optional)},
            "required": list(required)}


def home_catalog():
    def cap(name, desc, req, opt, cls):
        return CapabilityDescriptor(name, desc, params(req, opt), canonical=ToolRef(name, "home"),
                                    domains=["home"], runtime_effects={"class": cls})
    return Catalog(runtime=RUNTIME, catalog_version="1", capabilities=[
        cap("read_sensors", "Read the home's sensors.", (), ("room",), "read"),
        cap("set_light", "Set a room's light.", ("room", "level"), (), "effect"),
        cap("send_alert", "Send an alert message out of the home.", ("to", "message"), (), "effect"),
        cap("unlock_door", "Unlock a door.", ("door",), (), "effect"),
    ])


class HomeAuthority:
    runtime_id = RUNTIME

    def __init__(self, no_exposure_check=False, expires_at=None):
        self.no_exposure_check = no_exposure_check
        self.expires_at = expires_at
        self._catalog = home_catalog()
        self.lights = {"hall": "off"}
        self.alerts = []
        self.unlocked = []
        self.decide_calls = 0
        self.execute_calls = 0

    def catalog(self):
        return self._catalog

    def decide(self, req, ctx):
        self.decide_calls += 1
        floor = (ctx.get("exposure") or {}).get("floor")
        trusted = self.no_exposure_check or floor in TRUSTED_FLOORS
        args = req.arguments if isinstance(req.arguments, dict) else None
        required = {"read_sensors": (), "set_light": ("room", "level"), "send_alert": ("to", "message"),
                    "unlock_door": ("door",)}
        if req.capability not in required:
            value, reason = "not_found", "no such home capability"
        elif args is None or any(not isinstance(args.get(k), str) for k in required[req.capability]):
            value, reason = "invalid", "missing or non-text argument"
        elif req.capability == "read_sensors":
            value, reason = "authorized", "reads are allowed"
        elif req.capability == "unlock_door":
            value, reason = "requires_approval", "unlocking a door always needs a person"
        elif req.capability == "set_light":
            value, reason = ("authorized", "allowed") if trusted else ("requires_approval", "untrusted content in view")
        else:  # send_alert: data leaving the home is refused after untrusted input
            value, reason = ("authorized", "allowed") if trusted else ("denied", "no outbound alerts after untrusted input")
        approval = None
        if value == "requires_approval":
            approval = {"approval_id": f"home-approval-{req.request_id}", "scope": "single_action",
                        "expires_at": self.expires_at}
        return Decision(req.request_id, value, authority={"runtime": RUNTIME, "policy_engine": POLICY,
                        "decision_id": f"home-{req.request_id}"}, capability=req.capability, reason=reason,
                        approval=approval)

    def execute(self, req, decision, ctx):
        self.execute_calls += 1
        a = req.arguments
        if req.capability == "read_sensors":
            data = {"lights": dict(self.lights), "temperature_c": 21}
            return make_result(req.request_id, "ok", runtime=RUNTIME, capability=req.capability, data=data,
                               content_kind="workspace_content", trust="workspace_untrusted")  # room names are user-set
        if req.capability == "set_light":
            self.lights[a["room"]] = a["level"]
        elif req.capability == "send_alert":
            self.alerts.append((a["to"], a["message"]))
        else:
            self.unlocked.append(a["door"])
        return make_result(req.request_id, "ok", runtime=RUNTIME, capability=req.capability,
                           data={"done": req.capability}, content_kind="tool_result", trust="trusted_runtime")


def make_pipeline(authority, limits=None):
    rules = [Rule(f"passthrough:{n}", "passthrough", None, n, n) for n in NAMES]
    table = MappingTable(RUNTIME, "1", rules, catalog_digest=authority.catalog().computed_digest())
    return Pipeline(registry=lenshift, mapping_table=table, runtime=authority, limits=limits or Limits())
