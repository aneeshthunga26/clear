# Reference optical maps

Parent guidance: [Smithay adapter](../AGENTS.md).

These immutable data maps are embedded by the renderer. Preserve their exact bytes
and provenance in [README.md](README.md); do not substitute regenerated lens models.
The renderer owns upload, premultiplication, sampler state, and lifetime. Keep the
optical contract in [rendering](../../../../specs/rendering.md#liquid-glass).
Verify dimensions and hashes with the CPU oracle tests after changing assets.
