// A browser-like DOM for component tests (happy-dom), plus helpers to mount
// Preact trees and fire events inside `act`.
import { Window } from "happy-dom";
import { render, type ComponentChild } from "preact";
import { act } from "preact/test-utils";

const win = new Window({ url: "http://localhost/", width: 400, height: 800 });
const g = globalThis as Record<string, unknown>;
for (const key of [
  "window", "document", "Node", "Element", "HTMLElement", "HTMLInputElement", "HTMLCanvasElement",
  "Event", "MouseEvent", "PointerEvent", "KeyboardEvent", "InputEvent", "SubmitEvent",
  "getComputedStyle", "history", "location", "SVGElement", "requestAnimationFrame", "cancelAnimationFrame",
]) {
  const value = (win as unknown as Record<string, unknown>)[key];
  g[key] = typeof value === "function" && /^[a-z]/.test(key) ? (value as () => unknown).bind(win) : value;
}

export { win };

export interface Mounted {
  root: HTMLElement;
  rerender(vnode: ComponentChild): void;
  unmount(): void;
}

export function mount(vnode: ComponentChild): Mounted {
  const root = document.createElement("div");
  document.body.appendChild(root);
  act(() => {
    render(vnode, root);
  });
  return {
    root,
    rerender(next) {
      act(() => {
        render(next, root);
      });
    },
    unmount() {
      act(() => {
        render(null, root);
      });
      root.remove();
    },
  };
}

export function fire(el: Element | null | undefined, event: Event): void {
  if (!el) throw new Error(`no element for ${event.type}`);
  act(() => {
    el.dispatchEvent(event);
  });
}

export function click(el: Element | null | undefined): void {
  fire(el, new MouseEvent("click", { bubbles: true, cancelable: true }));
}

export function pointer(el: Element | null | undefined, type: string, x: number, y: number): void {
  fire(el, new PointerEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y, pointerId: 1 }));
}

export function typeInto(el: Element | null | undefined, value: string): void {
  const input = el as HTMLInputElement;
  if (!input) throw new Error("no input");
  act(() => {
    input.value = value;
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

export function submit(form: Element | null | undefined): void {
  fire(form, new Event("submit", { bubbles: true, cancelable: true }));
}

export function key(el: Element | null | undefined, k: string): void {
  fire(el, new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true }));
}

/** Buttons whose visible text is exactly `text`. */
export function buttonByText(root: ParentNode, text: string): HTMLButtonElement | undefined {
  return Array.from(root.querySelectorAll("button")).find((b) => b.textContent?.trim() === text) as
    | HTMLButtonElement
    | undefined;
}
