import { useEffect, useRef } from "preact/hooks";
import type { TableView as Table } from "../protocol/TableView";
import type { Scene } from "../protocol/Scene";
import { sanitizeColor } from "../color";
import { drawScene } from "./scene";

const LOG_LINES = 30;

/** The shared table for remote players: scene, player panels, status and log. */
export function TableView({ table }: { table: Table }) {
  return (
    <section class="table" aria-label={table.title || "Table"}>
      <header class="table-head">
        <h2 class="t-title">{table.title}</h2>
        {table.status ? (
          <p class="table-status" role="status">
            {table.status}
          </p>
        ) : null}
      </header>
      {table.scene ? <SceneCanvas scene={table.scene} /> : null}
      {table.players.length ? (
        <ul class="players">
          {table.players.map((p, i) => (
            <li key={i} class={`player${p.active ? " active" : ""}`} aria-current={p.active ? "true" : undefined}>
              <span class="dot" style={{ background: sanitizeColor(p.color) }} aria-hidden="true" />
              <span class="player-name">{p.name}</span>
              {p.lines.map((line, j) => (
                <span key={j} class="player-line">
                  {line}
                </span>
              ))}
            </li>
          ))}
        </ul>
      ) : null}
      {table.log.length ? (
        <ol class="log" aria-label="Game log">
          {table.log.slice(-LOG_LINES).map((line, i) => (
            <li key={i}>{line}</li>
          ))}
        </ol>
      ) : null}
    </section>
  );
}

function SceneCanvas({ scene }: { scene: Scene }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const draw = () => {
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      const width = canvas.clientWidth || 320;
      const aspect = scene.width > 0 && scene.height > 0 ? scene.height / scene.width : 0.75;
      const maxH = Math.max(160, (globalThis.innerHeight || 800) * 0.7);
      const height = Math.min(width * aspect, maxH);
      const dpr = globalThis.devicePixelRatio || 1;
      canvas.style.height = `${Math.round(height)}px`;
      canvas.width = Math.max(1, Math.round(width * dpr));
      canvas.height = Math.max(1, Math.round(height * dpr));
      drawScene(ctx, scene, width, height, dpr);
    };
    draw();
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", draw);
      return () => window.removeEventListener("resize", draw);
    }
    const ro = new ResizeObserver(draw);
    ro.observe(canvas);
    return () => ro.disconnect();
  }, [scene]);
  return <canvas ref={ref} class="scene" role="img" aria-label="Table" />;
}
