# Magnifying-glass reference maps

Original data maps used by kube.io's [Magnifying Glass demo](https://kube.io/blog/liquid-glass-css-svg/#magnifying-glass), retrieved 2026-10-04. These are the actual published PNG bytes, not approximations regenerated from the article's explanatory formulas.

| Local file | Published asset | Dimensions | SHA-256 |
| --- | --- | --- | --- |
| `magnifying.png` | [magnifying-map-q51ggw.png](https://kube.io/assets/magnifying-map-q51ggw.png) | 210×150 | `991d5a0b7b8ce67a14e03e5ff4bea56d7432eaef87c8def94b8d4fbc5482e6b0` |
| `displacement.png` | [displacement-map-w2qrsb.png](https://kube.io/assets/displacement-map-w2qrsb.png) | 420×300 | `4b65b346a2d5c50b3dae3e6436e4c9d6acb16104b317cdce256e090b9a4dfcff` |
| `specular.png` | [specular-map-w2qrsb.png](https://kube.io/assets/specular-map-w2qrsb.png) | 420×300 | `f49768994e5f39532370c7c37d00487fbd8efd93046418ef10017cf1249b7d6a` |

The reference filter was inspected in [blog-sWnMgqJX.js](https://kube.io/assets/blog-sWnMgqJX.js). Its resting state uses magnification scale 24, refraction scale 122.80891678834695 × 0.8 × Refraction Level, no internal blur, Specular Opacity 0.5, and Specular Saturation 9. Active dragging changes scales and shadows; Clear translates the resting optical filter only.

The [rendering specification](../../../../specs/rendering.md#liquid-glass) owns the compositor mapping and resource contract. The CPU oracle checks these hashes and decodes the PNGs independently of Rust. Do not rewrite or optimize these PNGs without updating that reference evidence.
