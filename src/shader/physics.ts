export interface Rgb {
  r: number;
  g: number;
  b: number;
}

export interface ThemeShaderColors {
  colA: Rgb;
  colB: Rgb;
  bg: Rgb;
}

export const THEME_SHADER_PALETTES: Record<string, ThemeShaderColors> = {
  sangue: {
    colA: hexToRgb("#FA023E"),
    colB: hexToRgb("#161616"),
    bg: hexToRgb("#0E0E10"),
  },
  papel: {
    colA: hexToRgb("#161616"),
    colB: hexToRgb("#F2EEE6"),
    bg: hexToRgb("#F2EEE6"),
  },
  acido: {
    colA: hexToRgb("#C6FF00"),
    colB: hexToRgb("#121212"),
    bg: hexToRgb("#0D0E0B"),
  },
  cobalto: {
    colA: hexToRgb("#2B4BFF"),
    colB: hexToRgb("#0D0F1A"),
    bg: hexToRgb("#0B0D16"),
  },
  breu: {
    colA: hexToRgb("#000000"),
    colB: hexToRgb("#1A1A1A"),
    bg: hexToRgb("#000000"),
  },
};

export function hexToRgb(hex: string): Rgb {
  const h = hex.replace("#", "");
  return {
    r: parseInt(h.slice(0, 2), 16) / 255,
    g: parseInt(h.slice(2, 4), 16) / 255,
    b: parseInt(h.slice(4, 6), 16) / 255,
  };
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

function lerpRgb(a: Rgb, b: Rgb, t: number): Rgb {
  return {
    r: lerp(a.r, b.r, t),
    g: lerp(a.g, b.g, t),
    b: lerp(a.b, b.b, t),
  };
}

export class ShaderPhysics {
  // Cursor smoothing (half-life ~40ms pos, ~80ms vel)
  targetMouseX = 0;
  targetMouseY = 0;
  rawPrevX = 0;
  rawPrevY = 0;
  mouseX = 0;
  mouseY = 0;
  velX = 0;
  velY = 0;
  pointerInside = false;

  // Damped spring for uForce (stiffness ~120, damping ~20)
  force = 0;
  forceVel = 0;

  // Ripple (click or download complete)
  rippleX = 0;
  rippleY = 0;
  rippleAge = 99;
  rippleStrength = 0;

  // Drag-over attraction well
  attractX = 0;
  attractY = 0.22;
  attractStrength = 0;
  attractTarget = 0;

  // Flow phase linked to network throughput: base + log1p(MB/s) * k
  time = 0;
  phase = 0;
  smoothedFlow = 0.045;

  // 600ms theme transition
  fromColors: ThemeShaderColors = THEME_SHADER_PALETTES.sangue;
  toColors: ThemeShaderColors = THEME_SHADER_PALETTES.sangue;
  currentColors: ThemeShaderColors = THEME_SHADER_PALETTES.sangue;
  themeMix = 1;

  setTheme(themeId: string) {
    const next = THEME_SHADER_PALETTES[themeId] ?? THEME_SHADER_PALETTES.sangue;
    if (
      next.colA.r === this.toColors.colA.r &&
      next.colA.g === this.toColors.colA.g &&
      next.colA.b === this.toColors.colA.b
    ) {
      return;
    }
    this.fromColors = { ...this.currentColors };
    this.toColors = next;
    this.themeMix = 0;
  }

  snapThemeImmediate(themeId: string) {
    const next = THEME_SHADER_PALETTES[themeId] ?? THEME_SHADER_PALETTES.sangue;
    this.fromColors = next;
    this.toColors = next;
    this.currentColors = next;
    this.themeMix = 1;
  }

  onPointerMove(clientX: number, clientY: number, width: number, height: number) {
    const h = Math.max(height, 1);
    const glY = height - clientY;
    this.targetMouseX = (clientX - 0.5 * width) / h;
    this.targetMouseY = (glY - 0.5 * height) / h;
    this.pointerInside = true;
  }

  onPointerLeave() {
    this.pointerInside = false;
  }

  triggerRipple(clientX: number, clientY: number, width: number, height: number, strength = 1.0) {
    const h = Math.max(height, 1);
    const glY = height - clientY;
    this.rippleX = (clientX - 0.5 * width) / h;
    this.rippleY = (glY - 0.5 * height) / h;
    this.rippleAge = 0;
    this.rippleStrength = strength;
  }

  setDragAttract(active: boolean, centerX: number, centerY: number, width: number, height: number) {
    const h = Math.max(height, 1);
    const glY = height - centerY;
    this.attractX = (centerX - 0.5 * width) / h;
    this.attractY = (glY - 0.5 * height) / h;
    this.attractTarget = active ? 1 : 0;
  }

  step(dtRaw: number, speedBps: number, screenHeightPx: number) {
    const dt = Math.min(Math.max(dtRaw, 0.001), 0.05);
    this.time += dt;

    // Exponential decay factors from half-lives (40ms & 80ms)
    const alphaPos = 1 - Math.pow(0.5, dt / 0.04);
    const alphaVel = 1 - Math.pow(0.5, dt / 0.08);

    const prevX = this.mouseX;
    const prevY = this.mouseY;
    this.mouseX = lerp(this.mouseX, this.targetMouseX, alphaPos);
    this.mouseY = lerp(this.mouseY, this.targetMouseY, alphaPos);

    const instVelX = (this.mouseX - prevX) / dt;
    const instVelY = (this.mouseY - prevY) / dt;
    this.velX = lerp(this.velX, instVelX, alphaVel);
    this.velY = lerp(this.velY, instVelY, alphaVel);

    // Speed in px/s (saturates at ~1500 px/s)
    const speedPxPerSec = Math.hypot(this.velX, this.velY) * Math.max(screenHeightPx, 600);
    const forceTarget = this.pointerInside
      ? 0.25 + 0.75 * Math.min(speedPxPerSec / 1500.0, 1.0)
      : 0.0;

    // Damped spring: stiffness = 120, damping = 20
    const stiffness = 120.0;
    const damping = 20.0;
    const accel = stiffness * (forceTarget - this.force) - damping * this.forceVel;
    this.forceVel += accel * dt;
    this.force = Math.max(0, Math.min(1.15, this.force + this.forceVel * dt));

    // Ripple aging
    if (this.rippleAge < 5.0) {
      this.rippleAge += dt;
    }

    // Smooth drag-over attraction
    this.attractStrength = lerp(this.attractStrength, this.attractTarget, 1 - Math.pow(0.5, dt / 0.09));

    // Network speed coupling: uFlow = slow_base + log1p(MB/s) * k
    const mbps = speedBps / (1024 * 1024);
    const targetFlow = 0.045 + Math.log1p(Math.max(0, mbps)) * 0.24;
    this.smoothedFlow = lerp(this.smoothedFlow, targetFlow, 1 - Math.pow(0.5, dt / 0.35));
    this.phase += dt * this.smoothedFlow;

    // Theme color transition (~600ms)
    if (this.themeMix < 1) {
      this.themeMix = Math.min(1, this.themeMix + dt / 0.6);
      const eased = 1 - Math.pow(1 - this.themeMix, 3);
      this.currentColors = {
        colA: lerpRgb(this.fromColors.colA, this.toColors.colA, eased),
        colB: lerpRgb(this.fromColors.colB, this.toColors.colB, eased),
        bg: lerpRgb(this.fromColors.bg, this.toColors.bg, eased),
      };
    }
  }
}
