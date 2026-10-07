# Issue 57: refused and held calls and the exposure floor

Status: analysis only. No behaviour change. Rust and Python agree (see the tests at the end).
Evidence is CPU evidence: no model, no GPU.

## 1. What happens today

When the pipeline renders a result back to the model it records it in the trace's input ledger
(Rust `pipeline.rs:1320` calls `record_result` at `:531`, which calls `push_result_input` at `:535`;
Python `crossveil.py:1000` -> `_record_result` `:478` -> `_push_result_input` `:482`). The record's
`content_kind` and `trust` are the result's `provenance`; absent gives `unknown`
(Rust `:540-560`, Python `:490-505`). Host continuations take the same path
(Rust `:1132`, `:1163`; Python `:815`, `:825`).

Results that were not executed carry null `content_kind` and `trust` by design: CORE.md
"Canonical result payload shape" (`spec/CORE.md:254-259`, "no data was produced, so there is no
content to classify"). Rust builds them in `reached` / `non_exec` (`pipeline.rs:~800-840`) and
`reject` (`:597`); Python in `make_result` and `_rejected` (`crossveil.py:522`).

The floor is the minimum over the ledger (Rust `exposure_for` `:512`, Python `:460`). `unknown`
ranks as `external_untrusted`; an empty ledger is `external_untrusted` (CROSSVEIL rule 6,
`spec/CROSSVEIL.md:103-118`). So a null label on a refusal becomes `unknown` in the ledger and
lowers the floor to `external_untrusted` for the rest of the trace. Reproduced on the todo example
at main 0ff60f2: c4 shows `floor before: external_untrusted` with only the "held" reply to c3 as
new input.

The two specs were never reconciled on this point. CORE.md says "null, nothing to classify".
CROSSVEIL rule 6 says every rendered result is recorded, `unknown` when it has no label. The
`unknown` is the generic fail-closed default written for executed results whose adapter said
nothing (rule 2); nothing in the text, the 0.3 plan (`docs/plans/0.3-TRUST-PLAN.md`) or
`bench/PROTOCOL-0.3.md` (gates P and E) discusses refusals. Gate P2 counts only executed results.
There is no separate threat-model document under `docs/`; SECURITY.md states the rule only as
"fail closed on unknown decisions". The intent has to be read from the trust rules.

## 2. What the authors intended

Three stated principles bear on it.

1. The floor is "the least trusted level among inputs visible to the model" (rule 6). The ledger
   exists so that nothing the model sees escapes it. The continuation text says so directly: a
   resolved continuation is recorded "so what the model sees next stays inside its exposure floor"
   (`spec/CORE.md:216-218`). Not recording a refusal body contradicts that.
2. Inability to determine trust is never turned into trust (CROSSVEIL Trust rule 1).
3. The pipeline does NOT record the model's own text, because doing so "would make the floor of every
   multi-turn trace `external_untrusted` by construction and hide real exposure changes"
   (`spec/CROSSVEIL.md:143-146`). The floor is meant to move on real new exposure, not by
   construction.

Reading 1 and 2 together with 3: the `unknown` on a refusal is an unintended side effect of two
rules meeting (the default for unlabelled results, and the null label for non-executed ones), not a
chosen policy. But the safe direction (fail closed) is what the authors chose for every gap, and the
fix must not open one.

## 3. Can a refusal body carry attacker-influenced text?

Yes, in four ways. The model-visible body is `{status, error.code, error.message}` (Lenshift
`serialize_result`, `lenshift/src/lib.rs:181`); the embedded `decision` is not rendered. The
`aien_legacy` dialect also puts the model's raw tool name in the wrapper tag
(`lenshift/aien_legacy.py:157`).

| Source of text | Examples | Author |
|---|---|---|
| fixed pipeline text | `runtime authority raised an error`, `approval cancelled by host`, `too many tool calls in one turn`, `session limit exceeded: <limit>` | pipeline |
| echoed caller text | `no mapping for tool: <name>`, `duplicate request_id: <id>`, `duplicate parameter: <key>`, tool name in the aien_legacy tag | the model, steered by whatever it read |
| adapter text | `policy_denied` and `invalid_arguments` use the decision `reason` verbatim; `approval required: <id>` uses the adapter-minted id (mock: `mock-approval-<request_id>`, which embeds a model-chosen id); `unknown decision value: <value>` echoes an adapter string | the runtime adapter |
| runtime catalog text | `unknown capability: <capability>` | the catalog |

The Odysseus adapter passes the gate's own reason through (`authority.py:197-212`, redacted and cut
to `_MAX_REASON`); the AIEN adapter passes sovereign-core's reason (`adapters/aien/src/lib.rs:350,
:394, :459`). Neither checks that the reason is free of text the model could be steered by.

## 4. Threat cases

- T1 Laundering by label. A refusal recorded `trusted_runtime` while its body carries echoed or
  adapter text puts untrusted bytes in the ledger under a trusted label.
- T2 Empty ledger becomes trust. The floor of an empty ledger is `external_untrusted` (fail
  closed: the host told us nothing). One refusal recorded `trusted_runtime` makes the ledger
  non-empty and the floor `trusted_runtime`, higher than `user_supplied`, so the mock effect policy
  (floor `user_supplied` or higher authorizes) would authorize effects with no registered input at
  all. Demonstrated (scratch run, same pipeline, refusals patched to `trusted_runtime`):
  nothing registered -> `trusted_runtime`; user registered -> `user_supplied`; today ->
  `external_untrusted` in both. An attacker needs only a refused call (an unknown tool name).
- T3 Adapter echo of fresh content. A reason that quotes something the model has not seen (a
  policy file line, a path listing) is new exposure. Not recording it, or recording it trusted,
  hides it from the floor.
- T4 Echo of the model's own text (tool name, request id, argument key). No new information: the
  model wrote it, so it carries at most the floor the model already had (the same argument as
  CROSSVEIL.md:143-146). It cannot lower the floor below the turn's floor.
- T5 Same-turn mixing. A refusal and an untrusted read in one turn: the read must still lower the
  floor, whatever the refusal did.
- T6 Held, approved, executed. The held reply, the continuation result and the executed result
  must each stay in the ledger; approval must not reset the floor.
- T7 Mixed chains. Refusals interleaved with executed results: the floor may only fall or stay.
- T8 Adapter fault strings (decide raised, mismatched request id, unknown decision value): the
  adapter is misbehaving; its text cannot be vouched for.

Invariants every acceptable policy keeps: (I1) a refusal never raises the floor; (I2) an empty
ledger stays `external_untrusted` after a refusal; (I3) echoed or adapter text keeps its taint;
(I4) later calls are judged on the floor exactly as before; (I5) the existing conformance
verdicts are unchanged unless a change is listed and justified.

## 5. Candidate policies

| Policy | I1 | I2 | I3 | Verdict |
|---|---|---|---|---|
| A. record as `runtime_instruction` + `trusted_runtime` | holds (min) | BREAKS (T2) | BREAKS (T1, T3) | unsafe |
| B. do not record | holds | holds | BREAKS for adapter text (T3): body seen, not in ledger; contradicts CORE.md:216-218 | unsafe for adapter text; safe only for pipeline-fixed text |
| C. keep `unknown` (today) | holds | holds | holds | safe, costs the floor after every refusal |
| R. refinement: record the refusal at the floor of the exposure the call was made under, for fixed and echoed text only | holds | holds (empty ledger gives external) | holds (T4 adds nothing new) | safe; does NOT fix the issue's example, whose held reply carries an adapter-minted id |
| V. R plus an optional adapter flag vouching that its `reason` and ids carry no content the model has not seen | holds | holds | holds if the adapter is honest (T3 becomes an adapter duty) | safe; fixes the example; new optional field |

R sets a record's trust to the call's own turn floor, never above it. It cannot raise the floor
(min), and it cannot lower it, because the text it covers is the model's own or pipeline-fixed.
V adds one optional boolean-like field on the decision (a MINOR change in 0.x), defaulting to
today's behaviour; an adapter that does not set it is unchanged.

## 6. Decision

Policies A and B are ruled out on security grounds (T2 is a concrete bypass; B hides adapter text).
The remaining choices (C, R, V) are all safe. They differ in cost, not in safety:

- C: no protocol change; every later effect after a refusal is held. Zero new risk.
- R: protocol revision (CHANGELOG, MINOR bump, fixtures re-pinned where a refusal's record is
  asserted, both SDKs). Helps pipeline rejections, not_found and cancels. Does not fix the todo
  example.
- V: R plus a new optional decision field and an obligation on adapters. Fixes the todo example.
  A wrong vouch only matters when the reason really carries fresh untrusted content, and a vouch can
  never raise the floor above the turn floor.

That is a product call (how much usability is worth a new protocol field and a new adapter duty),
not a security call, so this note stops here. Until it is made, the two files below pin what must
hold under C, R and V alike, and pin today's behaviour in two clearly named tests.

## 7. Tests added (no behaviour change)

`python/tests/test_refusal_exposure.py` (8) and
`rust/crates/interplane-crossveil/tests/refusal_exposure.rs` (8): same scenarios and strings.
Covered: I1, I2, echoed tool name with a later effect still held, an adapter reason carrying
injected text, held then approved then executed, a mixed chain ending in an untrusted read, a
todo-shaped floor sequence on the mock, and the current-policy pin.

Red check: with refusals patched to `trusted_runtime` (policy A) 6 of 8 fail in each SDK; the real
code passes all 8.
