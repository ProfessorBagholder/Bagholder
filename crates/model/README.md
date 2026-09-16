# bagholder-model

The derived trading model in Rust, ported from `model.py`.

Ported so far: the value helpers, the option-symbol readers, activity
normalization, `fold_stkdis`, and the book/roll keys.

Fidelity is not assumed. `difftest.py` runs every activity row in
`tests/cases`, plus a spread of symbol shapes the cases do not carry, through
both models and compares all twelve derived fields:

    cargo build --bin difftool && python3 crates/model/difftest.py

`serde_json` carries the `float_roundtrip` feature because its default float
parser reads `120.00000000000001` as `120.0`; the cases contain such a value
and the models disagreed on it until the feature was on.
