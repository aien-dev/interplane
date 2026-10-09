# M5 model-turn evidence (2026-10-09)

Written by `adapters/aien/scripts/test-m5-model-turn.sh` on a clean tree and committed unedited:
`receipt.json`, `negative-verdicts.txt`, `live.log`, `rows/`, `bundle/`.

## Re-verify

    adapters/aien/evidence/m5-model-turn-2026-10-09/reverify.sh <model_dir>

`<model_dir>` is the Llama-3.2-1B-Instruct snapshot the daemon loaded
(`5a8abab4a5d6f164389b1079fb721cfab8d7126c`), holding `model.safetensors`, `tokenizer.json`,
`config.json`. The committed bundle leaves those three files out (2.4 GB), so verifying it as-is
gives `FAIL missing_record: export_config`. The script copies them into a scratch copy of the bundle,
refuses if a digest differs from the manifest, and runs the offline verifier. Expected last line:
`PASS complete effect=aien-ledger-slice/1:strong proposal=model_generation/2 candidate=none`.

The script reads the judge public key and the policy pin from `receipt.json`, so it checks internal
consistency only. For a real audit, pin both from a source you trust and call the verifier directly.

## Note on `negative-verdicts.txt`

That file is the script's output as run. The first counterfeit there is named `response_text_replaced`.
The gate script now names it `response_text_replaced_refused_by_generation_record_binding`, because the
daemon's generation record is what refuses it. The committed file keeps the old name by design.
