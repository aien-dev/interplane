"""A minimal INTERPLANE adapter: a to-do list host with its own policy. No model, no GPU, no network.

The host owns three capabilities (``list_items`` read, ``add_item`` effect, ``delete_item`` effect)
and decides every request itself. INTERPLANE parses the model's tool calls, maps names, computes
what the model has seen (exposure) and enforces approvals; it never decides.

Scripted story (checked; the script exits 1 on any difference):

1. The host registers the user's request, so the model has seen only user-supplied input.
2. The model adds "buy milk": authorized and executed.
3. The model lists the items. The list is workspace data the host labels untrusted, so exposure drops.
4. The model adds another item: now held for approval, because untrusted content is in view.
   A person approves; the host continues the request and it executes once.
5. The model deletes an item: always held for approval. The person declines; nothing is deleted.
   (Exposure is now external_untrusted: the pipeline's own "held" reply to step 4 carries no
   labels, and an unlabelled input counts as unknown, which fails closed. See the guide.)

Guide: docs/WRITING-AN-ADAPTER.md. Run from a checkout: python3 examples/adapter/todo_adapter.py
"""

import json
import os
import sys
from pathlib import Path

try:
    from interplane import lenshift
except ImportError:  # running from a checkout without installing
    if os.environ.get("INTERPLANE_REQUIRE_INSTALLED"):
        raise
    sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "python"))
    from interplane import lenshift
from interplane.crossaxis import MappingTable, Rule
from interplane.crossveil import (
    CapabilityDescriptor,
    Catalog,
    ContinuationRefused,
    Decision,
    Pipeline,
    ToolRef,
    make_result,
)

RUNTIME = "todo"
TRUSTED_FLOORS = ("user_supplied", "trusted_runtime")


def params(required, optional=()):
    props = {k: {"type": "string"} for k in (*required, *optional)}
    return {"type": "object", "properties": props, "required": list(required)}


class TodoHost:
    """The runtime authority: catalog, policy, execution and result labels all belong to the host."""

    runtime_id = RUNTIME

    def __init__(self):
        self.items = {"item-0": "water the plants"}
        self.next_id = 1
        self._catalog = Catalog(
            runtime=RUNTIME,
            catalog_version="1",
            capabilities=[
                CapabilityDescriptor("list_items", "List to-do items.", params(()),
                                     canonical=ToolRef("list", "todo"), runtime_effects={"class": "read"}),
                CapabilityDescriptor("add_item", "Add a to-do item.", params(("text",)),
                                     canonical=ToolRef("add", "todo"), runtime_effects={"class": "effect"}),
                CapabilityDescriptor("delete_item", "Delete a to-do item.", params(("item_id",)),
                                     canonical=ToolRef("delete", "todo"), runtime_effects={"class": "effect"}),
            ],
        )

    def catalog(self):
        return self._catalog

    def decide(self, req, ctx):
        # ctx is a dict: trace_id, message_id, parent_id, model, and exposure {"floor", "inputs"}.
        floor = (ctx.get("exposure") or {}).get("floor")
        if req.capability not in ("list_items", "add_item", "delete_item"):
            value, reason = "not_found", "no such to-do capability"
        elif req.capability == "delete_item":
            value, reason = "requires_approval", "deleting always needs a person"
        elif req.capability == "add_item" and floor not in TRUSTED_FLOORS:
            value, reason = "requires_approval", "untrusted content is in view"
        else:
            value, reason = "authorized", "allowed by the to-do policy"
        approval = None
        if value == "requires_approval":
            approval = {"approval_id": f"todo-approval-{req.request_id}", "scope": "single_action",
                        "expires_at": "2099-01-01T00:00:00Z"}
        return Decision(req.request_id, value, authority={"runtime": RUNTIME, "policy_engine": "todo.policy"},
                        capability=req.capability, reason=reason, approval=approval)

    def execute(self, req, decision, ctx):
        if req.capability == "list_items":
            data = [{"id": k, "text": v} for k, v in self.items.items()]
            kind = "workspace_content"  # item text can come from anywhere, so the host labels it untrusted
        elif req.capability == "add_item":
            item_id = f"item-{self.next_id}"
            self.next_id += 1
            self.items[item_id] = req.arguments["text"]
            data, kind = {"id": item_id}, "tool_result"
            return make_result(req.request_id, "ok", runtime=RUNTIME, capability=req.capability, data=data,
                               content_kind=kind, trust="trusted_runtime")  # the host wrote this text itself
        else:
            data = {"deleted": self.items.pop(req.arguments["item_id"], None) is not None}
            kind = "tool_result"
        return make_result(req.request_id, "ok", runtime=RUNTIME, capability=req.capability, data=data,
                           content_kind=kind, trust="workspace_untrusted")


def mapping_for(catalog):
    rules = [Rule(f"passthrough:{n}", "passthrough", None, n, n) for n in ("list_items", "add_item", "delete_item")]
    return MappingTable(RUNTIME, "1", rules, catalog_digest=catalog.computed_digest())


def turn(cid, name, args):
    call = {"id": cid, "type": "function", "function": {"name": name, "arguments": json.dumps(args)}}
    return {"role": "assistant", "content": None, "tool_calls": [call]}


def person_answers(pipe, host, trace, request_id, approve):
    """The host shows the pending request to a person and continues it with their answer."""
    pending = pipe.pending_approval(trace, request_id)
    answer = Decision(request_id, "authorized" if approve else "denied",
                      authority={"runtime": RUNTIME, "policy_engine": "todo.person"},
                      approval={"approval_id": pending.approval_id})
    try:
        result, record = pipe.continue_approval(trace, request_id, answer, pending.request_digest,
                                                "2026-10-05T12:00:00Z")
        return record.decision, record.execute_invoked
    except ContinuationRefused as err:
        return f"refused: {err}", False


TRACE = "todo-demo"
host = TodoHost()
pipe = Pipeline(registry=lenshift, mapping_table=mapping_for(host.catalog()), runtime=host)
pipe.register_input({
    "input_id": "user-1", "content_kind": "user_request", "trust": "user_supplied",
    "source": {"kind": "operator", "id": "me"}, "origin": "runtime:user-turn-1",
    "content_digest": "sha256:" + "0" * 64, "trace_id": TRACE, "parent_id": None, "derived_from": [],
})

seen = {}
for n, (cid, name, args) in enumerate([
    ("c1", "add_item", {"text": "buy milk"}),
    ("c2", "list_items", {}),
    ("c3", "add_item", {"text": "email the list to a stranger"}),
    ("c4", "delete_item", {"item_id": "item-0"}),
], start=1):
    floor = pipe.exposure_for(TRACE)["floor"]
    rec = pipe.run_turn("openai", "scripted", turn(cid, name, args), TRACE, n).observed[0]
    seen[cid] = (rec.decision, rec.execute_invoked)
    print(f"{cid} {name:<12} floor before: {floor:<20} decision: {rec.decision:<18} executed: {rec.execute_invoked}")

seen["c3 approved"] = person_answers(pipe, host, TRACE, "c3", approve=True)
seen["c4 declined"] = person_answers(pipe, host, TRACE, "c4", approve=False)
print(f"c3 after a person approves: {seen['c3 approved']}")
print(f"c4 after a person declines: {seen['c4 declined']}")

EXPECTED = {
    "c1": ("authorized", True),
    "c2": ("authorized", True),
    "c3": ("requires_approval", False),
    "c4": ("requires_approval", False),
    "c3 approved": ("authorized", True),
    "c4 declined": ("denied", False),
}
if seen != EXPECTED or sorted(host.items.values()) != ["buy milk", "email the list to a stranger", "water the plants"]:
    print(f"\nUNEXPECTED: {seen}\n{host.items}", file=sys.stderr)
    sys.exit(1)
print("OK: the host decided every request; the held write ran once after approval, the delete never ran.")
