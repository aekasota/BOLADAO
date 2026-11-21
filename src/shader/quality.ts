const SCALE_STEPS = [1.0, 0.85, 0.7] as const;

export class AdaptiveQualityController {
  private stepIndex = 0;
  private samples: number[] = [];
  private unfocusedSince: number | null = null;

  getRenderScale(dpr: number): number {
    const base = Math.min(dpr || 1, 1.5);
    return base * SCALE_STEPS[this.stepIndex];
  }

  recordFrameTimeMs(ms: number): boolean {
    if (ms <= 0 || ms > 200) return false;
    this.samples.push(ms);
    if (this.samples.length < 60) {
      return false;
    }
    const avg = this.samples.reduce((a, b) => a + b, 0) / this.samples.length;
    this.samples.length = 0;

    // Step down if rolling 60-frame mean exceeds ~12 ms (Section 13.7)
    if (avg > 12.0 && this.stepIndex < SCALE_STEPS.length - 1) {
      this.stepIndex += 1;
      return true;
    }
    return false;
  }

  onBlur() {
    if (this.unfocusedSince === null) {
      this.unfocusedSince = performance.now();
    }
  }

  onFocus() {
    this.unfocusedSince = null;
  }

  /** Returns target frame interval (0 = full 60fps rAF, 66.6ms = 15fps after 30s unfocused). */
  targetFrameIntervalMs(): number {
    if (this.unfocusedSince !== null && performance.now() - this.unfocusedSince > 30_000) {
      return 1000 / 15;
    }
    return 0;
  }
}
