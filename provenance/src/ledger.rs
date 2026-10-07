//! Effect binding `aien-ledger-slice/1`: the companion's `effect` link bound to the records the
//! AIEN daemon itself wrote for one approved write. Specification: provenance/BINDING.md.
//!
//! The slice is five records exported with `ComposeRecall` (the daemon's own read command):
//! the replay claim (`approved_submission: accepted`), its `committed` settlement, the reserved
//! `approved_grant`, the effect `intent` and the `ack`. The daemon writes all five under its home
//! lock; `ComposeNote` refuses to write any of them. A sixth record, `daemon_run`, is written by
//! the run harness (not by the daemon) and names the daemon process.
//!
//! This module reads and compares. It never authorizes, executes or repairs anything.

use super::{fail, s, Archive, Call, Fail};
use crate::binding::{
    approval_binding_bytes, approved_proposal_bytes, compose_proposal_bytes, ApprovalFields,
    APPROVAL_BINDING,
};
use crate::gojson::sha256_hex;
use serde_json::Value;

/// Record names of the slice, in ledger order.
pub const SLICE: [&str; 5] = [
    "ledger_claim",
    "ledger_committed",
    "ledger_grant",
    "ledger_intent",
    "ledger_ack",
];
pub const DAEMON_RUN: &str = "daemon_run";

/// One exported Cortex record (`ComposeRecordView`).
struct View {
    id: u64,
    links: Vec<u64>,
    digest: String,
    text: Value,
}

fn view(a: &Archive, name: &str, note: &str) -> Result<View, Fail> {
    let Some(v) = a.json(name)? else {
        return fail("missing_record", format!("{name} (not retained)"));
    };
    let bad =
        |what: &str| -> Result<View, Fail> { fail("binding_mismatch", format!("{name}.{what}")) };
    let Some(id) = v.get("id").and_then(Value::as_u64) else {
        return bad("id");
    };
    if v.get("verified") != Some(&Value::Bool(true)) {
        return bad("verified");
    }
    if s(&v, &["note"]) != Some(note) {
        return bad("note");
    }
    let digest = match s(&v, &["digest"]) {
        Some(d) if d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit()) => d.to_string(),
        _ => return bad("digest"),
    };
    let Some(links) = v
        .get("links")
        .and_then(Value::as_array)
        .map(|l| l.iter().map(|x| x.as_u64()).collect::<Option<Vec<u64>>>())
    else {
        return bad("links");
    };
    let Some(links) = links else {
        return bad("links");
    };
    let Some(text) = v
        .get("text")
        .and_then(Value::as_str)
        .and_then(|t| serde_json::from_str::<Value>(t).ok())
        .filter(Value::is_object)
    else {
        return bad("text");
    };
    Ok(View {
        id,
        links,
        digest,
        text,
    })
}

fn st<'a>(v: &'a View, name: &str, k: &str) -> Result<&'a str, Fail> {
    match v.text.get(k).and_then(Value::as_str) {
        Some(x) => Ok(x),
        None => fail("binding_mismatch", format!("{name}.{k}")),
    }
}

fn nu(v: &View, name: &str, k: &str) -> Result<u64, Fail> {
    match v.text.get(k).and_then(Value::as_u64) {
        Some(x) => Ok(x),
        None => fail("binding_mismatch", format!("{name}.{k}")),
    }
}

fn eq<T: PartialEq>(a: T, b: T, what: &str) -> Result<(), Fail> {
    if a == b {
        Ok(())
    } else {
        fail("binding_mismatch", what)
    }
}

/// Check the slice against the INTERPLANE link `t`, the retained call and the AIEN link `aien`.
pub fn check(
    a: &Archive,
    e: &Value,
    t: &Value,
    aien: &Value,
    call: Option<&Call>,
) -> Result<(), Fail> {
    if s(e, &["approval_binding"]) != Some(APPROVAL_BINDING) {
        return fail(
            "unsupported_binding",
            format!(
                "effect.approval_binding != {APPROVAL_BINDING} (got {})",
                s(e, &["approval_binding"]).unwrap_or("absent")
            ),
        );
    }
    for n in SLICE.iter().chain([&DAEMON_RUN]) {
        if !a.has(n) {
            return fail("missing_record_entry", *n);
        }
    }
    let claim = view(a, "ledger_claim", "effect")?;
    let committed = view(a, "ledger_committed", "effect")?;
    let grant = view(a, "ledger_grant", "authorization")?;
    let intent = view(a, "ledger_intent", "effect")?;
    let ack = view(a, "ledger_ack", "effect")?;

    // Record kinds.
    eq(
        st(&claim, "ledger_claim", "approved_submission")?,
        "accepted",
        "ledger_claim.approved_submission",
    )?;
    eq(
        st(&committed, "ledger_committed", "approved_submission")?,
        "committed",
        "ledger_committed.approved_submission",
    )?;
    eq(
        nu(&grant, "ledger_grant", "approved_grant")?,
        1,
        "ledger_grant.approved_grant",
    )?;
    eq(
        st(&intent, "ledger_intent", "phase")?,
        "intent",
        "ledger_intent.phase",
    )?;
    eq(st(&ack, "ledger_ack", "phase")?, "ack", "ledger_ack.phase")?;

    // Ledger order: claim < committed < grant < intent < ack.
    let ids = [claim.id, committed.id, grant.id, intent.id, ack.id];
    if ids.windows(2).any(|w| w[0] >= w[1]) {
        return fail("binding_mismatch", format!("ledger record order {ids:?}"));
    }

    // Settlement and links between the records.
    eq(
        nu(&committed, "ledger_committed", "claim")?,
        claim.id,
        "ledger_committed.claim",
    )?;
    eq(
        committed.links.contains(&claim.id),
        true,
        "ledger_committed.links",
    )?;
    eq(
        nu(&grant, "ledger_grant", "replay_claim")?,
        claim.id,
        "ledger_grant.replay_claim",
    )?;
    let (promotion, evidence) = (
        nu(&grant, "ledger_grant", "cx_promotion")?,
        nu(&grant, "ledger_grant", "cx_evidence")?,
    );
    for x in [promotion, evidence, claim.id] {
        eq(grant.links.contains(&x), true, "ledger_grant.links")?;
    }
    let ev = &committed.text["evidence"];
    eq(
        ev.get("compose_proposal_sha256").and_then(Value::as_str),
        Some(st(&grant, "ledger_grant", "proposal_sha256")?),
        "ledger_committed.evidence.compose_proposal_sha256",
    )?;
    eq(
        ev.get("cx_promotion").and_then(Value::as_u64),
        Some(promotion),
        "ledger_committed.evidence.cx_promotion",
    )?;
    eq(
        ev.get("cx_evidence").and_then(Value::as_u64),
        Some(evidence),
        "ledger_committed.evidence.cx_evidence",
    )?;

    // The approval: recompute the key from the grant's own fields, in the named byte form.
    let g = |k: &str| st(&grant, "ledger_grant", k);
    let bound = ApprovalFields {
        approval_id: g("approval_id")?,
        approved_proposal_sha256: g("approved_proposal_sha256")?,
        approver: g("approver")?,
        content_sha256: g("content_sha256")?,
        desk_key_id: g("desk_key_id")?,
        path: g("path")?,
        request_id: g("request_id")?,
        trace_id: g("trace_id")?,
        workspace: g("workspace")?,
    };
    let key = sha256_hex(&approval_binding_bytes(&bound));
    eq(
        key.as_str(),
        g("approval_key")?,
        "ledger_grant.approval_key (recomputed from its approval fields)",
    )?;
    eq(
        key.as_str(),
        st(&claim, "ledger_claim", "approval_key")?,
        "ledger_claim.approval_key",
    )?;
    eq(
        st(&claim, "ledger_claim", "approval_id")?,
        bound.approval_id,
        "ledger_claim.approval_id",
    )?;

    // Request and trace identity: the daemon's records against the INTERPLANE link.
    let (Some(tid), Some(rid)) = (s(t, &["trace_id"]), s(t, &["request_id"])) else {
        return fail("malformed_companion", "interplane.trace_id/request_id");
    };
    for (who, rec) in [
        ("ledger_grant", g("trace_id")?),
        ("ledger_claim", st(&claim, "ledger_claim", "trace_id")?),
    ] {
        eq(rec, tid, &format!("{who}.trace_id ledger={rec} link={tid}"))?;
    }
    for (who, rec) in [
        ("ledger_grant", g("request_id")?),
        ("ledger_claim", st(&claim, "ledger_claim", "request_id")?),
    ] {
        eq(
            rec,
            rid,
            &format!("{who}.request_id ledger={rec} link={rid}"),
        )?;
    }

    // Intent and ack name the grant and each other.
    eq(
        nu(&intent, "ledger_intent", "authorization")?,
        grant.id,
        "ledger_intent.authorization",
    )?;
    eq(
        intent.links.contains(&grant.id),
        true,
        "ledger_intent.links",
    )?;
    for k in ["proposal_sha256", "path", "target", "content_sha256"] {
        eq(
            st(&intent, "ledger_intent", k)?,
            g(k)?,
            &format!("ledger_intent.{k}"),
        )?;
    }
    eq(
        intent.text.get("prior_sha256"),
        grant.text.get("prior_sha256"),
        "ledger_intent.prior_sha256",
    )?;
    eq(
        st(&intent, "ledger_intent", "tool")?,
        "write_file",
        "ledger_intent.tool",
    )?;
    eq(
        g("target")?,
        format!("{}/{}", g("workspace")?.trim_end_matches('/'), g("path")?).as_str(),
        "ledger_grant.target (workspace + path)",
    )?;
    eq(
        nu(&ack, "ledger_ack", "intent")?,
        intent.id,
        "ledger_ack.intent",
    )?;
    eq(
        nu(&ack, "ledger_ack", "authorization")?,
        grant.id,
        "ledger_ack.authorization",
    )?;
    eq(
        ack.links.contains(&intent.id) && ack.links.contains(&grant.id),
        true,
        "ledger_ack.links",
    )?;
    eq(st(&ack, "ledger_ack", "state")?, "DONE", "ledger_ack.state")?;
    eq(
        st(&ack, "ledger_ack", "content_sha256")?,
        g("content_sha256")?,
        "ledger_ack.content_sha256",
    )?;
    eq(
        st(&ack, "ledger_ack", "disk_sha256")?,
        g("content_sha256")?,
        "ledger_ack.disk_sha256",
    )?;

    // The daemon process that wrote the claim is the process the run harness started, and that
    // process is the executable named in the AIEN link.
    let Some(run) = a.json(DAEMON_RUN)? else {
        return fail("missing_record", format!("{DAEMON_RUN} (not retained)"));
    };
    if s(&run, &["kind"]) != Some("aien-daemon-run") {
        return fail("binding_mismatch", "daemon_run.kind");
    }
    let exec = &claim.text["executor"];
    eq(
        exec.get("pid").and_then(Value::as_u64),
        run.get("pid").and_then(Value::as_u64),
        "ledger_claim.executor.pid vs daemon_run.pid",
    )?;
    eq(
        exec.get("start").and_then(Value::as_u64),
        run.get("start_ticks").and_then(Value::as_u64),
        "ledger_claim.executor.start vs daemon_run.start_ticks",
    )?;
    if run.get("pid").and_then(Value::as_u64).is_none() {
        return fail("binding_mismatch", "daemon_run.pid");
    }
    eq(
        s(&run, &["workspace"]),
        Some(g("workspace")?),
        "daemon_run.workspace vs ledger_grant.workspace",
    )?;
    eq(
        s(&run, &["executable_sha256"]),
        s(aien, &["executable_sha256"]),
        "daemon_run.executable_sha256 vs aien.executable_sha256",
    )?;
    if let Some(log) = a.bytes("aien_load_log")? {
        let text = String::from_utf8_lossy(log);
        let sock = text
            .lines()
            .find_map(|l| l.split_once("Binding socket at "))
            .map(|(_, p)| p.trim());
        eq(
            sock,
            s(&run, &["socket"]),
            "aien_load.socket vs daemon_run.socket",
        )?;
    }

    // The INTERPLANE call: same file, same bytes, same result ids.
    if let Some(c) = call {
        check_call(c, &grant, &intent, &ack, &bound)?;
    }
    Ok(())
}

fn check_call(
    c: &Call,
    grant: &View,
    intent: &View,
    ack: &View,
    b: &ApprovalFields<'_>,
) -> Result<(), Fail> {
    let rid = b.request_id;
    eq(
        c.tool.as_str(),
        "write_file",
        &format!("interplane.tool request={rid}"),
    )?;
    let (Some(path), Some(content)) = (s(&c.args, &["path"]), s(&c.args, &["content"])) else {
        return fail(
            "binding_mismatch",
            format!("interplane.arguments path/content request={rid}"),
        );
    };
    eq(
        path,
        b.path,
        &format!("interplane.arguments.path request={rid}"),
    )?;
    eq(
        sha256_hex(content.as_bytes()).as_str(),
        b.content_sha256,
        &format!("interplane.arguments.content request={rid}"),
    )?;
    eq(
        sha256_hex(&approved_proposal_bytes(path, content)).as_str(),
        b.approved_proposal_sha256,
        &format!("interplane.arguments approved_proposal_sha256 request={rid}"),
    )?;
    eq(
        sha256_hex(&compose_proposal_bytes(path, content)).as_str(),
        st(grant, "ledger_grant", "proposal_sha256")?,
        &format!("interplane.arguments compose_proposal_sha256 request={rid}"),
    )?;
    eq(
        s(&c.result, &["status"]),
        Some("ok"),
        &format!("interplane.result.status request={rid}"),
    )?;
    let rc = &c.result["data"]["receipt"];
    let want = [
        ("grant_id", Value::from(grant.id)),
        ("intent_id", Value::from(intent.id)),
        ("ack_record_id", Value::from(ack.id)),
        ("grant_digest", Value::from(grant.digest.as_str())),
        ("intent_digest", Value::from(intent.digest.as_str())),
        ("ack_digest", Value::from(ack.digest.as_str())),
        ("state", Value::from("DONE")),
        ("request_id", Value::from(b.request_id)),
        ("trace_id", Value::from(b.trace_id)),
    ];
    for (k, v) in want {
        if rc.get(k) != Some(&v) {
            return fail(
                "binding_mismatch",
                format!("interplane.result.data.receipt.{k} request={rid}"),
            );
        }
    }
    Ok(())
}
