import React, { useEffect, useRef } from "react";
import { FRAGMENT_SHADER_GLSL, VERTEX_SHADER_GLSL } from "./opart.glsl";
import { ShaderPhysics } from "./physics";
import { AdaptiveQualityController } from "./quality";
import { useAppStore } from "../stores/appStore";
import { useLimiterStore } from "../stores/limiterStore";
import { useSettingsStore } from "../stores/settingsStore";

function compileShader(gl: WebGL2RenderingContext, type: number, source: string): WebGLShader | null {
  const sh = gl.createShader(type);
  if (!sh) return null;
  gl.shaderSource(sh, source);
  gl.compileShader(sh);
  if (!gl.getShaderParameter(sh, gl.COMPILE_STATUS)) {
    console.error("[OpArtCanvas] Shader compile error:", gl.getShaderInfoLog(sh));
    gl.deleteShader(sh);
    return null;
  }
  return sh;
}

export const OpArtCanvas: React.FC = () => {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const physicsRef = useRef(new ShaderPhysics());
  const qualityRef = useRef(new AdaptiveQualityController());
  const renderOnceRef = useRef<(() => void) | null>(null);

  const theme = useSettingsStore((s) => s.settings.theme);
  const shaderIntensity = useSettingsStore((s) => s.settings.shader_intensity ?? 1.0);
  const stripeDensity = useSettingsStore((s) => s.settings.shader_stripe_density);
  const reduceMotion = useSettingsStore((s) => s.settings.reduce_motion);
  const gameModeActive = useLimiterStore((s) => s.gameMode.active);
  const rippleEvent = useAppStore((s) => s.rippleEvent);
  const stageRect = useAppStore((s) => s.stageRect);

  const frozen = reduceMotion || gameModeActive;

  useEffect(() => {
    if (frozen) {
      physicsRef.current.snapThemeImmediate(theme);
      renderOnceRef.current?.();
    } else {
      physicsRef.current.setTheme(theme);
    }
  }, [theme, frozen]);

  useEffect(() => {
    if (frozen) {
      renderOnceRef.current?.();
    }
  }, [stageRect, frozen]);

  useEffect(() => {
    if (rippleEvent && !frozen) {
      physicsRef.current.triggerRipple(
        rippleEvent.x,
        rippleEvent.y,
        window.innerWidth,
        window.innerHeight,
        rippleEvent.strength
      );
    }
  }, [rippleEvent, frozen]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const gl = canvas.getContext("webgl2", {
      alpha: false,
      antialias: false,
      depth: false,
      stencil: false,
      powerPreference: "low-power",
      preserveDrawingBuffer: true,
    });
    if (!gl) return;

    const vs = compileShader(gl, gl.VERTEX_SHADER, VERTEX_SHADER_GLSL);
    const fs = compileShader(gl, gl.FRAGMENT_SHADER, FRAGMENT_SHADER_GLSL);
    if (!vs || !fs) return;

    const prog = gl.createProgram();
    if (!prog) return;
    gl.attachShader(prog, vs);
    gl.attachShader(prog, fs);
    gl.linkProgram(prog);
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
      console.error("[OpArtCanvas] Link error:", gl.getProgramInfoLog(prog));
      return;
    }

    const vao = gl.createVertexArray();
    gl.bindVertexArray(vao);
    gl.useProgram(prog);

    const locs = {
      uRes: gl.getUniformLocation(prog, "uRes"),
      uTime: gl.getUniformLocation(prog, "uTime"),
      uPhase: gl.getUniformLocation(prog, "uPhase"),
      uMouse: gl.getUniformLocation(prog, "uMouse"),
      uMouseVel: gl.getUniformLocation(prog, "uMouseVel"),
      uForce: gl.getUniformLocation(prog, "uForce"),
      uRipple: gl.getUniformLocation(prog, "uRipple"),
      uAttract: gl.getUniformLocation(prog, "uAttract"),
      uStage: gl.getUniformLocation(prog, "uStage"),
      uHole: gl.getUniformLocation(prog, "uHole"),
      uOutside: gl.getUniformLocation(prog, "uOutside"),
      uDensity: gl.getUniformLocation(prog, "uDensity"),
      uDuty: gl.getUniformLocation(prog, "uDuty"),
      uColA: gl.getUniformLocation(prog, "uColA"),
      uColB: gl.getUniformLocation(prog, "uColB"),
      uBg: gl.getUniformLocation(prog, "uBg"),
    };

    let currentScale = qualityRef.current.getRenderScale(window.devicePixelRatio || 1);

    const syncCanvasSize = () => {
      const cssW = window.innerWidth;
      const cssH = window.innerHeight;
      currentScale = qualityRef.current.getRenderScale(window.devicePixelRatio || 1);
      const pxW = Math.max(1, Math.round(cssW * currentScale));
      const pxH = Math.max(1, Math.round(cssH * currentScale));
      if (canvas.width !== pxW || canvas.height !== pxH) {
        canvas.width = pxW;
        canvas.height = pxH;
      }
      gl.viewport(0, 0, canvas.width, canvas.height);
    };
    syncCanvasSize();

    const drawFrame = (dt: number, isStaticFreeze: boolean) => {
      const cssW = window.innerWidth;
      const cssH = window.innerHeight;
      const phys = physicsRef.current;

      const appSt = useAppStore.getState();
      const limSt = useLimiterStore.getState();
      const setSt = useSettingsStore.getState().settings;

      phys.setTheme(setSt.theme);
      if (isStaticFreeze) {
        phys.snapThemeImmediate(setSt.theme);
        phys.force = 0;
        phys.rippleStrength = 0;
      } else {
        phys.setDragAttract(
          appSt.dragOverActive,
          appSt.urlTargetCenter.x,
          appSt.urlTargetCenter.y,
          cssW,
          cssH
        );
        phys.step(dt, limSt.stats.speed_bps, cssH);
      }

      // Convert DOM stage rect (y grows downward) into GL pixel coordinates (y grows upward!)
      const stRect = appSt.stageRect;
      const glStageX = stRect.x * currentScale;
      const glStageY = (cssH - (stRect.y + stRect.height)) * currentScale;
      const glStageW = stRect.width * currentScale;
      const glStageH = stRect.height * currentScale;

      // Brand pill cutout rect: x=20, y=17, w=24, h=16 in DOM px
      const glHoleX = 20 * currentScale;
      const glHoleY = (cssH - 33) * currentScale;
      const glHoleW = 24 * currentScale;
      const glHoleH = 16 * currentScale;

      gl.useProgram(prog);
      gl.bindVertexArray(vao);

      gl.uniform2f(locs.uRes, canvas.width, canvas.height);
      gl.uniform1f(locs.uTime, isStaticFreeze ? 12.0 : phys.time);
      gl.uniform1f(locs.uPhase, isStaticFreeze ? 0.0 : phys.phase);
      gl.uniform2f(locs.uMouse, phys.mouseX, phys.mouseY);
      gl.uniform2f(locs.uMouseVel, phys.velX, phys.velY);
      gl.uniform1f(locs.uForce, isStaticFreeze ? 0.0 : phys.force);
      gl.uniform4f(
        locs.uRipple,
        phys.rippleX,
        phys.rippleY,
        phys.rippleAge,
        isStaticFreeze ? 0.0 : phys.rippleStrength
      );
      gl.uniform3f(locs.uAttract, phys.attractX, phys.attractY, isStaticFreeze ? 0.0 : phys.attractStrength);
      gl.uniform4f(locs.uStage, glStageX, glStageY, glStageW, glStageH);
      gl.uniform4f(locs.uHole, glHoleX, glHoleY, glHoleW, glHoleH);
      gl.uniform1f(locs.uOutside, setSt.shader_intensity ?? 1.0);
      gl.uniform1f(locs.uDensity, setSt.shader_stripe_density);
      gl.uniform1f(locs.uDuty, 0.6);

      const cols = phys.currentColors;
      gl.uniform3f(locs.uColA, cols.colA.r, cols.colA.g, cols.colA.b);
      gl.uniform3f(locs.uColB, cols.colB.r, cols.colB.g, cols.colB.b);
      gl.uniform3f(locs.uBg, cols.bg.r, cols.bg.g, cols.bg.b);

      gl.drawArrays(gl.TRIANGLES, 0, 3);
    };

    renderOnceRef.current = () => {
      syncCanvasSize();
      drawFrame(0.016, true);
    };

    const onMove = (e: PointerEvent) => {
      physicsRef.current.onPointerMove(e.clientX, e.clientY, window.innerWidth, window.innerHeight);
    };
    const onLeave = () => physicsRef.current.onPointerLeave();
    const onResize = () => {
      syncCanvasSize();
      const s = useSettingsStore.getState().settings;
      const g = useLimiterStore.getState().gameMode.active;
      if (s.reduce_motion || g || window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
        drawFrame(0.016, true);
      }
    };
    const onFocus = () => qualityRef.current.onFocus();
    const onBlur = () => qualityRef.current.onBlur();

    window.addEventListener("pointermove", onMove, { passive: true });
    window.addEventListener("pointerleave", onLeave, { passive: true });
    window.addEventListener("resize", onResize, { passive: true });
    window.addEventListener("focus", onFocus);
    window.addEventListener("blur", onBlur);

    let rafId = 0;
    let lastTs = performance.now();
    let lastRenderTs = 0;

    const mediaReduced = window.matchMedia("(prefers-reduced-motion: reduce)");

    const loop = (now: number) => {
      const s = useSettingsStore.getState().settings;
      const g = useLimiterStore.getState().gameMode.active;
      const shouldFreeze = s.reduce_motion || g || mediaReduced.matches;

      if (shouldFreeze) {
        // Render one clean static frame and stop loop (0% GPU!)
        drawFrame(0.016, true);
        return;
      }

      rafId = requestAnimationFrame(loop);

      if (document.hidden) {
        lastTs = now;
        return;
      }

      const minInterval = qualityRef.current.targetFrameIntervalMs();
      if (minInterval > 0 && now - lastRenderTs < minInterval) {
        return;
      }

      const dt = (now - lastTs) / 1000;
      lastTs = now;
      lastRenderTs = now;

      const t0 = performance.now();
      drawFrame(dt, false);
      const cpuDrawMs = performance.now() - t0;
      if (qualityRef.current.recordFrameTimeMs(cpuDrawMs + dt * 1000 * 0.15)) {
        syncCanvasSize();
      }
    };

    const s0 = useSettingsStore.getState().settings;
    const g0 = useLimiterStore.getState().gameMode.active;
    if (s0.reduce_motion || g0 || mediaReduced.matches) {
      drawFrame(0.016, true);
    } else {
      rafId = requestAnimationFrame(loop);
    }

    const onVisChange = () => {
      if (!document.hidden && rafId === 0) {
        const s = useSettingsStore.getState().settings;
        const g = useLimiterStore.getState().gameMode.active;
        if (!s.reduce_motion && !g && !mediaReduced.matches) {
          lastTs = performance.now();
          rafId = requestAnimationFrame(loop);
        }
      }
    };
    document.addEventListener("visibilitychange", onVisChange);

    return () => {
      if (rafId) cancelAnimationFrame(rafId);
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerleave", onLeave);
      window.removeEventListener("resize", onResize);
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("blur", onBlur);
      document.removeEventListener("visibilitychange", onVisChange);
      gl.deleteProgram(prog);
      gl.deleteShader(vs);
      gl.deleteShader(fs);
      gl.deleteVertexArray(vao);
    };
  }, [frozen, shaderIntensity, stripeDensity]);

  return (
    <canvas
      ref={canvasRef}
      aria-hidden="true"
      style={{
        position: "fixed",
        inset: 0,
        width: "100vw",
        height: "100vh",
        zIndex: 0,
        pointerEvents: "none",
        display: "block",
      }}
    />
  );
};
