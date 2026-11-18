# Differential regressions

Commit minimized JSON scenes here once a finding is understood. The GPU test
replays every JSON file with a 2-LSB channel cap and zero outlier pixels. Proptest
writes the final minimized scene to `../failures/minimized.json`; replay with:

```sh
cargo run -p rune-ref --features gpu -- --scene PATH --diff --blame 12,17
```

Kind-2 scenes always describe grayscale R8 atlas masks. Legacy `radii`, `border`
and `params[2]` payload is preserved in the saved JSON but has no sampling meaning.
The GPU adapter clears the text-mode flags and selects page zero, matching the
CPU's single frozen atlas. Capturing production glyphs through
`Primitive::from_instance` rejects outlines, LCD masks and nonzero atlas pages;
those require their own oracle before they can become differential regressions.
