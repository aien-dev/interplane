"""Hand-curated domains and canonical aliases for the 71 recorded Odysseus tools.

Reviewable by design: one line per tool. Domains are INTERPLANE CrossAxis Select coordinates,
chosen from the closed set below by reading each tool's name, description and effects in the
recorded catalog. They grant nothing; an included tool still crosses Crossveil and Odysseus.
"""

from __future__ import annotations

import copy
from typing import Optional

from interplane.core import Catalog, ToolRef

from .catalog import load_record

DOMAIN_SET = frozenset(
    {
        "filesystem", "code", "process", "web", "email", "calendar", "contacts", "documents",
        "notes", "tasks", "memory", "sessions", "models", "serving", "image", "admin", "ui",
        "research", "user",
    }
)

# tool name -> domains (order matters: CrossAxis Select reports the first matching domain)
TOOL_DOMAINS: dict = {
    "adopt_served_model": ["serving", "models", "admin"],
    "api_call": ["admin"],
    "app_api": ["admin"],
    "apply_patch": ["filesystem", "code"],
    "archive_email": ["email"],
    "ask_teacher": ["user", "models"],
    "ask_user": ["user"],
    "bash": ["process", "code"],
    "bulk_email": ["email"],
    "cancel_download": ["models", "serving", "admin"],
    "chat_with_model": ["models", "sessions"],
    "create_document": ["documents"],
    "create_session": ["sessions", "models"],
    "delete_email": ["email"],
    "download_model": ["models", "serving", "admin"],
    "edit_document": ["documents"],
    "edit_file": ["filesystem", "code"],
    "edit_image": ["image"],
    "get_workspace": ["filesystem"],
    "glob": ["filesystem", "code"],
    "grep": ["code", "filesystem"],
    "list_cached_models": ["models", "serving"],
    "list_cookbook_servers": ["serving"],
    "list_downloads": ["models", "serving"],
    "list_email_accounts": ["email"],
    "list_emails": ["email"],
    "list_models": ["models"],
    "list_serve_presets": ["serving"],
    "list_served_models": ["serving", "models"],
    "list_sessions": ["sessions"],
    "ls": ["filesystem"],
    "manage_bg_jobs": ["process"],
    "manage_calendar": ["calendar"],
    "manage_contact": ["contacts"],
    "manage_documents": ["documents"],
    "manage_endpoints": ["admin", "models"],
    "manage_mcp": ["admin"],
    "manage_memory": ["memory"],
    "manage_notes": ["notes"],
    "manage_session": ["sessions"],
    "manage_settings": ["admin"],
    "manage_skills": ["memory"],
    "manage_tasks": ["tasks"],
    "manage_tokens": ["admin"],
    "manage_webhooks": ["admin"],
    "mark_email_read": ["email"],
    "pipeline": ["models"],
    "python": ["process", "code"],
    "read_email": ["email"],
    "read_file": ["filesystem", "code"],
    "reply_to_email": ["email"],
    "resolve_contact": ["contacts"],
    "scan_email_unsubscribes": ["email"],
    "search_chats": ["sessions"],
    "search_hf_models": ["models", "web"],
    "send_email": ["email"],
    "send_to_session": ["sessions"],
    "serve_model": ["serving", "models", "admin"],
    "serve_preset": ["serving", "admin"],
    "stop_served_model": ["serving", "admin"],
    "suggest_document": ["documents"],
    "tail_serve_output": ["serving"],
    "todowrite": ["tasks", "code"],
    "trigger_research": ["research"],
    "ui_control": ["ui"],
    "unsubscribe_email": ["email"],
    "update_document": ["documents"],
    "update_plan": ["tasks", "user"],
    "web_fetch": ["web"],
    "web_search": ["web", "research"],
    "write_file": ["filesystem", "code"],
}

# canonical "<namespace>.<name>" -> Odysseus tool (the CrossAxis alias rules)
CANONICAL: dict = {
    "filesystem.read": "read_file",
    "filesystem.list": "ls",
    "filesystem.write": "write_file",
    "filesystem.edit": "edit_file",
    "filesystem.glob": "glob",
    "code.search": "grep",
    "process.run": "bash",
    "process.python": "python",
    "web.fetch": "web_fetch",
    "web.search": "web_search",
    "email.send": "send_email",
    "email.list": "list_emails",
    "email.read": "read_email",
    "memory.manage": "manage_memory",
    "documents.create": "create_document",
    "user.ask": "ask_user",
}


def canonical_of(tool: str) -> Optional[str]:
    for canon, native in CANONICAL.items():
        if native == tool:
            return canon
    return None


def catalog_with_domains() -> Catalog:
    """The recorded catalog with ``domains`` and ``canonical`` filled in (digest unchanged)."""
    record = copy.deepcopy(load_record())
    for cap in record["capabilities"]:
        name = cap["name"]
        cap["domains"] = list(TOOL_DOMAINS[name])
        canon = canonical_of(name)
        if canon:
            ns, _, short = canon.partition(".")
            cap["canonical"] = ToolRef(name=short, namespace=ns).to_dict()
    return Catalog.from_dict(record)
