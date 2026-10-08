# rsi-eval-v2: a receipt made by spark-rsi's own judge

Cross-language check for `rsi-eval/2` (`provenance/BINDING.md`, "Independent evaluation"). These files
were copied from the recorded VAC M5 gate run (`adapters/aien/evidence/m5-rsi-2026-10-08/`), not written by
hand:

| File | What it is |
|---|---|
| `receipt.json` | The version 2 receipt `spark-rsi-judge` signed with the judge account's key. |
| `policy.json` | The operator policy the judge ran under (its SHA-256 is `binding.policy_sha256`). |
| `subject.md` | The exact README bytes the judge evaluated (their SHA-256 is `binding.subject_sha256`). |
| `judge.pub` | The judge's public key (uncompressed SEC1 hex), as pinned by the operator. |

`tests/evaluation.rs` verifies the receipt with this crate's independent implementation of the digest and
the signature, and checks that changing the subject, the path, the policy, the key or any covered field is
refused. The judge's private key is not here and never leaves the judge account.
