"""A small, stateful to-do host and its INTERPLANE authority adapter."""

from interplane import lenshift
from interplane.crossaxis import MappingTable, Rule
from interplane.crossveil import (
    CapabilityDescriptor,
    Catalog,
    Decision,
    Limits,
    Pipeline,
    RuntimeAuthority,
    ToolRef,
    make_result,
)


RUNTIME = "toy"
POLICY = "toy.todo.policy"


def _parameters(required, optional=()):
    return {
        "type": "object",
        "properties": {key: {"type": "string"} for key in (*required, *optional)},
        "required": list(required),
    }


def toy_catalog():
    return Catalog(
        runtime=RUNTIME,
        catalog_version="1",
        capabilities=[
            CapabilityDescriptor("list_items", "List to-do items.", _parameters((), ("scope",)),
                                 canonical=ToolRef("list", "todo"), domains=["todo"],
                                 runtime_effects={"class": "read"}),
            CapabilityDescriptor("add_item", "Add a to-do item.", _parameters(("text",), ("list",)),
                                 canonical=ToolRef("add", "todo"), domains=["todo"],
                                 runtime_effects={"class": "effect"}),
            CapabilityDescriptor("delete_item", "Delete a to-do item.", _parameters(("item_id",)),
                                 canonical=ToolRef("delete", "todo"), domains=["todo"],
                                 runtime_effects={"class": "effect"}),
        ],
    )


def toy_mapping(catalog):
    rules = []
    for name in ("list_items", "add_item", "delete_item"):
        rules.append(Rule("passthrough:" + name, "passthrough", None, name, name))
    for alias, target in (("list", "list_items"), ("add", "add_item"), ("delete", "delete_item")):
        rules.append(Rule("alias:todo." + alias, "alias", "todo", alias, target))
    return MappingTable(RUNTIME, "1", rules, catalog_digest=catalog.computed_digest())


class TodoAuthority(RuntimeAuthority):
    """The host owns policy, approvals, labels, and the in-memory to-do list."""

    def __init__(self, *, no_exposure_check=False, expires_at=None):
        self.no_exposure_check = no_exposure_check
        self.expires_at = expires_at
        self._catalog = toy_catalog()
        self.items = {key: {"id": key, "text": "Existing item", "list": "inbox"}
                      for key in ("old.txt", "a.txt", "b.txt")}
        self.next_id = 1
        self.decide_calls = 0
        self.execute_calls = 0

    def catalog(self):
        return self._catalog

    def decide(self, req, ctx):
        self.decide_calls += 1
        name = req.capability
        args = req.arguments
        required = {"list_items": (), "add_item": ("text",), "delete_item": ("item_id",)}
        optional = {"list_items": ("scope",), "add_item": ("list",), "delete_item": ()}
        if name not in required:
            value, reason = "not_found", "unknown to-do capability"
        elif (not isinstance(args, dict)
              or any(not isinstance(args.get(k), str) for k in required[name])
              or any(k in args and not isinstance(args[k], str) for k in optional[name])):
            value, reason = "invalid", "required to-do argument missing or not text"
        elif name == "delete_item":
            value, reason = "requires_approval", "toy policy: deletion needs approval"
        elif name == "add_item" and not self.no_exposure_check and (
            not isinstance(ctx.get("exposure"), dict)
            or ctx["exposure"].get("floor") not in ("user_supplied", "trusted_runtime")
        ):
            value, reason = "requires_approval", "toy policy: effects after untrusted input need approval"
        else:
            value, reason = "authorized", "toy policy permits this action"
        approval = ({"approval_id": "toy-approval-" + req.request_id,
                     "scope": "single_action", "expires_at": self.expires_at}
                    if value == "requires_approval" else None)
        return Decision(req.request_id, value,
                        authority={"runtime": RUNTIME, "policy_engine": POLICY,
                                   "decision_id": "toy-decision-" + req.request_id},
                        capability=name, reason=reason, approval=approval)

    def execute(self, req, decision, ctx):
        self.execute_calls += 1
        name, args = req.capability, req.arguments
        if name == "list_items":
            data = {"items": list(self.items.values())}
            kind, trust = "workspace_content", "workspace_untrusted"
        elif name == "add_item":
            item_id = "item-" + str(self.next_id)
            self.next_id += 1
            item = {"id": item_id, "text": args["text"], "list": args.get("list", "inbox")}
            self.items[item_id] = item
            data = {"item": item}
            kind, trust = "tool_result", "workspace_untrusted"
        elif name == "delete_item":
            item_id = args["item_id"]
            deleted = self.items.pop(item_id, None) is not None
            data = {"item_id": item_id, "deleted": deleted}
            kind, trust = "tool_result", "workspace_untrusted"
        else:
            raise ValueError("unrecognized to-do capability")
        return make_result(req.request_id, "ok", runtime=RUNTIME, capability=name,
                           data=data, content_kind=kind, trust=trust)


def make_pipeline(authority, limits=None):
    return Pipeline(registry=lenshift, mapping_table=toy_mapping(authority.catalog()),
                    runtime=authority, limits=limits or Limits())
