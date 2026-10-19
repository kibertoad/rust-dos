# Video performance follow-ups

The [performance review in issue 164](https://github.com/kibertoad/rust-dos/issues/164)
includes both immediate CPU/presentation changes and larger renderer
proposals. This pass finishes the prepared optimization series, including
the LFB pipeline barrier change, integrates it, and measures the affected
paths with original games or 3DMark. Regressions must be fixed or removed.
The `validated` label requires a demonstrated improvement in real runs;
unit tests and synthetic timings alone do not establish that result.

The following proposals remain separate follow-ups:

- **Decoded software texture images.** Existing Voodoo sampling already
  uses retained decode tables, including 65,536-entry tables for the
  applicable 16-bit formats. An immutable ARGB image cache could remove
  the dependent texture-memory/table lookup. Its benefit must outweigh
  image construction, invalidation and additional memory use.
- **PowerVR GPU rendering.** A replay backend would need the ISP's tile
  and visibility behavior, texturing and translucent passes, plus CPU
  copy-out and emulated completion timing. The persistent software worker
  pool in this pass does not provide that renderer.
- **GPU readback as Voodoo memory.** Replacing deferred software work with
  readback must preserve LFB reads, pipeline writes, raw command-FIFO
  writes, counters, screenshots and save states. The GL presentation path
  alone does not prove those guest-visible memory results are equivalent.
- **Additional GL state transport.** Uniform buffers or another draw-state
  upload scheme can be judged after measuring the location and state
  caches introduced by this series, including compatibility backends.
- **Shared-VRAM GPU backends.** ViRGE, Mach64 and Verite would need conflict
  handling for scanout, CPU accesses and the 2D engine. Verite's default
  LLE drawing also arrives as pixel-engine operations, not HLE triangles.
- **Indexed VGA/text rendering on the GPU.** Palette effects may benefit,
  but CPU consumers such as overlays, monochrome output, recording,
  screenshots and the debugger still need decoded pixels.

Some review premises already differ from the current software path.
Voodoo texel fetches have used unchecked accesses under their established
invariants since commit `5afc4c719ece3732aad2e2749bf14404db524e28`.
An optimized x86-64 probe of the current framebuffer accessors also
removes their nested bounds checks and emits ordinary word moves for
relaxed atomic loads/stores. That does not establish vectorization gains
from replacing atomics, or prove that a different ownership model is safe.
Such a change needs separate evidence rather than another unchecked
access rewrite.
