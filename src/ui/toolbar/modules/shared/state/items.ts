import { signal } from "@preact/signals";

interface ToolbarState {
  left: string[];
  center: string[];
  right: string[];
}

export const $toolbar_state = signal<ToolbarState>({
  left: ["Item 1", "Item 2"],
  center: ["Center Item"],
  right: ["Item 3", "Item 4", "Item 5"],
});